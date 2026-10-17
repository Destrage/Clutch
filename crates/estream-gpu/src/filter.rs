//! Video filters: per-source chains of GPU passes between a source and the compositor.
//!
//! Every pass reads its input through the compositor's source binding layout (texture +
//! sampler) and writes an `Rgba16Float` target exposed through the same layout, so a source's
//! own binding, a previous pass and the compositor all fit together. Colors are straight
//! alpha, linear light (what sources provide and the compositor expects).

use crate::{Compositor, GpuContext};
use std::collections::HashMap;
use std::sync::Arc;

const PRELUDE: &str = include_str!("shaders/filter_prelude.wgsl");
const BUILTIN: &str = include_str!("shaders/filter_builtin.wgsl");
const EPILOGUE: &str = "
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return effect(in.uv);
}
";
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Starting point offered in the shader editor for custom filters.
pub const EXAMPLE_CUSTOM_FILTER: &str = "\
// Wavy glass: distorts the source over time.
// Available: source(uv), size(), time(), param(0..3), to_srgb(c), to_linear(c), luma(c)
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let strength = param(0u) * 0.02;
    let speed = 1.0 + param(1u) * 4.0;
    let offset = vec2<f32>(
        sin(uv.y * 30.0 + time() * speed),
        cos(uv.x * 30.0 + time() * speed),
    ) * strength;
    return source(uv + offset);
}
";

// ---------------------------------------------------------------- catalog

/// A numeric filter parameter (drives the generic UI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterParam {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default: f32,
    pub unit: &'static str,
}

/// A file a filter needs (stored in `settings.file`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterFile {
    pub label: &'static str,
    pub extensions: &'static [&'static str],
}

/// What the UI and the engine know about a filter type.
#[derive(Clone, Copy, Debug)]
pub struct FilterKind {
    pub kind: &'static str,
    pub name: &'static str,
    pub params: &'static [FilterParam],
    pub file: Option<FilterFile>,
    /// Takes WGSL code in `settings.code`.
    pub code: bool,
}

const fn p(
    key: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    default: f32,
    unit: &'static str,
) -> FilterParam {
    FilterParam {
        key,
        label,
        min,
        max,
        step,
        default,
        unit,
    }
}

pub const KINDS: &[FilterKind] = &[
    FilterKind {
        kind: "color_correction",
        name: "Color correction",
        params: &[
            p("brightness", "Brightness", -1.0, 1.0, 0.01, 0.0, ""),
            p("contrast", "Contrast", -1.0, 1.0, 0.01, 0.0, ""),
            p("saturation", "Saturation", 0.0, 3.0, 0.01, 1.0, "×"),
            p("gamma", "Gamma", -2.0, 2.0, 0.01, 0.0, ""),
            p("hue_shift", "Hue shift", -180.0, 180.0, 1.0, 0.0, "°"),
            p("opacity", "Opacity", 0.0, 1.0, 0.01, 1.0, ""),
        ],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "lut",
        name: "LUT",
        params: &[p("strength", "Strength", 0.0, 1.0, 0.01, 1.0, "")],
        file: Some(FilterFile {
            label: "LUT file (.cube)",
            extensions: &["cube"],
        }),
        code: false,
    },
    FilterKind {
        kind: "chroma_key",
        name: "Chroma key",
        params: &[
            p("hue", "Key hue", 0.0, 360.0, 1.0, 120.0, "°"),
            p("similarity", "Similarity", 0.0, 1.0, 0.01, 0.25, ""),
            p("smoothness", "Smoothness", 0.0, 0.5, 0.01, 0.08, ""),
            p("spill", "Spill reduction", 0.0, 1.0, 0.01, 0.5, ""),
        ],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "luma_key",
        name: "Luma key",
        params: &[
            p("low", "Low", 0.0, 1.0, 0.01, 0.0, ""),
            p("high", "High", 0.0, 1.0, 0.01, 1.0, ""),
            p("smoothness", "Smoothness", 0.0, 0.5, 0.01, 0.05, ""),
        ],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "crop_pad",
        name: "Crop / pad",
        params: &[
            p("left", "Left", -4000.0, 4000.0, 1.0, 0.0, "px"),
            p("top", "Top", -4000.0, 4000.0, 1.0, 0.0, "px"),
            p("right", "Right", -4000.0, 4000.0, 1.0, 0.0, "px"),
            p("bottom", "Bottom", -4000.0, 4000.0, 1.0, 0.0, "px"),
        ],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "blur",
        name: "Blur",
        params: &[p("radius", "Radius", 0.0, 64.0, 1.0, 8.0, "px")],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "sharpen",
        name: "Sharpen",
        params: &[p("amount", "Amount", 0.0, 2.0, 0.01, 0.5, "")],
        file: None,
        code: false,
    },
    FilterKind {
        kind: "image_mask",
        name: "Image mask",
        params: &[p("use_luminance", "Use brightness", 0.0, 1.0, 1.0, 0.0, "")],
        file: Some(FilterFile {
            label: "Mask image",
            extensions: &["png", "jpg", "jpeg", "webp", "bmp"],
        }),
        code: false,
    },
    FilterKind {
        kind: "custom_shader",
        name: "Custom shader",
        params: &[
            p("p1", "Parameter 1", 0.0, 1.0, 0.01, 0.5, ""),
            p("p2", "Parameter 2", 0.0, 1.0, 0.01, 0.5, ""),
            p("p3", "Parameter 3", 0.0, 1.0, 0.01, 0.0, ""),
            p("p4", "Parameter 4", 0.0, 1.0, 0.01, 0.0, ""),
        ],
        file: None,
        code: true,
    },
];

