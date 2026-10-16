//! Cross-platform media sources: images and text.

pub mod image;
pub mod text;
pub mod upload;

use estream_engine::SourceRegistry;

pub fn register(registry: &mut SourceRegistry) {
    registry.register(image::source_type());
    registry.register(text::source_type());
}
