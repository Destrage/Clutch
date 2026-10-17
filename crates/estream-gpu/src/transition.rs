//! Scene transitions: blends an outgoing and an incoming canvas into the target.
//! Built-in kinds share one shader; custom kinds are user WGSL functions placed between
//! a fixed prelude and epilogue, validated with naga before they reach the GPU.

use crate::{CANVAS_FORMAT, GpuContext};
use std::collections::HashMap;

const PRELUDE: &str = include_str!("shaders/transition_prelude.wgsl");
const BUILTIN: &str = include_str!("shaders/transition_builtin.wgsl");
const EPILOGUE: &str = "
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return transition(in.uv, clamp(params.progress, 0.0, 1.0));
}
";

/// Starting point offered in the custom shader editor.
pub const EXAMPLE_CUSTOM_TRANSITION: &str = "\
// Circle reveal: the new scene grows from the center.
// Available: from_color(uv), to_color(uv), resolution(), ease(t), over(a, b)
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let aspect = resolution().x / resolution().y;
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let radius = ease(progress) * 1.2 * aspect;
    let t = 1.0 - smoothstep(radius - 0.02, radius, length(p));
    return mix(from_color(uv), to_color(uv), t);
}
";

/// Built-in transition shapes understood by the shared shader.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuiltinKind {
    Fade,
    FadeToColor,
    Slide,
    Swipe,
    Wipe,
}

impl BuiltinKind {
    fn index(self) -> u32 {
        match self {
            BuiltinKind::Fade => 1,
            BuiltinKind::FadeToColor => 2,
            BuiltinKind::Slide => 3,
            BuiltinKind::Swipe => 4,
            BuiltinKind::Wipe => 5,
        }
    }
}

/// Which shader draws a transition.
#[derive(Clone, Debug, PartialEq)]
pub enum TransitionShader<'a> {
    Builtin(BuiltinKind),
    Custom(&'a str),
}

/// Per-frame parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionParams {
    /// 0 = only the outgoing scene, 1 = only the incoming one.
    pub progress: f32,
    /// Direction the incoming scene travels (unit vector, y down).
    pub direction: [f32; 2],
    /// Linear, premultiplied color for fade-to-color.
    pub color: [f32; 4],
    /// Edge softness for wipes, as a fraction of the canvas.
    pub softness: f32,
}

impl Default for TransitionParams {
    fn default() -> Self {
        Self {
            progress: 0.0,
            direction: [-1.0, 0.0],
            color: [0.0, 0.0, 0.0, 1.0],
            softness: 0.1,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    progress: f32,
    kind: u32,
    softness: f32,
    _pad: f32,
    direction: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
}

/// Assembles the full module for a custom transition.
fn custom_source(code: &str) -> String {
    crate::user_shader::assemble(PRELUDE, code, EPILOGUE)
}

/// Checks a custom transition before use. The error names the line in the user's code.
pub fn validate_custom_transition(code: &str) -> Result<(), String> {
    crate::user_shader::validate(
        PRELUDE,
        code,
        EPILOGUE,
        "transition",
        "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>",
    )
}

/// Pipelines and layouts for transitions. One per device; custom pipelines are cached.
pub struct TransitionRenderer {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    builtin: wgpu::RenderPipeline,
    custom: HashMap<u64, Option<wgpu::RenderPipeline>>,
}

/// Scratch canvases (outgoing / incoming) and the bind group sampling them.
pub struct TransitionTargets {
    pub width: u32,
    pub height: u32,
    pub from_view: wgpu::TextureView,
    pub to_view: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _from: wgpu::Texture,
    _to: wgpu::Texture,
}

impl TransitionRenderer {
    pub fn new(gpu: &GpuContext) -> Self {
        let device = &gpu.device;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transition"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<Uniform>() as u64),
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transition"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transition"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let builtin = Self::pipeline(gpu, &pipeline_layout, &format!("{PRELUDE}\n{BUILTIN}"));
        Self {
            layout,
            pipeline_layout,
            sampler,
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
                label: Some("transition"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("transition"),
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
                        format: CANVAS_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    /// Compiles (once) the pipeline for custom `code`. `false` if the code is invalid,
    /// in which case [`render`](Self::render) falls back to a fade.
    pub fn prepare_custom(&mut self, gpu: &GpuContext, code: &str) -> bool {
        let key = crate::user_shader::hash(code);
        if let Some(entry) = self.custom.get(&key) {
            return entry.is_some();
        }
        let pipeline = match validate_custom_transition(code) {
            Ok(()) => Some(Self::pipeline(
                gpu,
                &self.pipeline_layout,
                &custom_source(code),
            )),
            Err(e) => {
                tracing::error!(error = %e, "custom transition rejected; using a fade");
                None
            }
        };
        let ok = pipeline.is_some();
        self.custom.insert(key, pipeline);
        ok
    }

    /// Scratch targets for a canvas of this size.
    pub fn create_targets(&self, gpu: &GpuContext, width: u32, height: u32) -> TransitionTargets {
        let from = gpu.create_render_texture("transition from", width, height, CANVAS_FORMAT);
        let to = gpu.create_render_texture("transition to", width, height, CANVAS_FORMAT);
        let from_view = from.create_view(&Default::default());
        let to_view = to.create_view(&Default::default());
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition params"),
            size: size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&from_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&to_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        TransitionTargets {
            width,
            height,
            from_view,
            to_view,
            uniforms,
            bind_group,
            _from: from,
            _to: to,
        }
    }

    /// Blends `targets.from_view` and `targets.to_view` into `target` (a canvas-format view).
    pub fn render(
        &self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        targets: &TransitionTargets,
        target: &wgpu::TextureView,
        shader: &TransitionShader<'_>,
        params: &TransitionParams,
    ) {
        let (pipeline, kind) = match shader {
            TransitionShader::Builtin(kind) => (&self.builtin, kind.index()),
            TransitionShader::Custom(code) => {
                match self.custom.get(&crate::user_shader::hash(code)) {
                    Some(Some(p)) => (p, 0),
                    _ => (&self.builtin, BuiltinKind::Fade.index()),
                }
            }
        };
        let uniform = Uniform {
            progress: params.progress,
            kind,
            softness: params.softness,
            _pad: 0.0,
            direction: params.direction,
            size: [targets.width as f32, targets.height as f32],
            color: params.color,
        };
        gpu.queue
            .write_buffer(&targets.uniforms, 0, bytemuck::bytes_of(&uniform));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &targets.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_shader_is_valid() {
        validate_custom_transition(EXAMPLE_CUSTOM_TRANSITION).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn errors_point_at_the_users_line() {
        let code = "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {\n    let x = 1.0;\n    return from_colour(uv);\n}\n";
        let e = validate_custom_transition(code).unwrap_err();
        assert!(e.starts_with("Line 3:"), "{e}");
        let syntax = "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {\n    return to_color(uv)\n}\n";
        let e = validate_custom_transition(syntax).unwrap_err();
        assert!(e.contains("Line "), "{e}");
    }

    #[test]
    fn missing_function_is_explained() {
        let e = validate_custom_transition("fn other() {}").unwrap_err();
        assert!(e.contains("fn transition"), "{e}");
    }
}
