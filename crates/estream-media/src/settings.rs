//! Recording settings (stored in app settings, like an OBS profile).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// Matroska: survives crashes (default).
    #[default]
    Mkv,
    /// Fragmented MP4: crash-safe and widely playable.
    FragmentedMp4,
    Mp4,
    Mov,
}

impl Container {
    pub const ALL: [Container; 4] = [
        Container::Mkv,
        Container::FragmentedMp4,
        Container::Mp4,
        Container::Mov,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::FragmentedMp4 | Container::Mp4 => "mp4",
            Container::Mov => "mov",
        }
    }

    pub fn format_name(self) -> &'static str {
        match self {
            Container::Mkv => "matroska",
            Container::FragmentedMp4 | Container::Mp4 => "mp4",
            Container::Mov => "mov",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Container::Mkv => "MKV (crash-safe)",
            Container::FragmentedMp4 => "Fragmented MP4 (crash-safe)",
            Container::Mp4 => "MP4",
            Container::Mov => "MOV",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC (H.265)",
            VideoCodec::Av1 => "AV1",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateControl {
    Cbr,
    Vbr,
    /// Constant quality (CQP/CQ/CRF depending on the encoder).
    #[default]
    ConstantQuality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Performance,
    #[default]
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    pub folder: PathBuf,
    /// `strftime`-style pattern for file names (without extension).
    pub filename: String,
    pub container: Container,
    pub codec: VideoCodec,
    /// `"auto"` or an FFmpeg encoder name such as `h264_nvenc`.
    pub encoder: String,
    pub rate_control: RateControl,
    pub bitrate_kbps: u32,
    /// Quality level for constant-quality mode (lower = better; ~18–28).
    pub quality: u32,
    pub keyframe_seconds: u32,
    pub preset: Preset,
    pub audio_codec: AudioCodec,
    pub audio_bitrate_kbps: u32,
    /// Bitmask of audio tracks to record (bit 0 = track 1).
    pub tracks: u8,
    pub replay_seconds: u32,
    /// Remux MKV recordings to MP4 after they stop.
    pub auto_remux: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        let folder = dirs_video().join("eStream");
        Self {
            folder,
            filename: "eStream %Y-%m-%d %H-%M-%S".into(),
            container: Container::Mkv,
            codec: VideoCodec::H264,
            encoder: "auto".into(),
            rate_control: RateControl::ConstantQuality,
            bitrate_kbps: 12_000,
            quality: 22,
            keyframe_seconds: 2,
            preset: Preset::Balanced,
            audio_codec: AudioCodec::Aac,
            audio_bitrate_kbps: 192,
            tracks: 0b1,
            replay_seconds: 30,
            auto_remux: false,
        }
    }
}

fn dirs_video() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join("Videos"))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl RecordingSettings {
    /// A new, unused file path for a recording started now.
    pub fn next_path(&self, suffix: &str, container: Container) -> PathBuf {
        let stem = chrono::Local::now().format(&self.filename).to_string();
        let stem = format!("{stem}{suffix}");
        let mut path = self
            .folder
            .join(format!("{stem}.{}", container.extension()));
        let mut n = 2;
        while path.exists() {
            path = self
                .folder
                .join(format!("{stem} ({n}).{}", container.extension()));
            n += 1;
        }
        path
    }
}
