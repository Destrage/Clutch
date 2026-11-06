//! Renders a [`Paint`] once into a texture: solid colors as 1 × 1, gradients at full size with a
//! baked dither. Nothing here runs per frame.

use crate::context::GpuContext;
use estream_core::Paint;
use estream_core::paint::{MAX_STOPS, to_premul};

const SHADER: &str = include_str!("shaders/gradient.wgsl");

/// Longest side a gradient texture is drawn at; larger targets sample it bilinearly.
pub const MAX_GRADIENT_SIDE: u32 = 4096;

/// Storage format of a paint texture. Sample it through [`PaintTexture::view`], which reads it as
/// sRGB.
pub const PAINT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const PAINT_VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    kind: u32,
    count: u32,
    _pad: [u32; 2],
    geom: [f32; 4],
    size: [f32; 4],
    colors: [[f32; 4]; MAX_STOPS],
    at: [[f32; 4]; 2],
}

/// A rendered paint. `view` reads it as sRGB with straight alpha, like other source textures.
#[derive(Debug)]
pub struct PaintTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

#[derive(Debug)]
pub struct GradientRenderer {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

/// The size a paint is drawn at for a `width × height` target.
pub fn paint_size(paint: &Paint, width: u32, height: u32) -> (u32, u32) {
    if !paint.is_gradient() {
        return (1, 1);
    }
    let (w, h) = (width.max(1), height.max(1));
    let long = w.max(h);
    if long <= MAX_GRADIENT_SIDE {
        return (w, h);
    }
    let scale = MAX_GRADIENT_SIDE as f64 / long as f64;
    let fit = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (fit(w), fit(h))
}

impl GradientRenderer {
    pub fn new(gpu: &GpuContext) -> Self {
        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gradient"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<Uniform>() as u64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gradient"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gradient"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PAINT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    /// Draws `paint` for a `width × height` target (see [`paint_size`]) and submits the work.
    pub fn render(&self, gpu: &GpuContext, paint: &Paint, width: u32, height: u32) -> PaintTexture {
        let paint = paint.normalized();
        let (w, h) = paint_size(&paint, width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("paint"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PAINT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[PAINT_VIEW_FORMAT],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(PAINT_VIEW_FORMAT),
            ..Default::default()
        });
        match uniform(&paint, w, h) {
            None => {
                // Solid: one texel, written directly (sRGB bytes into the unorm storage).
                gpu.queue.write_texture(
                    texture.as_image_copy(),
                    &paint.first_color().to_array(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    texture.size(),
                );
            }
            Some(u) => self.draw(gpu, &texture, &u),
        }
        PaintTexture { texture, view }
    }

    fn draw(&self, gpu: &GpuContext, texture: &wgpu::Texture, u: &Uniform) {
        use wgpu::util::DeviceExt;
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gradient"),
                contents: bytemuck::bytes_of(u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gradient"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let target = texture.create_view(&Default::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gradient"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gradient"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
    }
}

/// Shader parameters for a normalized gradient drawn at `w × h`, or `None` for a solid paint.
fn uniform(paint: &Paint, w: u32, h: u32) -> Option<Uniform> {
    let (w, h) = (w as f32, h as f32);
    let (kind, geom) = match paint {
        Paint::Solid { .. } => return None,
        Paint::Linear { angle, .. } => {
            let a = angle.to_radians();
            let dir = [a.sin(), -a.cos()];
            let length = (w * dir[0]).abs() + (h * dir[1]).abs();
            (0, [dir[0], dir[1], (length * 0.5).max(0.5), 0.0])
        }
        Paint::Radial { center, radius, .. } => {
            let c = [center[0] * w, center[1] * h];
            let far = [0.0, w]
                .iter()
                .flat_map(|x| [0.0, h].map(|y| (x - c[0]).hypot(y - c[1])))
                .fold(0.0f32, f32::max);
            (1, [c[0], c[1], (far * radius).max(0.5), 0.0])
        }
    };
    let stops = paint.stops();
    let mut colors = [[0.0; 4]; MAX_STOPS];
    let mut at = [[0.0; 4]; 2];
    for (i, s) in stops.iter().take(MAX_STOPS).enumerate() {
        colors[i] = to_premul(s.color);
        at[i / 4][i % 4] = s.at;
    }
    Some(Uniform {
        kind,
        count: stops.len().clamp(1, MAX_STOPS) as u32,
        _pad: [0; 2],
        geom,
        size: [w, h, 0.0, 0.0],
        colors,
        at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_core::{GradientStop, Rgba8};

    #[test]
    fn big_targets_are_capped_and_keep_their_shape() {
        let p = Paint::linear(Rgba8::BLACK, Rgba8::WHITE);
        assert_eq!(paint_size(&p, 1080, 1920), (1080, 1920));
        assert_eq!(paint_size(&p, 4320, 7680), (2304, 4096));
        assert_eq!(paint_size(&Paint::solid(Rgba8::WHITE), 1920, 1080), (1, 1));
    }

    #[test]
    fn linear_length_spans_the_box_at_any_angle() {
        let mut p = Paint::linear(Rgba8::BLACK, Rgba8::WHITE);
        let u = uniform(&p, 200, 100).unwrap();
        assert!((u.geom[2] - 100.0).abs() < 1e-3, "90°: half the width");
        if let Paint::Linear { angle, .. } = &mut p {
            *angle = 180.0;
        }
        let u = uniform(&p, 200, 100).unwrap();
        assert!((u.geom[2] - 50.0).abs() < 1e-3, "180°: half the height");
        assert!((u.geom[1] - 1.0).abs() < 1e-6, "180° runs downwards");
    }

    #[test]
    fn radial_radius_reaches_the_farthest_corner() {
        let p = Paint::Radial {
            center: [0.0, 0.0],
            radius: 1.0,
            stops: vec![
                GradientStop::new(Rgba8::BLACK, 0.0),
                GradientStop::new(Rgba8::WHITE, 1.0),
            ],
        };
        let u = uniform(&p, 300, 400).unwrap();
        assert!((u.geom[2] - 500.0).abs() < 1e-3);
    }
}
