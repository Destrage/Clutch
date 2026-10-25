//! Encoding, muxing, recording and replay buffer for Clutch, built on FFmpeg 9
//! (LGPL shared build, linked dynamically).

pub use ffmpeg_next as ffmpeg;

pub mod audio_file;
pub mod clip_player;
pub mod encode;
pub mod frames;
#[cfg(windows)]
pub mod gpu_frames;
pub mod media_source;
pub mod multistream;
pub mod music_source;
pub mod mux;
pub mod output;
pub mod player;
pub mod settings;
pub mod sounds;
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
        if !headers_match_library() {
            tracing::error!(
                "FFmpeg was built against other headers than the library that loaded \
                 (e.g. the system's FFmpeg headers next to FFMPEG_DIR); codecs will be misread"
            );
        }
    });
}

/// Whether the codec ids compiled in match the loaded libavcodec. They differ when the bindings
/// came from another FFmpeg version's headers.
fn headers_match_library() -> bool {
    // SAFETY: takes a NUL-terminated name; returns a static descriptor or null.
    let descriptor = unsafe { ffmpeg::ffi::avcodec_descriptor_get_by_name(c"hevc".as_ptr()) };
    // SAFETY: non-null descriptors point to libavcodec's static table.
    descriptor.is_null() || unsafe { (*descriptor).id } == ffmpeg::ffi::AVCodecID::AV_CODEC_ID_HEVC
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
