//! Runtime video sources: the trait every source implements, the type registry,
//! and the built-in `color` and `test_pattern` sources.

use crate::properties::{PropKind, Property, PropertySchema, SelectOption};
use estream_audio::AudioSource;
use estream_core::{Rgba8, Size, SourceId, Timestamp};
use estream_gpu::{Compositor, GpuContext, wgpu};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub use estream_core::SourceStatus;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("unknown source type '{0}'")]
    UnknownKind(String),
    #[error("invalid settings: {0}")]
    Settings(String),
    /// The type exists, but not on this system (e.g. game capture on Linux).
    #[error("{0}")]
    Unavailable(String),
}

/// Status text of a source whose type another system has but this one doesn't.
pub const NOT_ON_THIS_SYSTEM: &str = "Not available on this system";

/// GPU resources available to sources, and which source is being called.
pub struct SourceContext<'a> {
    pub gpu: &'a GpuContext,
    pub compositor: &'a Compositor,
    /// Stable across restarts: sources key per-machine state by it (e.g. screen-share grants).
    pub source: SourceId,
}

/// The latest frame of a source, ready for compositing.
#[derive(Clone, Copy, Debug)]
pub struct SourceFrame<'a> {
    pub binding: &'a wgpu::BindGroup,
    pub size: Size,
}

/// A producer of video frames. Lives on the video thread.
pub trait VideoSource: Send {
    /// Applies new settings. Called when the settings change.
    fn update(&mut self, ctx: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError>;

    /// Advances to time `now`, recording any GPU work into `encoder`. Called once per engine frame.
    fn tick(&mut self, ctx: &SourceContext<'_>, encoder: &mut wgpu::CommandEncoder, now: Timestamp);

    /// The most recent frame, or `None` if no frame has been produced yet.
    fn frame(&self) -> Option<SourceFrame<'_>>;

    fn status(&self) -> SourceStatus {
        SourceStatus::Active
    }
}

type Factory =
    dyn Fn(&SourceContext<'_>, &Value) -> Result<Box<dyn VideoSource>, SourceError> + Send + Sync;
type SchemaFn = dyn Fn(&Value) -> PropertySchema + Send + Sync;
type AudioFactory = dyn Fn(&Value) -> Result<Box<dyn AudioSource>, String> + Send + Sync;

/// Describes a source type: identity, UI text, settings schema and constructor.
#[derive(Clone)]
pub struct SourceType {
    pub kind: String,
    pub name: String,
    pub description: String,
    /// Builds the schema for the current settings (option lists such as
    /// devices or windows are enumerated each time).
    schema: Arc<SchemaFn>,
    /// Cheap schema used to validate commands (must not enumerate devices or
    /// windows: it runs on the engine's control thread).
    validation: Option<Arc<SchemaFn>>,
    factory: Option<Arc<Factory>>,
    audio: Option<Arc<AudioFactory>>,
    /// Why sources of this type can't run here (a placeholder for another system's type).
    unavailable: Option<String>,
}

impl SourceType {
    pub fn new(
        kind: &str,
        name: &str,
        description: &str,
        schema: impl Fn(&Value) -> PropertySchema + Send + Sync + 'static,
        factory: impl Fn(&SourceContext<'_>, &Value) -> Result<Box<dyn VideoSource>, SourceError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            description: description.into(),
            schema: Arc::new(schema),
            validation: None,
            factory: Some(Arc::new(factory)),
            audio: None,
            unavailable: None,
        }
    }

    /// An audio-only source type (microphones, desktop audio, app audio).
    pub fn audio(
        kind: &str,
        name: &str,
        description: &str,
        schema: impl Fn(&Value) -> PropertySchema + Send + Sync + 'static,
        factory: impl Fn(&Value) -> Result<Box<dyn AudioSource>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            description: description.into(),
            schema: Arc::new(schema),
            validation: None,
            factory: None,
            audio: Some(Arc::new(factory)),
            unavailable: None,
        }
    }

    /// A type another system has but this one doesn't. Its sources load, keep their settings
    /// and show [`NOT_ON_THIS_SYSTEM`]; a video type keeps its place in scenes, an audio-only
    /// type its channel (silent).
    pub fn unavailable(kind: &str, name: &str, description: &str, video: bool) -> Self {
        let reason = NOT_ON_THIS_SYSTEM.to_owned();
        let mut t = if video {
            let reason = reason.clone();
            Self::new(
                kind,
                name,
                description,
                |_| PropertySchema::new(vec![]),
                move |_, _| Err(SourceError::Unavailable(reason.clone())),
            )
        } else {
            let reason = reason.clone();
            Self::audio(
                kind,
                name,
                description,
                |_| PropertySchema::new(vec![]),
                move |_| Ok(Box::new(UnavailableAudio(reason.clone()))),
            )
        };
        t.unavailable = Some(reason);
        t
    }

    /// `false` for [`SourceType::unavailable`] placeholders.
    pub fn is_available(&self) -> bool {
        self.unavailable.is_none()
    }

    /// Adds an audio part to a video source type.
    pub fn with_audio(
        mut self,
        factory: impl Fn(&Value) -> Result<Box<dyn AudioSource>, String> + Send + Sync + 'static,
    ) -> Self {
        self.audio = Some(Arc::new(factory));
        self
    }

    pub fn has_video(&self) -> bool {
        self.factory.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// Uses schema for validation instead of the full (UI) schema.
    pub fn with_validation(
        mut self,
        schema: impl Fn(&Value) -> PropertySchema + Send + Sync + 'static,
    ) -> Self {
        self.validation = Some(Arc::new(schema));
        self
    }

    pub fn validation_schema(&self, settings: &Value) -> PropertySchema {
        match &self.validation {
            Some(v) => v(settings),
            None => self.schema(settings),
        }
    }

    pub fn schema(&self, settings: &Value) -> PropertySchema {
        (self.schema)(settings)
    }
}

/// Maps source type identifiers (e.g. `"color"`) to their [`SourceType`].
#[derive(Clone, Default)]
pub struct SourceRegistry {
    types: Vec<SourceType>,
}

impl std::fmt::Debug for SourceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set()
            .entries(self.types.iter().map(|t| &t.kind))
            .finish()
    }
}

impl SourceRegistry {
    /// A registry with all built-in source types.
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(SourceType::new(
            "test_pattern",
            "Test pattern",
            "Animated color bars for checking your setup",
            |_| {
                PropertySchema::new(vec![
                    Property::int("width", "Width", 1, 16384, 1920),
                    Property::int("height", "Height", 1, 16384, 1080),
                ])
            },
            |ctx, s| {
                let mut src = TestPatternSource::new(ctx);
                src.update(ctx, s)?;
                Ok(Box::new(src))
            },
        ));
        r.register(SourceType::new(
            "color",
            "Color",
            "A solid color block — backgrounds, frames, overlays",
            |_| {
                PropertySchema::new(vec![
                    Property::int("width", "Width", 1, 16384, 1920),
                    Property::int("height", "Height", 1, 16384, 1080),
                    Property::new(
                        "color",
                        "Color",
                        PropKind::Color,
                        serde_json::to_value(Rgba8::WHITE).unwrap_or_default(),
                    ),
                ])
            },
            |ctx, s| {
                let mut src = ColorSource::new(ctx);
                src.update(ctx, s)?;
                Ok(Box::new(src))
            },
        ));
        for t in audio_types() {
            r.register(t);
        }
        r
    }

