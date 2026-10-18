//! The shared memory between Clutch (producer) and the camera source in Windows' Frame Server
//! (consumer). The consumer creates the mapping and asks for a frame size; Clutch writes NV12
//! frames of that size under a seqlock. Both sides leave heartbeats (`GetTickCount64`, which
//! is system-wide) so each knows whether the other is there.

use estream_shm::{Mapping, ShmError};
use std::sync::atomic::{Ordering, fence};

/// The mapping the Frame Server's camera source creates.
pub const MAPPING_NAME: &str = "Global\\estream-vcam-v1";
/// SYSTEM, LOCAL SERVICE, Administrators and interactive users may read and write.
pub const SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;BA)(A;;GA;;;IU)";

pub const MAX_WIDTH: u32 = 1920;
pub const MAX_HEIGHT: u32 = 1080;
const MAGIC: u32 = u32::from_le_bytes(*b"EVC1");
/// Shared memory layout version (bump when the header or frame format changes).
pub const VERSION: u32 = 1;
const HEADER: usize = 4096;

const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_WANT_W: usize = 8;
const OFF_WANT_H: usize = 12;
const OFF_CONSUMER_MS: usize = 16;
const OFF_SEQ: usize = 24;
const OFF_FRAME_W: usize = 32;
const OFF_FRAME_H: usize = 36;
const OFF_PRODUCER_MS: usize = 40;

/// Bytes of an NV12 frame.
pub fn nv12_len(width: u32, height: u32) -> usize {
    width as usize * height as usize * 3 / 2
}

pub const MAPPING_LEN: usize = HEADER + MAX_WIDTH as usize * MAX_HEIGHT as usize * 3 / 2;

