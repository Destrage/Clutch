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
