//! Linux capture: displays and windows through the ScreenCast portal over PipeWire.

mod capture;
mod frames;
mod screencast;
pub(crate) mod tokens;

use estream_engine::SourceRegistry;

pub fn register(registry: &mut SourceRegistry) {
    for t in capture::source_types() {
        registry.register(t);
    }
}
