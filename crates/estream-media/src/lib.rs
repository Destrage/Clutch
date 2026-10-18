//! Encoding, muxing, recording and replay buffer for eStream, built on FFmpeg 9
//! (LGPL shared build, linked dynamically).

pub use ffmpeg_next as ffmpeg;

pub mod audio_file;
pub mod encode;
pub mod media_source;
pub mod multistream;
pub mod mux;
pub mod output;
pub mod player;
pub mod settings;
pub mod stream;
pub mod thumbnail;
pub mod trim;
pub mod vcam;

pub use multistream::{Destination, LiveDestination, MultiStream};
pub use output::{MAX_REPLAY_SECONDS, OutputStats, Recording, ReplayBuffer, SavedClip};
pub use settings::{
    AudioCodec, Container, Preset, Protocol, RateControl, RecordingSettings, Service,
    StreamSettings, VideoCodec, redact,
};
pub use stream::{DestinationStats, StreamState};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("FFmpeg: {0}")]
    Ffmpeg(#[from] ffmpeg::Error),
    #[error("{0}")]
    NoEncoder(String),
    #[error("file error: {0}")]
    Io(String),
    #[error("{0}")]
    Engine(String),
}

/// Initializes FFmpeg once (quiet logging).
pub fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Err(e) = ffmpeg::init() {
            tracing::error!(error = %e, "FFmpeg failed to initialize");
        }
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
        tracing::info!(version = %version(), "FFmpeg loaded");
    });
}

/// A media file's duration in seconds.
pub fn probe_duration(path: &std::path::Path) -> Result<f64, MediaError> {
    init();
    let input = ffmpeg::format::input(path)?;
    match input.duration() {
        d if d > 0 => Ok(d as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)),
        _ => Err(MediaError::Io("the file's length is unknown".into())),
    }
}

/// libavutil version, e.g. `60.8.100`.
pub fn version() -> String {
    let v = ffmpeg::util::version();
    format!("{}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
}
