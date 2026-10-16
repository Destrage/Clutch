use crate::GpuContext;
use std::collections::HashMap;

/// Reuses textures by (size, format) so steady-state rendering never creates GPU resources.
#[derive(Debug, Default)]
pub struct TexturePool {
    free: HashMap<(u32, u32, wgpu::TextureFormat), Vec<wgpu::Texture>>,
}

impl TexturePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn acquire(
        &mut self,
        gpu: &GpuContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        let key = (width.max(1), height.max(1), format);
        self.free
            .get_mut(&key)
            .and_then(Vec::pop)
            .unwrap_or_else(|| gpu.create_render_texture("pooled", key.0, key.1, format))
    }

    pub fn release(&mut self, texture: wgpu::Texture) {
        let key = (texture.width(), texture.height(), texture.format());
        self.free.entry(key).or_default().push(texture);
    }

    pub fn len(&self) -> usize {
        self.free.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
