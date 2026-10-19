//! Hands frames from a capture thread to the video thread without copying or holding a lock
//! across a frame: each side owns a buffer and they swap with the slot's.

use estream_gpu::{YuvMatrix, wgpu};
use std::sync::Mutex;

/// How a frame's bytes are laid out (always tightly packed: rows are `width` pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    /// 4-byte pixels.
    Packed(wgpu::TextureFormat),
    /// A Y plane, then interleaved CbCr at half height.
    Nv12 { matrix: YuvMatrix, full_range: bool },
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub data: Vec<u8>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            layout: Layout::Packed(wgpu::TextureFormat::Bgra8UnormSrgb),
            data: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct FrameSlot {
    inner: Mutex<(u64, Frame)>,
}

impl FrameSlot {
    /// Publishes `frame`; it comes back holding an older buffer to fill next.
    pub fn publish(&self, frame: &mut Frame) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::swap(&mut guard.1, frame);
        guard.0 += 1;
    }

    /// Swaps the newest frame into `into` if it is newer than `last`.
    pub fn take(&self, last: &mut u64, into: &mut Frame) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.0 == *last {
            return false;
        }
        *last = guard.0;
        std::mem::swap(&mut guard.1, into);
        true
    }
}

/// Copies `height` rows of `width` 4-byte pixels, `stride` bytes apart, into `out` (tightly
/// packed). `opaque` sets every alpha byte (the 4th) to 255, for formats whose 4th byte is
/// padding. `false` if `src` is too short.
pub fn copy_rows(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    opaque: bool,
    out: &mut Vec<u8>,
) -> bool {
    let row = width as usize * 4;
    if height == 0 || stride < row || src.len() < stride * (height as usize - 1) + row {
        return false;
    }
    out.resize(row * height as usize, 0);
    for (dst, src) in out.chunks_exact_mut(row).zip(src.chunks(stride)) {
        dst.copy_from_slice(&src[..row]);
        if opaque {
            for px in dst.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_packed_and_padding_made_opaque() {
        // 2×2 pixels, rows 12 bytes apart (4 bytes of row padding).
        let src: Vec<u8> = (0..24).collect();
        let mut out = Vec::new();
        assert!(copy_rows(&src, 2, 2, 12, true, &mut out));
        assert_eq!(
            out,
            [0, 1, 2, 255, 4, 5, 6, 255, 12, 13, 14, 255, 16, 17, 18, 255]
        );
        assert!(copy_rows(&src, 2, 2, 12, false, &mut out));
        assert_eq!(out[3], 3);
        assert!(
            !copy_rows(&src[..15], 2, 2, 12, false, &mut out),
            "too short"
        );
    }

    #[test]
    fn slot_swaps_buffers_and_skips_repeats() {
        let slot = FrameSlot::default();
        let mut last = 0;
        let mut mine = Frame::default();
        assert!(!slot.take(&mut last, &mut mine), "nothing yet");
        let mut produced = Frame {
            width: 1,
            height: 1,
            data: vec![9; 4],
            ..Default::default()
        };
        slot.publish(&mut produced);
        assert!(slot.take(&mut last, &mut mine));
        assert_eq!(mine.data, [9; 4]);
        assert!(
            !slot.take(&mut last, &mut mine),
            "same frame twice is not new"
        );
    }
}
