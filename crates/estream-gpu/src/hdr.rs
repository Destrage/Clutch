//! HDR → SDR for screen captures of HDR displays: scRGB (linear BT.709, 1.0 = 80 nits) is
//! scaled so SDR white is 1.0, and brighter content is compressed into the SDR range instead
//! of clipping. Runs once per new captured frame.

use crate::context::GpuContext;

const SHADER: &str = include_str!("shaders/hdr.wgsl");

/// Format of the mapped picture (linear light, like the canvas).
pub const HDR_MAPPED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Where the shoulder starts, relative to SDR white. SDR content below it passes unchanged;
/// SDR white itself lands at about 95% (249 of 255 in sRGB).
pub const KNEE: f32 = 0.9;

/// Windows' SDR white level when it can't be read: the slider's default, 200 nits.
pub const DEFAULT_SDR_WHITE_NITS: f32 = 200.0;

/// What the display reports, turned into shader parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HdrParams {
    /// 80 / SDR white level in nits.
    pub scale: f32,
    /// Display peak / SDR white, at least 1.
    pub peak: f32,
    pub knee: f32,
    _pad: f32,
}

impl HdrParams {
    /// The bytes for the shader's uniform buffer.
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    /// From the SDR white level and the display's peak brightness, both in nits.
    pub fn new(sdr_white_nits: f32, peak_nits: f32) -> Self {
        let white = if sdr_white_nits.is_finite() && sdr_white_nits >= 1.0 {
            sdr_white_nits
        } else {
            DEFAULT_SDR_WHITE_NITS
        };
        let peak = if peak_nits.is_finite() {
            peak_nits
        } else {
            white
        };
        Self {
            scale: 80.0 / white,
            peak: (peak / white).max(1.0),
            knee: KNEE,
            _pad: 0.0,
        }
    }
}

/// The mapping on the CPU, exactly as the shader does it (for tests).
pub fn map_pixel(rgb: [f32; 3], p: &HdrParams) -> [f32; 3] {
    let c = rgb.map(|v| v.max(0.0) * p.scale);
    let m = c[0].max(c[1]).max(c[2]);
    if m <= p.knee {
        return c;
    }
    let k = shoulder(m, p) / m;
    c.map(|v| v * k)
}

fn shoulder(m: f32, p: &HdrParams) -> f32 {
    if m <= p.knee {
        return m;
    }
    let room = 1.0 - p.knee;
    let span = p.peak - p.knee;
    if span <= 0.0 {
        return m.min(1.0);
    }
    let t = (m - p.knee) / room;
    let white = span / room;
    let r = t * (1.0 + t / (white * white)) / (1.0 + t);
    (p.knee + room * r).min(1.0)
}

/// The pipeline, shared by every HDR capture on a device.
#[derive(Debug)]
pub struct HdrToSdr {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl HdrToSdr {
    pub fn new(gpu: &GpuContext) -> Self {
        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hdr to sdr"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<HdrParams>() as u64),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hdr to sdr"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hdr to sdr"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hdr to sdr"),
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
                    format: HDR_MAPPED_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    /// A uniform buffer for one capture's parameters (update it with `queue.write_buffer`).
    pub fn create_params(&self, gpu: &GpuContext, params: &HdrParams) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hdr params"),
                contents: bytemuck::bytes_of(params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Binds an scRGB texture (any float format) and its parameters.
    pub fn bind(
        &self,
        gpu: &GpuContext,
        input: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hdr to sdr"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    /// A texture of `width × height` to map into (sampled by the compositor afterwards).
    pub fn create_target(&self, gpu: &GpuContext, width: u32, height: u32) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr mapped"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_MAPPED_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Maps the bound input into `target` (same size as the input).
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hdr to sdr"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical HDR monitor: 200-nit SDR white, 1000-nit peak.
    fn params() -> HdrParams {
        HdrParams::new(200.0, 1000.0)
    }

    #[test]
    fn sdr_content_below_the_knee_is_unchanged() {
        let p = params();
        // SDR grey at half of SDR white: scRGB 0.5 × 2.5.
        let out = map_pixel([1.25, 0.5, 0.25], &p);
        assert!(
            (out[0] - 0.5).abs() < 1e-6 && (out[1] - 0.2).abs() < 1e-6,
            "{out:?}"
        );
    }

    #[test]
    fn sdr_white_stays_nearly_white() {
        let out = map_pixel([2.5, 2.5, 2.5], &params());
        assert!(out[0] > 0.94 && out[0] < 1.0, "{out:?}");
    }

    #[test]
    fn highlights_compress_up_to_the_peak_without_clipping() {
        let p = params();
        let level = |nits: f32| map_pixel([nits / 80.0; 3], &p)[0];
        let mut last = 0.0;
        for nits in (100..=1000).step_by(50) {
            let v = level(nits as f32);
            assert!(v >= last, "monotonic at {nits}");
            assert!(v <= 1.0);
            last = v;
        }
        assert!(
            level(400.0) < level(800.0),
            "detail kept between 400 and 800 nits"
        );
        assert!((level(1000.0) - 1.0).abs() < 1e-5, "the peak reaches white");
        assert_eq!(level(4000.0), 1.0, "beyond the peak");
    }

    #[test]
    fn hues_are_kept() {
        let out = map_pixel([8.0, 4.0, 2.0], &params());
        assert!((out[0] / out[1] - 2.0).abs() < 1e-4 && (out[1] / out[2] - 2.0).abs() < 1e-4);
        assert!(out[0] <= 1.0);
    }

    #[test]
    fn out_of_gamut_and_bad_reports_are_safe() {
        let p = params();
        let out = map_pixel([-0.2, 1.0, 0.5], &p);
        assert_eq!(out[0], 0.0);
        // Missing or nonsense levels fall back to 200 nits and a peak of SDR white.
        let p = HdrParams::new(f32::NAN, f32::NAN);
        assert_eq!(p.scale, 80.0 / DEFAULT_SDR_WHITE_NITS);
        assert_eq!(p.peak, 1.0);
        assert!(map_pixel([10.0; 3], &p)[0] <= 1.0);
    }
}
