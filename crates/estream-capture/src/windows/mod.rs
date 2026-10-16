//! Windows capture backends.

pub mod camera;
pub mod capture;
pub mod d3d;
pub mod enumerate;
pub mod wgc;

use estream_engine::SourceRegistry;

pub fn register(registry: &mut SourceRegistry) {
    registry.register(camera::source_type());
    if wgc::WgcCapture::is_supported() {
        for t in capture::source_types() {
            registry.register(t);
        }
    } else {
        tracing::warn!("Windows.Graphics.Capture is not supported on this system");
    }
}
