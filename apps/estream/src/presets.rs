//! Canvas presets for the first-run wizard and hardware encoder hints.

use estream_core::Rational;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPreset {
    pub label: &'static str,
    pub detail: &'static str,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
}

pub const PRESETS: &[CanvasPreset] = &[
    CanvasPreset {
        label: "1080p · 60 fps",
        detail: "1920 × 1080 — gaming, most streams",
        width: 1920,
        height: 1080,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "1080p · 30 fps",
        detail: "1920 × 1080 — talk shows, lower bandwidth",
        width: 1920,
        height: 1080,
        fps: Rational::fps(30),
    },
    CanvasPreset {
        label: "1440p · 60 fps",
        detail: "2560 × 1440 — high quality recording",
        width: 2560,
        height: 1440,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "720p · 60 fps",
        detail: "1280 × 720 — slower connections",
        width: 1280,
        height: 720,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "Vertical 1080 × 1920 · 30 fps",
        detail: "Shorts, Reels, TikTok",
        width: 1080,
        height: 1920,
        fps: Rational::fps(30),
    },
    CanvasPreset {
        label: "4K · 30 fps",
        detail: "3840 × 2160 — recording",
        width: 3840,
        height: 2160,
        fps: Rational::fps(30),
    },
];

/// Hardware encoders that are normally available for a GPU vendor (PCI vendor id).
/// Real probing happens when encoding support lands (add-recording).
pub fn likely_encoders(vendor: u32) -> &'static str {
    match vendor {
        0x10de => "NVIDIA NVENC (H.264, HEVC, AV1 on RTX 40+)",
        0x1002 | 0x1022 => "AMD AMF (H.264, HEVC, AV1 on RX 7000+)",
        0x8086 => "Intel Quick Sync (H.264, HEVC, AV1 on Arc)",
        0x106b => "Apple VideoToolbox (H.264, HEVC)",
        _ => "Software encoding (x264 / SVT-AV1)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_canvases() {
        for p in PRESETS {
            assert!(p.width % 2 == 0 && p.height % 2 == 0, "{}", p.label);
            assert!(p.fps.is_valid());
        }
        assert!(likely_encoders(0x10de).contains("NVENC"));
    }
}