pub fn kind(kind: &str) -> Option<&'static FilterKind> {
    KINDS.iter().find(|k| k.kind == kind)
}

/// Checks a custom filter before use. The error names the line in the user's code.
pub fn validate_custom_filter(code: &str) -> Result<(), String> {
    crate::user_shader::validate(
        PRELUDE,
        code,
        EPILOGUE,
        "effect",
        "fn effect(uv: vec2<f32>) -> vec4<f32>",
    )
}

// ---------------------------------------------------------------- ops

/// One resolved filter, ready for the GPU.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterOp {
    ColorCorrection([f32; 6]),
    /// Strength.
    Lut(f32),
    ChromaKey([f32; 4]),
    LumaKey([f32; 3]),
    /// Left, top, right, bottom in pixels (negative = pad).
    CropPad([f32; 4]),
    /// Radius in pixels.
    Blur(f32),
    Sharpen(f32),
    /// Use luminance instead of alpha.
    ImageMask(bool),
    Custom {
        code: String,
        params: [f32; 4],
    },
}

impl FilterOp {
    /// Builds an op from a catalog kind and a parameter lookup (missing values use defaults).
    pub fn from_params(
        kind: &str,
        value: impl Fn(&str) -> Option<f32>,
        code: Option<&str>,
    ) -> Option<Self> {
        let info = self::kind(kind)?;
        let v = |i: usize| -> f32 {
            let p = &info.params[i];
            value(p.key).unwrap_or(p.default).clamp(p.min, p.max)
        };
        Some(match kind {
            "color_correction" => FilterOp::ColorCorrection(std::array::from_fn(v)),
            "lut" => FilterOp::Lut(v(0)),
            "chroma_key" => FilterOp::ChromaKey(std::array::from_fn(v)),
            "luma_key" => FilterOp::LumaKey(std::array::from_fn(v)),
            "crop_pad" => FilterOp::CropPad(std::array::from_fn(v)),
            "blur" => FilterOp::Blur(v(0)),
            "sharpen" => FilterOp::Sharpen(v(0)),
            "image_mask" => FilterOp::ImageMask(v(0) > 0.5),
            "custom_shader" => FilterOp::Custom {
                code: code.unwrap_or(EXAMPLE_CUSTOM_FILTER).to_owned(),
                params: std::array::from_fn(v),
            },
            _ => return None,
        })
    }

    fn needs_lut(&self) -> bool {
        matches!(self, FilterOp::Lut(_))
    }

    fn needs_mask(&self) -> bool {
        matches!(self, FilterOp::ImageMask(_))
    }

    /// GPU passes: (shader kind, params). Blur needs two.
    fn passes(&self) -> Vec<(u32, [f32; 12])> {
        let pad = |values: &[f32]| {
            let mut out = [0.0; 12];
            out[..values.len()].copy_from_slice(values);
            out
        };
        match self {
            FilterOp::ColorCorrection(v) => vec![(1, pad(v))],
            FilterOp::Lut(s) => vec![(2, pad(&[*s]))],
            FilterOp::ChromaKey(v) => vec![(3, pad(v))],
            FilterOp::LumaKey(v) => vec![(4, pad(v))],
            FilterOp::CropPad(v) => vec![(5, pad(v))],
            FilterOp::Blur(r) => vec![(6, pad(&[*r])), (7, pad(&[*r]))],
            FilterOp::Sharpen(a) => vec![(8, pad(&[*a]))],
            FilterOp::ImageMask(l) => vec![(9, pad(&[if *l { 1.0 } else { 0.0 }]))],
            FilterOp::Custom { params, .. } => vec![(0, pad(params))],
        }
    }
}

