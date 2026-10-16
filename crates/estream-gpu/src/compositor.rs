//! Canvas compositor: draws scene items as textured quads into a linear-light
//! `Rgba16Float` canvas with premultiplied alpha.

use crate::GpuContext;
use estream_core::BlendMode;
use glam::{Affine2, Vec2};
use std::sync::Arc;

/// Internal canvas format: linear light, half float, premultiplied alpha (HDR-ready).
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// One quad to draw. `binding` comes from [`Compositor::create_source_binding`].
#[derive(Clone, Copy, Debug)]
pub struct DrawItem<'a> {
    pub binding: &'a wgpu::BindGroup,
    pub matrix: Affine2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    pub opacity: f32,
    pub blend: BlendMode,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ItemUniform {
    m0: [f32; 2],
    m1: [f32; 2],
    t: [f32; 2],
    canvas: [f32; 2],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    opacity: f32,
    _pad0: f32,
    _pad1: [f32; 2],
}

/// Shared, immutable GPU state (pipelines, layouts, sampler). One per device.
#[derive(Debug)]
pub struct Compositor {
    item_layout: wgpu::BindGroupLayout,
    source_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipelines: [wgpu::RenderPipeline; 4],
    uniform_stride: u64,
}

fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor as F, BlendOperation::Add};
    let comp = |src_factor, dst_factor| BlendComponent {
        src_factor,
        dst_factor,
        operation: Add,
    };
    // All modes operate on premultiplied colors.
    let over_alpha = comp(F::One, F::OneMinusSrcAlpha);
    match mode {
        BlendMode::Normal => wgpu::BlendState {
            color: comp(F::One, F::OneMinusSrcAlpha),
            alpha: over_alpha,
        },
        BlendMode::Additive => wgpu::BlendState {
            color: comp(F::One, F::One),
            alpha: over_alpha,
        },
        // src*dst + dst*(1 - srcA)
        BlendMode::Multiply => wgpu::BlendState {
            color: comp(F::Dst, F::OneMinusSrcAlpha),
            alpha: over_alpha,
        },
        // src + dst - src*dst
        BlendMode::Screen => wgpu::BlendState {
            color: comp(F::OneMinusDst, F::One),
            alpha: over_alpha,
        },
    }
}

impl Compositor {
    pub fn new(gpu: &GpuContext) -> Arc<Self> {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/compositor.wgsl"));

        let item_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor item"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(size_of::<ItemUniform>() as u64),
                },
                count: None,
            }],
        });
        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor source"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor"),
            bind_group_layouts: &[Some(&item_layout), Some(&source_layout)],
            immediate_size: 0,
        });
        let pipelines = BlendMode::ALL.map(|mode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("compositor quad"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: CANVAS_FORMAT,
                        blend: Some(blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("compositor linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let uniform_stride = (size_of::<ItemUniform>() as u64).div_ceil(align) * align;

        Arc::new(Self {
            item_layout,
            source_layout,
            sampler,
            pipelines,
            uniform_stride,
        })
    }

    /// Creates the bind group a source keeps alongside its texture.
    pub fn create_source_binding(
        &self,
        gpu: &GpuContext,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor source"),
            layout: &self.source_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn pipeline(&self, mode: BlendMode) -> &wgpu::RenderPipeline {
        let index = BlendMode::ALL
            .iter()
            .position(|m| *m == mode)
            .unwrap_or_default();
        &self.pipelines[index]
    }
}

/// Per-canvas renderer: owns the uniform buffer so canvases can render in one submission.
#[derive(Debug)]
pub struct CanvasRenderer {
    shared: Arc<Compositor>,
    uniforms: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    capacity: usize,
    staging: Vec<u8>,
}

impl CanvasRenderer {
    pub fn new(gpu: &GpuContext, shared: Arc<Compositor>) -> Self {
        let capacity = 64;
        let (uniforms, uniform_bind_group) = Self::create_uniforms(gpu, &shared, capacity);
        Self {
            shared,
            uniforms,
            uniform_bind_group,
            capacity,
            staging: Vec::with_capacity(capacity * 256),
        }
    }

    fn create_uniforms(
        gpu: &GpuContext,
        shared: &Compositor,
        capacity: usize,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compositor items"),
            size: shared.uniform_stride * capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor items"),
            layout: &shared.item_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<ItemUniform>() as u64),
                }),
            }],
        });
        (buffer, bind_group)
    }

    /// Records the canvas render pass into `encoder`. `target` must use [`CANVAS_FORMAT`].
    pub fn render(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        canvas_size: Vec2,
        background: wgpu::Color,
        items: &[DrawItem<'_>],
    ) {
        if items.len() > self.capacity {
            // Grows rarely (only when a scene gets bigger than ever before).
            self.capacity = items.len().next_power_of_two();
            let (buffer, bind_group) = Self::create_uniforms(gpu, &self.shared, self.capacity);
            self.uniforms = buffer;
            self.uniform_bind_group = bind_group;
        }

        let stride = self.shared.uniform_stride as usize;
        self.staging.clear();
        self.staging.resize(items.len() * stride, 0);
        for (i, item) in items.iter().enumerate() {
            let uniform = ItemUniform {
                m0: item.matrix.matrix2.x_axis.to_array(),
                m1: item.matrix.matrix2.y_axis.to_array(),
                t: item.matrix.translation.to_array(),
                canvas: canvas_size.to_array(),
                uv_min: item.uv_min.to_array(),
                uv_max: item.uv_max.to_array(),
                opacity: item.opacity.clamp(0.0, 1.0),
                _pad0: 0.0,
                _pad1: [0.0; 2],
            };
            let offset = i * stride;
            self.staging[offset..offset + size_of::<ItemUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        if !self.staging.is_empty() {
            gpu.queue.write_buffer(&self.uniforms, 0, &self.staging);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(background),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut current: Option<BlendMode> = None;
        for (i, item) in items.iter().enumerate() {
            if current != Some(item.blend) {
                pass.set_pipeline(self.shared.pipeline(item.blend));
                current = Some(item.blend);
            }
            pass.set_bind_group(0, &self.uniform_bind_group, &[(i * stride) as u32]);
            pass.set_bind_group(1, item.binding, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}