    /// Adds or replaces a source type.
    pub fn register(&mut self, source_type: SourceType) {
        self.types.retain(|t| t.kind != source_type.kind);
        self.types.push(source_type);
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.types.iter().map(|t| t.kind.as_str())
    }

    pub fn types(&self) -> &[SourceType] {
        &self.types
    }

    pub fn get(&self, kind: &str) -> Option<&SourceType> {
        self.types.iter().find(|t| t.kind == kind)
    }

    /// Validates settings for `kind`. Unknown kinds are accepted (they show as unavailable).
    pub fn validate(&self, kind: &str, settings: &Value) -> Result<(), String> {
        match self.get(kind) {
            Some(t) => t.validation_schema(settings).validate(settings),
            None => Ok(()),
        }
    }

    pub fn create(
        &self,
        kind: &str,
        ctx: &SourceContext<'_>,
        settings: &Value,
    ) -> Result<Box<dyn VideoSource>, SourceError> {
        let source_type = self
            .get(kind)
            .ok_or_else(|| SourceError::UnknownKind(kind.to_owned()))?;
        match &source_type.factory {
            Some(factory) => factory(ctx, settings),
            None => Err(SourceError::UnknownKind(kind.to_owned())),
        }
    }

    /// Creates the audio part of a source; None if the kind has no audio.
    pub fn create_audio(
        &self,
        kind: &str,
        settings: &Value,
    ) -> Option<Result<Box<dyn AudioSource>, String>> {
        let factory = self.get(kind)?.audio.as_ref()?;
        Some(factory(settings))
    }
}

/// The silent audio part of an unavailable audio-only type.
struct UnavailableAudio(String);

impl AudioSource for UnavailableAudio {
    fn update(&mut self, _: &Value) -> Result<(), String> {
        Ok(())
    }

    fn read(&mut self, _: &mut estream_audio::Block, _: Timestamp) -> usize {
        0
    }

    fn status(&self) -> SourceStatus {
        SourceStatus::Unavailable(self.0.clone())
    }
}

/// Parses settings into a typed struct (missing fields use the struct's defaults).
pub fn parse<T: for<'de> Deserialize<'de> + Default>(settings: &Value) -> Result<T, SourceError> {
    if settings.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(settings.clone()).map_err(|e| SourceError::Settings(e.to_string()))
}