// ---------------------------------------------------------------- LUT files

/// A parsed `.cube` 3D LUT, stored as RGBA8 (red varies fastest).
#[derive(Clone, Debug, PartialEq)]
pub struct CubeLut {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// Parses Adobe/Resolve `.cube` 3D LUTs (`LUT_3D_SIZE`, optional `DOMAIN_MIN/MAX`).
pub fn parse_cube(text: &str) -> Result<CubeLut, String> {
    let mut size = 0u32;
    let mut min = [0.0f32; 3];
    let mut max = [1.0f32; 3];
    let mut values: Vec<[f32; 3]> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let first = parts.next().unwrap_or_default();
        let floats = |parts: std::str::SplitWhitespace<'_>| -> Result<[f32; 3], String> {
            let v: Vec<f32> = parts
                .map(|p| {
                    p.parse::<f32>()
                        .map_err(|_| format!("line {}: bad number", n + 1))
                })
                .collect::<Result<_, _>>()?;
            v.try_into()
                .map_err(|_| format!("line {}: expected three numbers", n + 1))
        };
        match first {
            "TITLE" | "LUT_1D_SIZE" | "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                if first == "LUT_1D_SIZE" {
                    return Err("1D LUTs are not supported; use a 3D .cube LUT".into());
                }
            }
            "LUT_3D_SIZE" => {
                size = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|s| (2..=256).contains(s))
                    .ok_or_else(|| format!("line {}: invalid LUT_3D_SIZE", n + 1))?;
            }
            "DOMAIN_MIN" => min = floats(parts)?,
            "DOMAIN_MAX" => max = floats(parts)?,
            _ => {
                let mut all = vec![first];
                all.extend(parts);
                let v: Vec<f32> = all
                    .iter()
                    .map(|p| p.parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("line {}: unexpected '{line}'", n + 1))?;
                let rgb: [f32; 3] = v
                    .try_into()
                    .map_err(|_| format!("line {}: expected three numbers", n + 1))?;
                values.push(rgb);
            }
        }
    }
    if size == 0 {
        return Err("missing LUT_3D_SIZE".into());
    }
    let expected = (size as usize).pow(3);
    if values.len() != expected {
        return Err(format!(
            "expected {expected} entries for a {size}³ LUT, found {}",
            values.len()
        ));
    }
    let mut rgba = Vec::with_capacity(expected * 4);
    for v in values {
        for c in 0..3 {
            let range = (max[c] - min[c]).max(1e-6);
            rgba.push((((v[c] - min[c]) / range).clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        rgba.push(255);
    }
    Ok(CubeLut { size, rgba })
}

// ---------------------------------------------------------------- renderer

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    in_size: [f32; 2],
    out_size: [f32; 2],
    offset: [f32; 2],
    time_s: f32,
    kind: u32,
    p: [f32; 12],
}

/// Shared filter pipelines and layouts. One per device (owned by the video thread).
pub struct FilterRenderer {
    compositor: Arc<Compositor>,
    uniform_layout: wgpu::BindGroupLayout,
    aux_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    aux_sampler: wgpu::Sampler,
    dummy_lut: wgpu::TextureView,
    dummy_mask: wgpu::TextureView,
    builtin: wgpu::RenderPipeline,
    custom: HashMap<u64, Option<wgpu::RenderPipeline>>,
}

impl FilterRenderer {
    pub fn new(gpu: &GpuContext, compositor: Arc<Compositor>) -> Self {
        let device = &gpu.device;
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filter params"),
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
        let texture = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let aux_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filter aux"),
            entries: &[
                texture(0, wgpu::TextureViewDimension::D3),
                texture(1, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filter"),
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(compositor.source_layout()),
                Some(&aux_layout),
            ],
            immediate_size: 0,
        });
        let aux_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filter aux"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy_lut = Self::lut_texture(
            gpu,
            &CubeLut {
                size: 2,
                rgba: vec![255; 2 * 2 * 2 * 4],
            },
        );
        let dummy_mask = Self::mask_texture(gpu, 1, 1, &[255, 255, 255, 255]);
        let builtin = Self::pipeline(gpu, &pipeline_layout, &format!("{PRELUDE}\n{BUILTIN}"));
        Self {
            compositor,
            uniform_layout,
            aux_layout,
            pipeline_layout,
            aux_sampler,
            dummy_lut,
            dummy_mask,
            builtin,
            custom: HashMap::new(),
        }
    }

    fn pipeline(
        gpu: &GpuContext,
        layout: &wgpu::PipelineLayout,
        source: &str,
    ) -> wgpu::RenderPipeline {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("filter"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter"),
                layout: Some(layout),
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
                        format: TARGET_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    /// Compiles (once) a custom filter. `false` if invalid (the filter then passes through).
    pub fn prepare_custom(&mut self, gpu: &GpuContext, code: &str) -> bool {
        let key = crate::user_shader::hash(code);
        if let Some(entry) = self.custom.get(&key) {
            return entry.is_some();
        }
        let pipeline = match validate_custom_filter(code) {
            Ok(()) => Some(Self::pipeline(
                gpu,
                &self.pipeline_layout,
                &crate::user_shader::assemble(PRELUDE, code, EPILOGUE),
            )),
            Err(e) => {
                tracing::error!(error = %e, "custom filter rejected; passing the source through");
                None
            }
        };
        let ok = pipeline.is_some();
        self.custom.insert(key, pipeline);
        ok
    }

    /// Uploads a 3D LUT.
    pub fn lut_texture(gpu: &GpuContext, lut: &CubeLut) -> wgpu::TextureView {
        let size = wgpu::Extent3d {
            width: lut.size,
            height: lut.size,
            depth_or_array_layers: lut.size,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter lut"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &lut.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(lut.size * 4),
                rows_per_image: Some(lut.size),
            },
            size,
        );
        texture.create_view(&Default::default())
    }

    /// Uploads a mask image (8-bit sRGB RGBA).
    pub fn mask_texture(
        gpu: &GpuContext,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> wgpu::TextureView {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        texture.create_view(&Default::default())
    }

    fn aux_group(
        &self,
        gpu: &GpuContext,
        lut: Option<&wgpu::TextureView>,
        mask: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter aux"),
            layout: &self.aux_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(lut.unwrap_or(&self.dummy_lut)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(mask.unwrap_or(&self.dummy_mask)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.aux_sampler),
                },
            ],
        })
    }
}

