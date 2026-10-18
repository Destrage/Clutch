//! Exporting a montage to an MP4: every frame rendered with the same [`Renderer`] as the
//! preview, read back as NV12 and encoded with the studio's encoders, with the audio mix
//! encoded alongside. Written to a `.part` file and renamed at the end, so a cancelled or
//! failed export leaves nothing behind.

use crate::audio::AudioRenderer;
use crate::model::{Format, Montage};
use crate::render::{RenderError, Renderer};
use estream_audio::{BLOCK, Block};
use estream_core::{Rational, Timestamp};
use estream_engine::VideoFrame;
use estream_gpu::GpuContext;
use estream_media::encode::{self, AudioEncoder, VideoEncoder};
use estream_media::mux::{Muxer, StreamSpec};
use estream_media::{AudioCodec, Container, Preset, RateControl, RecordingSettings, VideoCodec};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportPreset {
    /// 1920×1080 at 60 fps.
    #[default]
    YouTube,
    /// 1080×1920 at 60 fps (Shorts, TikTok, Reels).
    Vertical,
    /// The montage's own format.
    Source,
    Custom(Format),
}

impl ExportPreset {
    pub fn format(self, montage: &Montage) -> Format {
        match self {
            ExportPreset::YouTube => Format::LANDSCAPE,
            ExportPreset::Vertical => Format::VERTICAL,
            ExportPreset::Source => montage.format,
            ExportPreset::Custom(f) => f,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::YouTube => "YouTube (1080p60)",
            ExportPreset::Vertical => "Shorts / TikTok / Reels (1080×1920)",
            ExportPreset::Source => "Same as the montage",
            ExportPreset::Custom(_) => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub path: PathBuf,
    pub preset: ExportPreset,
    pub codec: VideoCodec,
    /// Video bitrate (kbps).
    pub bitrate_kbps: u32,
}

impl ExportOptions {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            preset: ExportPreset::default(),
            codec: VideoCodec::H264,
            bitrate_kbps: 16_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Render(#[from] RenderError),
    #[error("{0}")]
    Media(#[from] estream_media::MediaError),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("there's nothing to export")]
    Empty,
    #[error("export cancelled")]
    Cancelled,
}

fn part_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("montage");
    path.with_file_name(format!("{stem}.part.mp4"))
}

/// Exports `montage`. `progress` receives 0..=1 and returns false to cancel.
pub fn export(
    gpu: &GpuContext,
    montage: &Montage,
    options: &ExportOptions,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<PathBuf, ExportError> {
    let duration = montage.duration();
    if duration <= 0.0 || montage.segments.is_empty() {
        return Err(ExportError::Empty);
    }
    let part = part_path(&options.path);
    let result = write(gpu, montage, options, &part, duration, progress);
    match result {
        Ok(()) => {
            std::fs::rename(&part, &options.path)?;
            Ok(options.path.clone())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn write(
    gpu: &GpuContext,
    montage: &Montage,
    options: &ExportOptions,
    part: &Path,
    duration: f64,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<(), ExportError> {
    let format = options.preset.format(montage);
    let mut renderer = Renderer::new(gpu, format.width, format.height);
    let (width, height) = renderer.size();
    let fps = Rational::fps(format.fps.max(1));
    let settings = RecordingSettings {
        codec: options.codec,
        encoder: "auto".into(),
        rate_control: RateControl::Vbr,
        bitrate_kbps: options.bitrate_kbps.clamp(1_000, 150_000),
        preset: Preset::Quality,
        keyframe_seconds: 2,
        ..RecordingSettings::default()
    };
    let chosen = encode::choose(&settings)?;
    let mut video = VideoEncoder::open(chosen.name, width, height, fps, &settings)?;
    let mut audio = AudioEncoder::open(AudioCodec::Aac, 192)?;
    let specs = [
        StreamSpec {
            parameters: video.parameters(),
            time_base: video.time_base,
            is_video: true,
        },
        StreamSpec {
            parameters: audio.parameters(),
            time_base: audio.time_base,
            is_video: false,
        },
    ];
    let mut muxer = Muxer::create(part, Container::Mp4, &specs)?;
    let mut write_error = None;

    // The audio is mixed up front (montages are short) and fed in step with the video.
    let mix = AudioRenderer::new().render(montage);
    let mut audio_frames = 0usize;
    let mut block = Block::SILENT;
    let mut feed_audio = |until_frames: usize,
                          audio: &mut AudioEncoder,
                          muxer: &mut Muxer,
                          write_error: &mut Option<estream_media::MediaError>|
     -> Result<(), ExportError> {
        while audio_frames < until_frames && audio_frames * 2 < mix.len() {
            for i in 0..BLOCK {
                let f = (audio_frames + i) * 2;
                block.l[i] = mix.get(f).copied().unwrap_or(0.0);
                block.r[i] = mix.get(f + 1).copied().unwrap_or(0.0);
            }
            audio_frames += BLOCK;
            audio.push(&block, &mut |p| {
                if let Err(e) = muxer.write(1, p) {
                    write_error.get_or_insert(e);
                }
            })?;
        }
        Ok(())
    };

    let frames = (duration * format.fps as f64).round().max(1.0) as i64;
    for n in 0..frames {
        let t = n as f64 / format.fps as f64;
        renderer.render(montage, t)?;
        let nv12 = renderer.read_nv12()?;
        let frame = VideoFrame::from_nv12(Timestamp::ZERO, width, height, nv12);
        video.encode(&frame, n, false, &mut |p| {
            if let Err(e) = muxer.write(0, p) {
                write_error.get_or_insert(e);
            }
        })?;
        let audio_until = ((t + 1.0 / format.fps as f64) * 48_000.0) as usize;
        feed_audio(audio_until, &mut audio, &mut muxer, &mut write_error)?;
        if let Some(e) = write_error.take() {
            return Err(e.into());
        }
        if !progress((n + 1) as f32 / frames as f32) {
            return Err(ExportError::Cancelled);
        }
    }
    feed_audio(mix.len() / 2, &mut audio, &mut muxer, &mut write_error)?;
    video.finish(&mut |p| {
        if let Err(e) = muxer.write(0, p) {
            write_error.get_or_insert(e);
        }
    })?;
    audio.finish(&mut |p| {
        if let Err(e) = muxer.write(1, p) {
            write_error.get_or_insert(e);
        }
    })?;
    if let Some(e) = write_error {
        return Err(e.into());
    }
    muxer.finish()?;
    Ok(())
}
