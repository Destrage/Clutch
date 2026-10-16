//! GPU conversion of NV12 frames (cameras, capture cards, decoders) to RGB.

use crate::GpuContext;
use std::sync::Arc;

/// YUV → RGB color matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
}

impl YuvMatrix {
    fn coefficients(self) -> (f32, f32) {
        match self {
            YuvMatrix::Bt601 => (0.299, 0.114),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
        }
    }

    /// The usual default when a source doesn't say: HD uses BT.709, SD uses BT.601.
    pub fn for_height(height: u32) -> Self {
        if height >= 720 {
            YuvMatrix::Bt709
        } else {
            YuvMatrix::Bt601
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    kr: f32,
    kb: f32,
    full_range: u32,
    _pad: u32,
}

/// Shared pipeline for NV12 conversion (one per device).
#[derive(Debug)]
pub struct YuvConverter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

/// Output format of converted frames (sampled as linear by the compositor).
pub const YUV_OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

impl YuvConverter {
    pub fn new(gpu: &GpuContext) -> Arc<Self> {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/yuv.wgsl"));
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12 to rgb"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: YUV_OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Arc::new(Self { layout, pipeline })
    }

    pub fn create_frame(&self, gpu: &GpuContext, width: u32, height: u32) -> Nv12Frame {
        let (width, height) = (width.max(2) & !1, height.max(2) & !1);
        let plane = |label, w, h, format| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = plane("nv12 y", width, height, wgpu::TextureFormat::R8Unorm);
        let uv = plane(
            "nv12 uv",
            width / 2,
            height / 2,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12 params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &y.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &uv.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let output = gpu.create_render_texture("nv12 rgb", width, height, YUV_OUTPUT_FORMAT);
        Nv12Frame {
            output_view: output.create_view(&Default::default()),
            output,
            y,
            uv,
            params,
            bind_group,
            width,
            height,
        }
    }
}

/// GPU textures for one NV12 stream at a fixed size, plus its RGB output.
#[derive(Debug)]
pub struct Nv12Frame {
    y: wgpu::Texture,
    uv: wgpu::Texture,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pub output: wgpu::Texture,
    output_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Nv12Frame {
    /// Uploads an NV12 image: `height` rows of Y (`stride` bytes each) followed by
    /// `height / 2` rows of interleaved CbCr with the same stride.
    pub fn upload(
        &self,
        gpu: &GpuContext,
        data: &[u8],
        stride: u32,
        matrix: YuvMatrix,
        full_range: bool,
    ) {
        let (w, h) = (self.width, self.height);
        let y_len = (stride * h) as usize;
        let uv_len = (stride * (h / 2)) as usize;
        if data.len() < y_len + uv_len || stride < w {
            tracing::warn!(
                len = data.len(),
                stride,
                w,
                h,
                "NV12 buffer too small; frame dropped"
            );
            return;
        }
        let write = |texture: &wgpu::Texture, bytes: &[u8], rows: u32| {
            gpu.queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(rows),
                },
                texture.size(),
            );
        };
        write(&self.y, &data[..y_len], h);
        write(&self.uv, &data[y_len..y_len + uv_len], h / 2);
        let (kr, kb) = matrix.coefficients();
        let params = Params {
            kr,
            kb,
            full_range: full_range as u32,
            _pad: 0,
        };
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Records the conversion into `output`.
    pub fn convert(&self, converter: &YuvConverter, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nv12 to rgb"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.output_view,
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
        pass.set_pipeline(&converter.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