/// Milliseconds since boot (the same clock in every process).
pub fn now_ms() -> u64 {
    #[cfg(windows)]
    {
        // SAFETY: no arguments, always succeeds.
        unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// One side's view of the shared memory.
pub struct Shared {
    map: Mapping,
}

impl Shared {
    /// Consumer side: creates the mapping (`Global\` names need `SeCreateGlobalPrivilege`).
    #[cfg(windows)]
    pub fn create(name: &str) -> Result<Self, ShmError> {
        let map = if name.starts_with("Global\\") {
            Mapping::create_with_sddl(name, MAPPING_LEN, SDDL)?
        } else {
            Mapping::create(name, MAPPING_LEN)?
        };
        map.write_u32(OFF_VERSION, VERSION);
        map.write_u32(OFF_MAGIC, MAGIC);
        Ok(Self { map })
    }

    /// Producer side: opens the mapping if a consumer created it.
    pub fn open(name: &str) -> Result<Self, ShmError> {
        let map = Mapping::open(name, MAPPING_LEN)?;
        if map.read_u32(OFF_MAGIC) != MAGIC || map.read_u32(OFF_VERSION) != VERSION {
            return Err(ShmError::Os("not an Clutch virtual camera mapping".into()));
        }
        Ok(Self { map })
    }

    // ------------------------------------------------------------ consumer

    pub fn set_wanted(&self, width: u32, height: u32) {
        self.map.write_u32(OFF_WANT_W, width);
        self.map.write_u32(OFF_WANT_H, height);
    }

    /// The consumer is delivering frames to an app (called per sample).
    pub fn touch_consumer(&self) {
        self.map
            .atomic_u64(OFF_CONSUMER_MS)
            .store(now_ms(), Ordering::Release);
    }

    pub fn producer_alive(&self, within_ms: u64) -> bool {
        let at = self.map.atomic_u64(OFF_PRODUCER_MS).load(Ordering::Acquire);
        at != 0 && now_ms().saturating_sub(at) <= within_ms
    }

    /// Copies a frame newer than `*last_seq` into `out`; returns its size. `None` if there's
    /// no new frame or it was being written (try again next time).
    pub fn read_frame(&self, out: &mut Vec<u8>, last_seq: &mut u64) -> Option<(u32, u32)> {
        let seq = self.map.atomic_u64(OFF_SEQ);
        let before = seq.load(Ordering::Acquire);
        if before == *last_seq || before % 2 == 1 {
            return None;
        }
        let (w, h) = (
            self.map.read_u32(OFF_FRAME_W),
            self.map.read_u32(OFF_FRAME_H),
        );
        if w == 0 || h == 0 || w > MAX_WIDTH || h > MAX_HEIGHT {
            return None;
        }
        let len = nv12_len(w, h);
        out.resize(len, 0);
        out.copy_from_slice(self.map.bytes(HEADER, len));
        fence(Ordering::Acquire);
        if seq.load(Ordering::Acquire) != before {
            return None;
        }
        *last_seq = before;
        Some((w, h))
    }

    // ------------------------------------------------------------ producer

    /// The size the consumer asked for (0×0 until it says).
    pub fn wanted(&self) -> (u32, u32) {
        (self.map.read_u32(OFF_WANT_W), self.map.read_u32(OFF_WANT_H))
    }

    pub fn consumer_alive(&self, within_ms: u64) -> bool {
        let at = self.map.atomic_u64(OFF_CONSUMER_MS).load(Ordering::Acquire);
        at != 0 && now_ms().saturating_sub(at) <= within_ms
    }

    /// Writes a `width`×`height` NV12 frame; `fill` gets exactly `nv12_len` bytes.
    pub fn write_frame(&self, width: u32, height: u32, fill: impl FnOnce(&mut [u8])) {
        let (width, height) = (width.min(MAX_WIDTH), height.min(MAX_HEIGHT));
        let seq = self.map.atomic_u64(OFF_SEQ);
        seq.fetch_add(1, Ordering::AcqRel); // odd: writing
        fence(Ordering::Release);
        self.map.write_u32(OFF_FRAME_W, width);
        self.map.write_u32(OFF_FRAME_H, height);
        fill(self.map.bytes_mut(HEADER, nv12_len(width, height)));
        seq.fetch_add(1, Ordering::Release); // even: done
        self.heartbeat();
    }

    /// The producer is alive even when frames don't change.
    pub fn heartbeat(&self) {
        self.map
            .atomic_u64(OFF_PRODUCER_MS)
            .store(now_ms(), Ordering::Release);
    }
}

// ---------------------------------------------------------------- frames

/// Fills an NV12 frame with one color (`y`, `u`, `v`).
pub fn fill_nv12(dst: &mut [u8], width: u32, height: u32, y: u8, u: u8, v: u8) {
    let luma = width as usize * height as usize;
    dst[..luma].fill(y);
    for uv in dst[luma..nv12_len(width, height)].as_chunks_mut::<2>().0 {
        uv[0] = u;
        uv[1] = v;
    }
}

/// Black in limited-range NV12.
pub fn fill_black(dst: &mut [u8], width: u32, height: u32) {
    fill_nv12(dst, width, height, 16, 128, 128);
}

/// Scales NV12 `src` into `dst` keeping the aspect ratio, centered with black bars
/// (bilinear; a plain copy when the sizes match).
pub fn fit_nv12(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, dh: u32) {
    if sw == dw && sh == dh {
        let len = nv12_len(dw, dh);
        dst[..len].copy_from_slice(&src[..len]);
        return;
    }
    fill_black(dst, dw, dh);
    if sw < 2 || sh < 2 || dw < 2 || dh < 2 {
        return;
    }
    let scale = (dw as f64 / sw as f64).min(dh as f64 / sh as f64);
    let even = |v: f64, max: u32| ((v.round() as u32) & !1).clamp(2, max);
    let cw = even(sw as f64 * scale, dw);
    let ch = even(sh as f64 * scale, dh);
    let ox = ((dw - cw) / 2) & !1;
    let oy = ((dh - ch) / 2) & !1;
    let (src_y, src_uv) = src.split_at(sw as usize * sh as usize);
    let (dst_y, dst_uv) = dst.split_at_mut(dw as usize * dh as usize);
    let plane = |src, dst, w, h, channels, (x, y, cw, ch)| Plane {
        src,
        sw: w,
        sh: h,
        stride: sw as usize,
        channels,
        dst,
        dstride: dw as usize,
        rect: (x, y, cw, ch),
    };
    plane(src_y, dst_y, sw, sh, 1, (ox, oy, cw, ch)).scale();
    plane(
        src_uv,
        dst_uv,
        sw / 2,
        sh / 2,
        2,
        (ox / 2, oy / 2, cw / 2, ch / 2),
    )
    .scale();
}

/// One plane to scale into a rectangle of the destination plane.
struct Plane<'a> {
    src: &'a [u8],
    sw: u32,
    sh: u32,
    stride: usize,
    channels: usize,
    dst: &'a mut [u8],
    dstride: usize,
    /// x, y, width, height in destination samples.
    rect: (u32, u32, u32, u32),
}

impl Plane<'_> {
    fn scale(self) {
        let (ox, oy, cw, ch) = self.rect;
        if cw == 0 || ch == 0 || self.sw == 0 || self.sh == 0 {
            return;
        }
        // 16.16 fixed point source coordinates of each destination sample center.
        let step_x = ((self.sw as u64) << 16) / cw as u64;
        let step_y = ((self.sh as u64) << 16) / ch as u64;
        let max_x = self.sw as i64 - 1;
        let max_y = self.sh as i64 - 1;
        for dy in 0..ch as usize {
            let fy = ((dy as i64 * step_y as i64) + (step_y as i64 >> 1) - (1 << 15)).max(0);
            let y0 = (fy >> 16).min(max_y) as usize;
            let y1 = (y0 as i64 + 1).min(max_y) as usize;
            let wy = (fy & 0xffff) as u32;
            let row0 = &self.src[y0 * self.stride..];
            let row1 = &self.src[y1 * self.stride..];
            let out = &mut self.dst[(oy as usize + dy) * self.dstride..];
            for dx in 0..cw as usize {
                let fx = ((dx as i64 * step_x as i64) + (step_x as i64 >> 1) - (1 << 15)).max(0);
                let x0 = (fx >> 16).min(max_x) as usize;
                let x1 = (x0 as i64 + 1).min(max_x) as usize;
                let wx = (fx & 0xffff) as u32;
                for c in 0..self.channels {
                    let p = |row: &[u8], x: usize| row[x * self.channels + c] as u32;
                    let top = p(row0, x0) * (0x10000 - wx) + p(row0, x1) * wx;
                    let bottom = p(row1, x0) * (0x10000 - wx) + p(row1, x1) * wx;
                    let v = ((top as u64 >> 8) * (0x10000 - wy) as u64
                        + (bottom as u64 >> 8) * wy as u64)
                        >> 24;
                    out[(ox as usize + dx) * self.channels + c] = v.min(255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut f = vec![0; nv12_len(w, h)];
        fill_nv12(&mut f, w, h, y, u, v);
        f
    }

    #[test]
    fn same_size_is_a_copy() {
        let src = solid(64, 32, 200, 10, 240);
        let mut dst = vec![0; src.len()];
        fit_nv12(&src, 64, 32, &mut dst, 64, 32);
        assert_eq!(src, dst);
    }

    #[test]
    fn downscale_keeps_color_and_aspect() {
        let src = solid(1920, 1080, 81, 90, 240); // red-ish
        let mut dst = vec![0; nv12_len(640, 360)];
        fit_nv12(&src, 1920, 1080, &mut dst, 640, 360);
        assert!(dst[..640 * 360].iter().all(|&y| y == 81));
        let uv = &dst[640 * 360..];
        assert!(uv.as_chunks::<2>().0.iter().all(|c| *c == [90, 240]));
    }

    #[test]
    fn vertical_canvas_is_pillarboxed() {
        let src = solid(1080, 1920, 200, 128, 128);
        let mut dst = vec![0; nv12_len(1280, 720)];
        fit_nv12(&src, 1080, 1920, &mut dst, 1280, 720);
        let row = &dst[360 * 1280..361 * 1280];
        assert_eq!(row[0], 16, "left bar is black");
        assert_eq!(row[1279], 16, "right bar is black");
        assert_eq!(row[640], 200, "content in the middle");
        // 1080×1920 into 720 tall → 405 wide (even: 404), centered.
        let lit = row.iter().filter(|&&y| y == 200).count();
        assert!((400..=406).contains(&lit), "{lit}");
    }

    #[test]
    fn gradient_scales_smoothly() {
        let (w, h) = (256, 2);
        let mut src = solid(w, h, 0, 128, 128);
        for y in 0..h as usize {
            for x in 0..w as usize {
                src[y * w as usize + x] = x as u8;
            }
        }
        let mut dst = vec![0; nv12_len(128, 2)];
        fit_nv12(&src, w, h, &mut dst, 128, 2);
        let row = &dst[..128];
        assert!(row.windows(2).all(|p| p[1] >= p[0]), "monotonic");
        assert!(row[0] <= 2 && row[127] >= 252, "{} {}", row[0], row[127]);
    }

    #[cfg(windows)]
    #[test]
    fn seqlock_round_trip() {
        let name = format!("Local\\estream-vcam-test-{}", std::process::id());
        let consumer = Shared::create(&name).unwrap();
        let producer = Shared::open(&name).unwrap();
        assert!(!producer.consumer_alive(1000));
        consumer.set_wanted(640, 360);
        consumer.touch_consumer();
        assert!(producer.consumer_alive(1000));
        assert_eq!(producer.wanted(), (640, 360));

        let mut out = Vec::new();
        let mut seq = 0;
        assert_eq!(consumer.read_frame(&mut out, &mut seq), None);
        producer.write_frame(640, 360, |f| fill_nv12(f, 640, 360, 50, 60, 70));
        assert!(consumer.producer_alive(1000));
        assert_eq!(consumer.read_frame(&mut out, &mut seq), Some((640, 360)));
        assert_eq!(out[0], 50);
        assert_eq!(&out[640 * 360..640 * 360 + 2], &[60, 70]);
        assert_eq!(
            consumer.read_frame(&mut out, &mut seq),
            None,
            "no new frame"
        );
    }
}
