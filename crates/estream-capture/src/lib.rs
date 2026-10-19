//! Capture and media sources for Clutch.
//!
//! Register them with [`register_all`]. Platform capture lives in
//! `platform::*` behind `cfg`; image and text sources are cross-platform.

pub mod camera;
pub mod devices;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod media;
#[cfg(windows)]
pub mod windows;

use estream_engine::{SourceRegistry, SourceType};

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

/// Where per-machine capture state is kept (Clutch's settings folder): on Linux, the
/// screen-share grants that skip the system's picker.
pub fn set_state_dir(dir: std::path::PathBuf) {
    #[cfg(target_os = "linux")]
    linux::tokens::set_dir(dir);
    #[cfg(not(target_os = "linux"))]
    let _ = dir;
}

/// Capture types some system has: `(kind, name, description, has video)`. Projects carry
/// them between systems, so where one isn't supported it still loads, as unavailable.
const PLATFORM_TYPES: &[(&str, &str, &str, bool)] = &[
    (
        "display_capture",
        "Display capture",
        "Capture an entire monitor",
        true,
    ),
    (
        "window_capture",
        "Window capture",
        "Capture a single application window",
        true,
    ),
    (
        "game_capture",
        "Game capture",
        "Capture fullscreen and borderless games",
        true,
    ),
    (
        "camera",
        "Video capture device",
        "Webcams and capture cards",
        true,
    ),
    (
        "app_audio",
        "Application audio",
        "Capture one app's audio (games, browsers, music players)",
        false,
    ),
];

/// Registers every capture/media source type supported on this platform, and unavailable
/// placeholders for the [`PLATFORM_TYPES`] it doesn't support.
pub fn register_all(registry: &mut SourceRegistry) {
    media::register(registry);
    #[cfg(windows)]
    windows::register(registry);
    #[cfg(target_os = "linux")]
    linux::register(registry);
    for &(kind, name, description, video) in PLATFORM_TYPES {
        if registry.get(kind).is_none() {
            registry.register(SourceType::unavailable(kind, name, description, video));
        }
    }
}
