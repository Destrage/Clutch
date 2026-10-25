//! Output format conversion from the linear canvas (preview sRGB, encoder NV12).

use crate::{CANVAS_FORMAT, GpuContext};
use std::sync::Arc;

/// Storage format of preview textures. Stored as plain `Rgba8Unorm` holding sRGB-encoded
/// values, so UI toolkits that sample without sRGB decoding (e.g. Slint) show correct colors.
pub const PREVIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// View format used to render into preview textures (hardware performs the sRGB encode).
pub const PREVIEW_VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Creates a preview texture: `Rgba8Unorm` storage, renderable through an sRGB view.
pub fn create_preview_texture(gpu: &GpuContext, width: u32, height: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PREVIEW_FORMAT,
        // COPY_DST: the studio copies pictures into preview-sized textures (scene thumbnails).
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[PREVIEW_VIEW_FORMAT],
    })
}

/// The view to pass to [`OutputConverter::to_srgb`] for a preview texture.
pub fn preview_target_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(PREVIEW_VIEW_FORMAT),
        ..Default::default()
    })
}

/// NV12 output as two planes: Y (`R8Unorm`, full size) and interleaved CbCr (`Rg8Unorm`, half size).
#[derive(Debug)]
pub struct Nv12Target {
    pub y: wgpu::Texture,
    pub uv: wgpu::Texture,
    y_view: wgpu::TextureView,
    uv_view: wgpu::TextureView,
}

impl Nv12Target {
    /// `width` and `height` must be even.
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Self {
        debug_assert!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "NV12 requires even dimensions"
        );
        let y = gpu.create_render_texture("nv12 y", width, height, wgpu::TextureFormat::R8Unorm);
        let uv = gpu.create_render_texture(
            "nv12 uv",
            width / 2,
            height / 2,
            wgpu::TextureFormat::Rg8Unorm,
        );
        Self {
            y_view: y.create_view(&Default::default()),
            uv_view: uv.create_view(&Default::default()),
            y,
            uv,
        }
    }

    /// Wraps planes created elsewhere (e.g. shared with another API): `y` is `R8Unorm` at
    /// full size, `uv` is `Rg8Unorm` at half size, both renderable.
    pub fn from_planes(y: wgpu::Texture, uv: wgpu::Texture) -> Self {
        debug_assert_eq!(y.format(), wgpu::TextureFormat::R8Unorm);
        debug_assert_eq!(uv.format(), wgpu::TextureFormat::Rg8Unorm);
        Self {
            y_view: y.create_view(&Default::default()),
            uv_view: uv.create_view(&Default::default()),
            y,
            uv,
        }
    }
}

/// Bind group for one canvas texture; create once per canvas texture and reuse.
#[derive(Debug)]
pub struct ConvertInput(wgpu::BindGroup);

#[derive(Debug)]
pub struct OutputConverter {
    layout: wgpu::BindGroupLayout,
    srgb: wgpu::RenderPipeline,
    y: wgpu::RenderPipeline,
    uv: wgpu::RenderPipeline,
}

impl OutputConverter {
    pub fn new(gpu: &GpuContext) -> Arc<Self> {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/convert.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("convert"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("convert"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Arc::new(Self {
            srgb: make("fs_srgb", PREVIEW_VIEW_FORMAT),
            y: make("fs_y", wgpu::TextureFormat::R8Unorm),
            uv: make("fs_uv", wgpu::TextureFormat::Rg8Unorm),
            layout,
        })
    }

    pub fn create_input(&self, gpu: &GpuContext, canvas: &wgpu::TextureView) -> ConvertInput {
        debug_assert_eq!(canvas.texture().format(), CANVAS_FORMAT);
        ConvertInput(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert input"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(canvas),
            }],
        }))
    }

    /// Writes the canvas into a same-size preview texture via [`preview_target_view`].
    pub fn to_srgb(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &ConvertInput,
        target: &wgpu::TextureView,
    ) {
        Self::pass(encoder, &self.srgb, input, target);
    }

    /// Writes the canvas into NV12 planes (BT.709, limited range).
    pub fn to_nv12(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &ConvertInput,
        target: &Nv12Target,
    ) {
        Self::pass(encoder, &self.y, input, &target.y_view);
        Self::pass(encoder, &self.uv, input, &target.uv_view);
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        input: &ConvertInput,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("convert"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &input.0, &[]);
        pass.draw(0..3, 0..1);
    }
}
