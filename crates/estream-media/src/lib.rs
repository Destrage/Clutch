//! Encoding, muxing, recording and replay buffer for eStream, built on FFmpeg 9
//! (LGPL shared build, linked dynamically).

pub use ffmpeg_next as ffmpeg;

pub mod encode;
pub mod media_source;
pub mod multistream;
pub mod mux;
pub mod output;
pub mod player;
pub mod settings;
pub mod stream;
pub mod vcam;

pub use multistream::{Destination, LiveDestination, MultiStream};
pub use output::{OutputStats, Recording, ReplayBuffer};
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

/// libavutil version, e.g. `60.8.100`.
pub fn version() -> String {
    let v = ffmpeg::util::version();
    format!("{}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
}
