//! Capture and media sources for Clutch.
//!
//! Register them with [`register_all`]. Platform capture lives in
//! `platform::*` behind `cfg`; image and text sources are cross-platform.

pub mod devices;
pub mod media;
#[cfg(windows)]
pub mod windows;

use estream_engine::SourceRegistry;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("GPU interop is unavailable: {0}")]
    Interop(String),
    #[error("capture failed: {0}")]
    Capture(String),
    #[cfg(windows)]
    #[error(transparent)]
    Windows(#[from] ::windows::core::Error),
}

/// Registers every capture/media source type supported on this platform.
pub fn register_all(registry: &mut SourceRegistry) {
    media::register(registry);
    #[cfg(windows)]
    windows::register(registry);
    let _ = registry;
}
