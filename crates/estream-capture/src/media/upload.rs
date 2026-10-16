//! A GPU texture filled from CPU pixels (images, rendered text).

use estream_core::Size;
use estream_engine::{SourceContext, SourceFrame};
use estream_gpu::wgpu;

/// Straight-alpha sRGB RGBA8 pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; (width * height * 4) as usize],
        }
    }
}

#[derive(Default)]
pub struct CpuTexture {
    texture: Option<wgpu::Texture>,
    binding: Option<wgpu::BindGroup>,
    size: Size,
}

impl CpuTexture {
    /// Uploads pixels, recreating the texture only when the size changes.
    pub fn upload(&mut self, ctx: &SourceContext<'_>, pixels: &Pixels) {
        if pixels.width == 0 || pixels.height == 0 {
            self.clear();
            return;
        }
        let same_size = self
            .texture
            .as_ref()
            .is_some_and(|t| t.width() == pixels.width && t.height() == pixels.height);
        if !same_size {
            let texture = ctx.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cpu source"),
                size: wgpu::Extent3d {
                    width: pixels.width,
                    height: pixels.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.binding = Some(
                ctx.compositor
                    .create_source_binding(ctx.gpu, &texture.create_view(&Default::default())),
            );
            self.texture = Some(texture);
        }
        if let Some(texture) = &self.texture {
            ctx.gpu.queue.write_texture(
                texture.as_image_copy(),
                &pixels.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pixels.width * 4),
                    rows_per_image: Some(pixels.height),
                },
                texture.size(),
            );
        }
        self.size = Size::new(pixels.width as f32, pixels.height as f32);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn frame(&self) -> Option<SourceFrame<'_>> {
        Some(SourceFrame {
            binding: self.binding.as_ref()?,
            size: self.size,
        })
    }
}