// ---------------------------------------------------------------- chain

struct Target {
    width: u32,
    height: u32,
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
    texture: wgpu::Texture,
}

struct Pass {
    kind: u32,
    params: [f32; 12],
    uniforms: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    target: Option<Target>,
}

struct Step {
    op: FilterOp,
    passes: Vec<Pass>,
    lut: Option<(wgpu::TextureView, u32)>,
    /// File the LUT/mask came from.
    aux_tag: Option<String>,
    mask: Option<wgpu::TextureView>,
    aux: wgpu::BindGroup,
}

/// The filters of one source, with their GPU resources.
#[derive(Default)]
pub struct FilterChain {
    steps: Vec<Step>,
    /// (step, pass) that produced the latest output.
    last: Option<(usize, usize)>,
}

/// Output of a chain run: what the compositor should draw.
#[derive(Clone, Copy, Debug)]
pub struct FilterOutput<'a> {
    pub binding: &'a wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    /// The texture behind `binding` for chain outputs (readback in tests and tools).
    pub texture: Option<&'a wgpu::Texture>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Sets the ops. Steps whose op kind is unchanged keep their resources (targets, LUTs);
    /// parameter changes only update uniforms.
    pub fn set_ops(&mut self, renderer: &mut FilterRenderer, gpu: &GpuContext, ops: Vec<FilterOp>) {
        let mut old = std::mem::take(&mut self.steps);
        self.last = None;
        for op in ops {
            if let FilterOp::Custom { code, .. } = &op {
                renderer.prepare_custom(gpu, code);
            }
            let reuse = old
                .iter()
                .position(|s| std::mem::discriminant(&s.op) == std::mem::discriminant(&op))
                .map(|i| old.remove(i));
            let passes = op.passes();
            let step = match reuse {
                Some(mut step) if step.passes.len() == passes.len() => {
                    for (pass, (kind, params)) in step.passes.iter_mut().zip(passes) {
                        pass.kind = kind;
                        pass.params = params;
                    }
                    step.op = op;
                    step
                }
                _ => Step {
                    passes: passes
                        .into_iter()
                        .map(|(kind, params)| {
                            let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("filter params"),
                                size: size_of::<Uniform>() as u64,
                                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            });
                            let uniform_group =
                                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("filter params"),
                                    layout: &renderer.uniform_layout,
                                    entries: &[wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: uniforms.as_entire_binding(),
                                    }],
                                });
                            Pass {
                                kind,
                                params,
                                uniforms,
                                uniform_group,
                                target: None,
                            }
                        })
                        .collect(),
                    aux: renderer.aux_group(gpu, None, None),
                    lut: None,
                    aux_tag: None,
                    mask: None,
                    op,
                },
            };
            self.steps.push(step);
        }
    }

    /// Provides the LUT for step `index` (or clears it).
    pub fn set_lut(
        &mut self,
        renderer: &FilterRenderer,
        gpu: &GpuContext,
        index: usize,
        lut: Option<&CubeLut>,
    ) {
        if let Some(step) = self.steps.get_mut(index) {
            step.lut = lut.map(|l| (FilterRenderer::lut_texture(gpu, l), l.size));
            step.aux = renderer.aux_group(gpu, step.lut.as_ref().map(|l| &l.0), step.mask.as_ref());
            step.aux_tag = None;
        }
    }

    /// Label of the file whose LUT/mask step `index` holds (set with [`set_aux_tag`]).
    ///
    /// [`set_aux_tag`]: Self::set_aux_tag
    pub fn aux_tag(&self, index: usize) -> Option<&str> {
        self.steps.get(index)?.aux_tag.as_deref()
    }

    /// Remembers which file step `index`'s LUT/mask came from (survives parameter changes).
    pub fn set_aux_tag(&mut self, index: usize, tag: Option<String>) {
        if let Some(step) = self.steps.get_mut(index) {
            step.aux_tag = tag;
        }
    }

    /// Provides the mask image for step `index` (8-bit sRGB RGBA), or clears it.
    pub fn set_mask(
        &mut self,
        renderer: &FilterRenderer,
        gpu: &GpuContext,
        index: usize,
        mask: Option<(u32, u32, &[u8])>,
    ) {
        if let Some(step) = self.steps.get_mut(index) {
            step.mask = mask.map(|(w, h, rgba)| FilterRenderer::mask_texture(gpu, w, h, rgba));
            step.aux = renderer.aux_group(gpu, step.lut.as_ref().map(|l| &l.0), step.mask.as_ref());
            step.aux_tag = None;
        }
    }

    /// Records the chain on `source` (a source binding and its size). Returns what to draw,
    /// or `None` if every step was skipped (the caller then draws the source).
    pub fn run(
        &mut self,
        renderer: &FilterRenderer,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        source: FilterOutput<'_>,
        time_s: f32,
    ) -> Option<FilterOutput<'_>> {
        let input = source.binding;
        // Index of the last pass that produced output: (step, pass).
        let mut last: Option<(usize, usize)> = None;
        let (mut w, mut h) = (source.width.max(1), source.height.max(1));
        for si in 0..self.steps.len() {
            let ready = {
                let step = &self.steps[si];
                let custom_ok = match &step.op {
                    FilterOp::Custom { code, .. } => renderer
                        .custom
                        .get(&crate::user_shader::hash(code))
                        .is_some_and(|p| p.is_some()),
                    _ => true,
                };
                custom_ok
                    && (!step.op.needs_lut() || step.lut.is_some())
                    && (!step.op.needs_mask() || step.mask.is_some())
            };
            if !ready {
                continue; // not loaded yet (or invalid): pass through
            }
            for pi in 0..self.steps[si].passes.len() {
                let (out_w, out_h, offset) = match &self.steps[si].op {
                    FilterOp::CropPad([l, t, r, b]) => (
                        (w as f32 - l - r).round().max(1.0) as u32,
                        (h as f32 - t - b).round().max(1.0) as u32,
                        [*l, *t],
                    ),
                    _ => (w, h, [0.0, 0.0]),
                };
                // Input: the source, or the previous pass's target.
                let input_binding: *const wgpu::BindGroup = match last {
                    None => input,
                    Some((s, p)) => match &self.steps[s].passes[p].target {
                        Some(t) => &t.binding,
                        None => input,
                    },
                };
                let step = &mut self.steps[si];
                let mut params = step.passes[pi].params;
                if let Some((_, size)) = &step.lut {
                    params[1] = *size as f32;
                }
                let pipeline = match &step.op {
                    FilterOp::Custom { code, .. } => {
                        match renderer.custom.get(&crate::user_shader::hash(code)) {
                            Some(Some(p)) => p,
                            _ => continue,
                        }
                    }
                    _ => &renderer.builtin,
                };
                let pass = &mut step.passes[pi];
                if pass
                    .target
                    .as_ref()
                    .is_none_or(|t| t.width != out_w || t.height != out_h)
                {
                    let texture = gpu.create_render_texture("filter", out_w, out_h, TARGET_FORMAT);
                    let view = texture.create_view(&Default::default());
                    let binding = renderer.compositor.create_source_binding(gpu, &view);
                    pass.target = Some(Target {
                        width: out_w,
                        height: out_h,
                        view,
                        binding,
                        texture,
                    });
                }
                let uniform = Uniform {
                    in_size: [w as f32, h as f32],
                    out_size: [out_w as f32, out_h as f32],
                    offset,
                    time_s,
                    kind: pass.kind,
                    p: params,
                };
                gpu.queue
                    .write_buffer(&pass.uniforms, 0, bytemuck::bytes_of(&uniform));
                let Some(target) = &pass.target else { continue };
                // SAFETY: `input_binding` points either at the caller's `input` or at the
                // target of an earlier pass, which is never the pass being written here and
                // isn't reallocated until a later frame.
                let input_ref = unsafe { &*input_binding };
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("filter"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
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
                rp.set_pipeline(pipeline);
                rp.set_bind_group(0, &pass.uniform_group, &[]);
                rp.set_bind_group(1, input_ref, &[]);
                rp.set_bind_group(2, &step.aux, &[]);
                rp.draw(0..3, 0..1);
                drop(rp);
                last = Some((si, pi));
                (w, h) = (out_w, out_h);
            }
        }
        self.last = last;
        self.output()
    }

    /// The output of the most recent [`run`](Self::run), if it produced one.
    pub fn output(&self) -> Option<FilterOutput<'_>> {
        let (s, p) = self.last?;
        let target = self.steps.get(s)?.passes.get(p)?.target.as_ref()?;
        Some(FilterOutput {
            binding: &target.binding,
            width: target.width,
            height: target.height,
            texture: Some(&target.texture),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_parser_reads_size_domain_and_values() {
        let text = "# test\nTITLE \"x\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
                    0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = parse_cube(text).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(&lut.rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&lut.rgba[4..8], &[255, 0, 0, 255]);
        assert_eq!(&lut.rgba[28..32], &[255, 255, 255, 255]);
    }

    #[test]
    fn cube_parser_errors() {
        assert!(parse_cube("0 0 0\n").unwrap_err().contains("LUT_3D_SIZE"));
        assert!(
            parse_cube("LUT_3D_SIZE 2\n0 0 0\n")
                .unwrap_err()
                .contains("expected 8")
        );
        assert!(parse_cube("LUT_1D_SIZE 16\n").unwrap_err().contains("1D"));
        assert!(parse_cube("LUT_3D_SIZE 2\nfoo bar baz\n").is_err());
    }

    #[test]
    fn example_filter_is_valid_and_errors_name_the_line() {
        validate_custom_filter(EXAMPLE_CUSTOM_FILTER).unwrap_or_else(|e| panic!("{e}"));
        let e =
            validate_custom_filter("fn effect(uv: vec2<f32>) -> vec4<f32> {\n  return nope;\n}")
                .unwrap_err();
        assert!(e.starts_with("Line 2:"), "{e}");
        assert!(
            validate_custom_filter("fn x() {}")
                .unwrap_err()
                .contains("fn effect")
        );
    }

    #[test]
    fn ops_use_defaults_and_clamp() {
        let op = FilterOp::from_params("blur", |_| None, None).unwrap();
        assert_eq!(op, FilterOp::Blur(8.0));
        let op = FilterOp::from_params("blur", |_| Some(500.0), None).unwrap();
        assert_eq!(op, FilterOp::Blur(64.0));
        assert_eq!(op.passes().len(), 2);
        assert!(FilterOp::from_params("nope", |_| None, None).is_none());
        for k in KINDS {
            assert!(k.params.len() <= 12, "{}", k.kind);
            assert!(
                FilterOp::from_params(k.kind, |_| None, None).is_some(),
                "{}",
                k.kind
            );
        }
    }
}
