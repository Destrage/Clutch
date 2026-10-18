//! Helpers shared by the montage integration tests.
#![allow(clippy::unwrap_used, dead_code)]

use std::io::Write;
use std::path::Path;

/// Writes `stereo` (interleaved, 48 kHz) as 16-bit PCM WAV.
pub fn write_wav(path: &Path, stereo: &[f32]) {
    let data: Vec<u8> = stereo
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let rate = 48_000u32;
    let mut header = Vec::new();
    header.extend(b"RIFF");
    header.extend((36 + data.len() as u32).to_le_bytes());
    header.extend(b"WAVEfmt ");
    header.extend(16u32.to_le_bytes());
    header.extend(1u16.to_le_bytes()); // PCM
    header.extend(2u16.to_le_bytes()); // stereo
    header.extend(rate.to_le_bytes());
    header.extend((rate * 4).to_le_bytes());
    header.extend(4u16.to_le_bytes());
    header.extend(16u16.to_le_bytes());
    header.extend(b"data");
    header.extend((data.len() as u32).to_le_bytes());
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&header).unwrap();
    f.write_all(&data).unwrap();
}

/// A stereo sine at `freq` Hz and `amp` for `secs` seconds.
pub fn sine(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
    (0..(secs * 48_000.0) as usize)
        .flat_map(|i| {
            let v = amp * (std::f32::consts::TAU * freq * i as f32 / 48_000.0).sin();
            [v, v]
        })
        .collect()
}

/// RMS of interleaved stereo between `from` and `to` seconds.
pub fn rms(stereo: &[f32], from: f64, to: f64) -> f32 {
    let (a, b) = ((from * 48_000.0) as usize * 2, (to * 48_000.0) as usize * 2);
    let s = &stereo[a.min(stereo.len())..b.min(stereo.len())];
    (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt()
}

/// BT.601 limited-range Y, Cb, Cr for sRGB-coded `rgb`.
fn yuv(rgb: [f32; 3]) -> [u8; 3] {
    let [r, g, b] = rgb;
    let y = 16.0 + 65.481 * r + 128.553 * g + 24.966 * b;
    let cb = 128.0 - 37.797 * r - 74.203 * g + 112.0 * b;
    let cr = 128.0 + 112.0 * r - 93.786 * g - 18.214 * b;
    [y.round() as u8, cb.round() as u8, cr.round() as u8]
}

/// Writes a `secs` long, 30 fps, 320×180 Y4M clip whose color is `color(x / width)`.
pub fn write_y4m(path: &Path, secs: u32, color: impl Fn(f32) -> [f32; 3]) {
    let (w, h) = (320usize, 180usize);
    let mut y = vec![0u8; w * h];
    let mut u = vec![0u8; w * h / 4];
    let mut v = vec![0u8; w * h / 4];
    for row in 0..h {
        for col in 0..w {
            let [yy, cb, cr] = yuv(color(col as f32 / w as f32));
            y[row * w + col] = yy;
            if row % 2 == 0 && col % 2 == 0 {
                let i = (row / 2) * (w / 2) + col / 2;
                u[i] = cb;
                v[i] = cr;
            }
        }
    }
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(
        f,
        "YUV4MPEG2 W{w} H{h} F30:1 Ip A1:1 C420jpeg XCOLORRANGE=LIMITED"
    )
    .unwrap();
    for _ in 0..secs * 30 {
        f.write_all(b"FRAME\n").unwrap();
        f.write_all(&y).unwrap();
        f.write_all(&u).unwrap();
        f.write_all(&v).unwrap();
    }
}

/// What a file holds: (codec, seconds) per stream, video size, and the container's length.
#[derive(Debug)]
pub struct Probe {
    pub streams: Vec<(String, f64)>,
    pub size: Option<(u32, u32)>,
    pub duration: f64,
}

pub fn probe(path: &Path) -> Probe {
    use estream_media::ffmpeg;
    estream_media::init();
    let input = ffmpeg::format::input(path).unwrap();
    let mut size = None;
    let streams = input
        .streams()
        .map(|s| {
            let params = s.parameters();
            let tb = s.time_base();
            let secs = s.duration() as f64 * tb.0 as f64 / tb.1.max(1) as f64;
            let ctx = ffmpeg::codec::context::Context::from_parameters(params).unwrap();
            if let Ok(v) = ctx.decoder().video() {
                size = Some((v.width(), v.height()));
            }
            (format!("{:?}", s.parameters().id()), secs)
        })
        .collect();
    Probe {
        streams,
        size,
        duration: input.duration() as f64 / 1e6,
    }
}