/// Checks a texture size is within GPU limits.
pub fn check_size(width: u32, height: u32) -> Result<(), SourceError> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(SourceError::Settings(format!(
            "size {width}x{height} must be within 1..=16384"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- color

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSettings {
    pub width: u32,
    pub height: u32,
    pub color: Rgba8,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            color: Rgba8::WHITE,
        }
    }
}

/// A solid color. Backed by a 1×1 texture stretched to the configured size.
pub struct ColorSource {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    size: Size,
}

impl ColorSource {
    fn new(ctx: &SourceContext<'_>) -> Self {
        let texture = ctx.gpu.create_render_texture(
            "color source",
            1,
            1,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let binding = ctx
            .compositor
            .create_source_binding(ctx.gpu, &texture.create_view(&Default::default()));
        Self {
            texture,
            binding,
            size: Size::default(),
        }
    }
}

impl VideoSource for ColorSource {
    fn update(&mut self, ctx: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let s: ColorSettings = parse(settings)?;
        check_size(s.width, s.height)?;
        ctx.gpu.queue.write_texture(
            self.texture.as_image_copy(),
            &s.color.to_array(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            self.texture.size(),
        );
        self.size = Size::new(s.width as f32, s.height as f32);
        Ok(())
    }

    fn tick(&mut self, _: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, _: Timestamp) {}

    fn frame(&self) -> Option<SourceFrame<'_>> {
        Some(SourceFrame {
            binding: &self.binding,
            size: self.size,
        })
    }
}

// ---------------------------------------------------------------- test pattern

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TestPatternSettings {
    pub width: u32,
    pub height: u32,
}

impl Default for TestPatternSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PatternUniform {
    size: [f32; 2],
    time: f32,
    _pad: f32,
}

/// SMPTE-style color bars with a moving marker, rendered on the GPU every frame.
pub struct TestPatternSource {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    target: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    size: Size,
}

impl TestPatternSource {
    fn new(ctx: &SourceContext<'_>) -> Self {
        let device = &ctx.gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/test_pattern.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test pattern"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test pattern"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test pattern"),
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test pattern"),
            size: size_of::<PatternUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test pattern"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            uniforms,
            uniform_group,
            target: None,
            size: Size::default(),
        }
    }
}

impl VideoSource for TestPatternSource {
    fn update(&mut self, ctx: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let s: TestPatternSettings = parse(settings)?;
        check_size(s.width, s.height)?;
        let size = Size::new(s.width as f32, s.height as f32);
        if self.target.is_none() || self.size != size {
            let texture = ctx.gpu.create_render_texture(
                "test pattern",
                s.width,
                s.height,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            );
            let view = texture.create_view(&Default::default());
            let binding = ctx.compositor.create_source_binding(ctx.gpu, &view);
            self.target = Some((texture, view, binding));
            self.size = size;
        }
        Ok(())
    }

    fn tick(
        &mut self,
        ctx: &SourceContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        now: Timestamp,
    ) {
        let Some((_, view, _)) = &self.target else {
            return;
        };
        let uniform = PatternUniform {
            size: [self.size.width, self.size.height],
            time: (now.as_secs_f64() % 3600.0) as f32,
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniform));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test pattern"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
        pass.set_bind_group(0, &self.uniform_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.target.as_ref().map(|(_, _, binding)| SourceFrame {
            binding,
            size: self.size,
        })
    }
}

// ---------------------------------------------------------------- audio types

fn device_schema(direction: estream_audio::devices::Direction, label: &str) -> PropertySchema {
    let devices = estream_audio::devices::devices(direction);
    let default_name = devices
        .iter()
        .find(|d| d.is_default)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "none found".into());
    let mut options = vec![SelectOption::new("", format!("Default ({default_name})"))];
    options.extend(devices.into_iter().map(|d| SelectOption::new(d.id, d.name)));
    PropertySchema::new(vec![Property::select("device", label, options, "")])
}

fn device_validation(_: &Value) -> PropertySchema {
    PropertySchema::new(vec![Property::select("device", "Device", vec![], "")])
}

fn audio_types() -> Vec<SourceType> {
    use estream_audio::devices::{DeviceSource, Direction};
    vec![
        SourceType::audio(
            "audio_input",
            "Audio input",
            "Microphones and line inputs",
            |_| device_schema(Direction::Input, "Device"),
            |settings| {
                let mut s = DeviceSource::new(Direction::Input);
                s.update(settings)?;
                Ok(Box::new(s))
            },
        )
        .with_validation(device_validation),
        SourceType::audio(
            "audio_output",
            "Desktop audio",
            "Everything playing on a speaker or headphone device",
            |_| device_schema(Direction::Output, "Device"),
            |settings| {
                let mut s = DeviceSource::new(Direction::Output);
                s.update(settings)?;
                Ok(Box::new(s))
            },
        )
        .with_validation(device_validation),
        SourceType::audio(
            "audio_test_tone",
            "Test tone",
            "A sine wave for checking levels and routing",
            |_| {
                PropertySchema::new(vec![
                    Property::float("frequency", "Frequency (Hz)", 20.0, 20000.0, 1.0, 440.0),
                    Property::float("level_db", "Level (dBFS)", -60.0, 0.0, 0.5, -18.0),
                ])
            },
            |settings| {
                let mut s = estream_audio::tone::ToneSource::default();
                s.update(settings)?;
                Ok(Box::new(s))
            },
        ),
    ]
}
