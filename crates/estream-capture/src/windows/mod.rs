//! Windows capture backends.

pub mod app_audio;
pub mod camera;
pub mod capture;
pub mod capture_audio;
pub mod d3d;
pub mod enumerate;
pub mod wgc;

use estream_engine::SourceRegistry;

pub fn register(registry: &mut SourceRegistry) {
    registry.register(camera::source_type());
    registry.register(app_audio::source_type());
    if wgc::WgcCapture::is_supported() {
        for t in capture::source_types() {
            registry.register(t);
        }
        // Desktop audio leaves out what a capture already captures (needs process loopback).
        if let Some(plain) = registry.get("audio_output").cloned() {
            registry.register(capture_audio::desktop_type(plain));
        }
    } else {
        tracing::warn!("Windows.Graphics.Capture is not supported on this system");
    }
}
