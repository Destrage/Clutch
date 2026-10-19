//! Linux capture: displays and windows through the ScreenCast portal over PipeWire, cameras
//! through V4L2.

pub mod camera;
mod capture;
mod frames;
mod mjpeg;
mod screencast;
pub(crate) mod tokens;

use estream_engine::SourceRegistry;

pub fn register(registry: &mut SourceRegistry) {
    for t in capture::source_types() {
        registry.register(t);
    }
    registry.register(camera::source_type());
}
