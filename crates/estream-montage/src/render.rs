//! Drawing a montage frame on the GPU. Preview and export use the same [`Renderer`], so what
//! you see while editing is what gets exported.
//!
//! Each frame takes up to three passes:
//! 1. **Video:** the segment's source frame (decoded, uploaded and color converted), cropped
//!    and fitted into the output format. During a transition both segments are drawn into
//!    the transition's own textures and the transition shader mixes them.
//! 2. **Canvas:** the video, then the titles on top, with the end fade applied.
//! 3. **Output:** the canvas converted for the preview (sRGB) or for the encoder (NV12).

use crate::model::{Anchor, Crop, FrameAt, Montage, Title, TitleStyle, Transition};
use estream_capture::media::text::{self, TextSettings};
use estream_core::{BlendMode, Rgba8};
use estream_gpu::glam::{Affine2, Vec2};
use estream_gpu::{
    BuiltinKind, CANVAS_FORMAT, CanvasRenderer, Compositor, ConvertInput, DrawItem, GpuContext,
    Nv12Frame, Nv12Target, OutputConverter, TransitionParams, TransitionRenderer, TransitionShader,
    TransitionTargets, YuvConverter, YuvMatrix, wgpu,
};
use estream_media::frames::FrameReader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A push to the next segment with motion blur along the way.
pub const WHIP_SHADER: &str = "\
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let shift = ease(progress);
    let blur = sin(progress * 3.14159265) * 0.08;
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 9; i = i + 1) {
        let u = uv.x + shift + (f32(i) / 8.0 - 0.5) * blur;
        if (u < 1.0) {
            acc = acc + from_color(vec2<f32>(u, uv.y));
        } else {
            acc = acc + to_color(vec2<f32>(u - 1.0, uv.y));
        }
    }
    return acc / 9.0;
}
";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("{0}")]
    Media(#[from] estream_media::MediaError),
    #[error("{0}")]
    Gpu(#[from] estream_gpu::GpuError),
    #[error("{0} is missing")]
    Missing(String),
}

/// One source file in use: its reader and its frame on the GPU.
struct Source {
    reader: FrameReader,
    frame: Nv12Frame,
    binding: wgpu::BindGroup,
    /// The source time of the frame on the GPU (avoids re-uploading the same frame).
    shown: Option<f64>,
    /// Size of the decoded picture.
    size: (u32, u32),
}

/// A title's text rendered to a texture.
struct TitleTexture {
    key: (String, TitleStyle, u32),
    _texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    size: (u32, u32),
}

pub struct Renderer {
    gpu: GpuContext,
    width: u32,
    height: u32,
    compositor: Arc<Compositor>,
    yuv: Arc<YuvConverter>,
    transitions: TransitionRenderer,
    targets: TransitionTargets,
    /// Pass renderers (each owns its uniforms, so passes can share one submission).
    from_pass: CanvasRenderer,
    to_pass: CanvasRenderer,
    video_pass: CanvasRenderer,
    canvas_pass: CanvasRenderer,
    /// Kept alive for its view and binding.
    _video: wgpu::Texture,
    video_view: wgpu::TextureView,
    video_binding: wgpu::BindGroup,
    canvas: wgpu::Texture,
    canvas_view: wgpu::TextureView,
    converter: Arc<OutputConverter>,
    convert_input: ConvertInput,
    nv12: Nv12Target,
    sources: HashMap<PathBuf, Source>,
    titles: HashMap<u64, TitleTexture>,
}

impl Renderer {
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(2) & !1, height.max(2) & !1);
        let compositor = Compositor::new(gpu);
        let mut transitions = TransitionRenderer::new(gpu);
        if !transitions.prepare_custom(gpu, WHIP_SHADER) {
            tracing::error!("the whip transition shader didn't compile; whips will fade");
        }
        let targets = transitions.create_targets(gpu, width, height);
        let video = gpu.create_render_texture("montage video", width, height, CANVAS_FORMAT);
        let video_view = video.create_view(&Default::default());
        let video_binding = compositor.create_source_binding(gpu, &video_view);
        let canvas = gpu.create_render_texture("montage canvas", width, height, CANVAS_FORMAT);
        let canvas_view = canvas.create_view(&Default::default());
        let converter = OutputConverter::new(gpu);
        let convert_input = converter.create_input(gpu, &canvas_view);
        Self {
            gpu: gpu.clone(),
            width,
            height,
            yuv: YuvConverter::new(gpu),
            transitions,
            targets,
            from_pass: CanvasRenderer::new(gpu, compositor.clone()),
            to_pass: CanvasRenderer::new(gpu, compositor.clone()),
            video_pass: CanvasRenderer::new(gpu, compositor.clone()),
            canvas_pass: CanvasRenderer::new(gpu, compositor.clone()),
            compositor,
            _video: video,
            video_view,
            video_binding,
            canvas,
            canvas_view,
            converter,
            convert_input,
            nv12: Nv12Target::new(gpu, width, height),
            sources: HashMap::new(),
            titles: HashMap::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The canvas (linear `Rgba16Float`) after [`Renderer::render`].
    pub fn canvas(&self) -> &wgpu::Texture {
        &self.canvas
    }

    /// Closes readers of sources the montage no longer uses.
    pub fn forget_unused(&mut self, montage: &Montage) {
        self.sources
            .retain(|path, _| montage.segments.iter().any(|s| &s.source == path));
        self.titles
            .retain(|id, _| montage.titles.iter().any(|t| t.id == *id));
    }

    /// Uploads the frame of `path` showing at `time` (if it isn't the one already uploaded).
    fn prepare_source(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        time: f64,
    ) -> Result<(), RenderError> {
        if !self.sources.contains_key(path) {
            if !path.is_file() {
                return Err(RenderError::Missing(path.display().to_string()));
            }
            let reader = FrameReader::open(path)?;
            let frame = self
                .yuv
                .create_frame(&self.gpu, reader.width, reader.height);
            let view = frame.output.create_view(&Default::default());
            let binding = self.compositor.create_source_binding(&self.gpu, &view);
            let size = (reader.width, reader.height);
            self.sources.insert(
                path.to_path_buf(),
                Source {
                    reader,
                    frame,
                    binding,
                    shown: None,
                    size,
                },
            );
        }
        let Some(source) = self.sources.get_mut(path) else {
            return Ok(());
        };
        let Some(picture) = source.reader.frame_at(time)? else {
            return Ok(());
        };
        if source.shown == Some(picture.time) {
            return Ok(());
        }
        if (picture.width, picture.height) != source.size {
            source.size = (picture.width, picture.height);
            source.frame = self
                .yuv
                .create_frame(&self.gpu, picture.width, picture.height);
            let view = source.frame.output.create_view(&Default::default());
            source.binding = self.compositor.create_source_binding(&self.gpu, &view);
        }
        source.frame.upload(
            &self.gpu,
            &picture.data,
            picture.width,
            YuvMatrix::for_height(picture.height),
            picture.full_range,
        );
        source.frame.convert(&self.yuv, encoder);
        source.shown = Some(picture.time);
        Ok(())
    }

    /// Where a cropped source goes: the crop region fitted (letterboxed) into the frame.
    fn placement(&self, source_size: (u32, u32), crop: Crop) -> (Affine2, Vec2, Vec2) {
        let (sw, sh) = (source_size.0.max(1) as f32, source_size.1.max(1) as f32);
        let (w, h) = (self.width as f32, self.height as f32);
        let region = Vec2::new(sw * crop.w.max(1e-3), sh * crop.h.max(1e-3));
        let scale = (w / region.x).min(h / region.y);
        let size = region * scale;
        let offset = (Vec2::new(w, h) - size) / 2.0;
        let uv_min = Vec2::new(crop.x, crop.y);
        let uv_max = uv_min + Vec2::new(crop.w, crop.h);
        (
            Affine2::from_scale_angle_translation(size, 0.0, offset),
            uv_min,
            uv_max,
        )
    }

    fn title_texture(&mut self, title: &Title) -> Option<&TitleTexture> {
        let key = (title.text.clone(), title.style, self.height);
        if self.titles.get(&title.id).is_none_or(|t| t.key != key) {
            let h = self.height as f64;
            let settings = match title.style {
                TitleStyle::Bold => TextSettings {
                    size: (h * 0.075).max(16.0),
                    color: Rgba8::WHITE,
                    outline: true,
                    outline_color: Rgba8::BLACK,
                    outline_width: (h * 0.005).max(2.0),
                    shadow: true,
                    shadow_offset: (h * 0.004).max(2.0),
                    align: "center".into(),
                    wrap_width: (self.width as f64 * 0.9) as i64,
                    ..TextSettings::default()
                },
                TitleStyle::Clean => TextSettings {
                    size: (h * 0.045).max(12.0),
                    color: Rgba8::WHITE,
                    shadow: true,
                    shadow_offset: (h * 0.003).max(1.5),
                    align: "center".into(),
                    wrap_width: (self.width as f64 * 0.9) as i64,
                    ..TextSettings::default()
                },
            };
            let pixels = text::render(&settings, &title.text);
            if pixels.width == 0 || pixels.height == 0 {
                self.titles.remove(&title.id);
                return None;
            }
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("montage title"),
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
            self.gpu.queue.write_texture(
                texture.as_image_copy(),
                &pixels.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pixels.width * 4),
                    rows_per_image: Some(pixels.height),
                },
                texture.size(),
            );
            let binding = self
                .compositor
                .create_source_binding(&self.gpu, &texture.create_view(&Default::default()));
            self.titles.insert(
                title.id,
                TitleTexture {
                    key,
                    _texture: texture,
                    binding,
                    size: (pixels.width, pixels.height),
                },
            );
        }
        self.titles.get(&title.id)
    }

    /// Draws timeline time `t` of `montage` into the canvas.
    pub fn render(&mut self, montage: &Montage, t: f64) -> Result<(), RenderError> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let canvas_size = Vec2::new(self.width as f32, self.height as f32);
        let black = wgpu::Color::BLACK;
        let duration = montage.duration();
        let fade = if montage.fade_out > 0.0 {
            ((duration - t) / montage.fade_out as f64).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };

        // 1. Video.
        let frame = montage.frame_at(t);
        let has_video = !matches!(frame, FrameAt::Black);
        match frame {
            FrameAt::Black => {}
            FrameAt::Segment(i, st) => {
                let seg = &montage.segments[i];
                self.prepare_source(&mut encoder, &seg.source, st)?;
                if let Some(src) = self.sources.get(&seg.source) {
                    let (matrix, uv_min, uv_max) = self.placement(src.size, seg.crop);
                    self.video_pass.render(
                        &self.gpu,
                        &mut encoder,
                        &self.video_view,
                        canvas_size,
                        black,
                        &[DrawItem {
                            binding: &src.binding,
                            matrix,
                            uv_min,
                            uv_max,
                            opacity: 1.0,
                            blend: BlendMode::Normal,
                        }],
                    );
                }
            }
            FrameAt::Transition {
                from,
                to,
                transition,
                progress,
            } => {
                let (a, b) = (&montage.segments[from.0], &montage.segments[to.0]);
                self.prepare_source(&mut encoder, &a.source, from.1)?;
                self.prepare_source(&mut encoder, &b.source, to.1)?;
                let draw = |this: &Self, path: &Path, crop: Crop| {
                    this.sources.get(path).map(|src| {
                        let (matrix, uv_min, uv_max) = this.placement(src.size, crop);
                        (matrix, uv_min, uv_max)
                    })
                };
                if let (Some(pa), Some(pb), Some(sa), Some(sb)) = (
                    draw(self, &a.source, a.crop),
                    draw(self, &b.source, b.crop),
                    self.sources.get(&a.source),
                    self.sources.get(&b.source),
                ) {
                    let item = |binding, p: (Affine2, Vec2, Vec2)| DrawItem {
                        binding,
                        matrix: p.0,
                        uv_min: p.1,
                        uv_max: p.2,
                        opacity: 1.0,
                        blend: BlendMode::Normal,
                    };
                    self.from_pass.render(
                        &self.gpu,
                        &mut encoder,
                        &self.targets.from_view,
                        canvas_size,
                        black,
                        &[item(&sa.binding, pa)],
                    );
                    self.to_pass.render(
                        &self.gpu,
                        &mut encoder,
                        &self.targets.to_view,
                        canvas_size,
                        black,
                        &[item(&sb.binding, pb)],
                    );
                    let (shader, params) = match transition {
                        Transition::Whip { .. } => (
                            TransitionShader::Custom(WHIP_SHADER),
                            TransitionParams::default(),
                        ),
                        Transition::DipToBlack { .. } => (
                            TransitionShader::Builtin(BuiltinKind::FadeToColor),
                            TransitionParams {
                                color: [0.0, 0.0, 0.0, 1.0],
                                ..TransitionParams::default()
                            },
                        ),
                        Transition::Crossfade { .. } | Transition::Cut => (
                            TransitionShader::Builtin(BuiltinKind::Fade),
                            TransitionParams::default(),
                        ),
                    };
                    self.transitions.render(
                        &self.gpu,
                        &mut encoder,
                        &self.targets,
                        &self.video_view,
                        &shader,
                        &TransitionParams { progress, ..params },
                    );
                }
            }
        }

        // 2. Canvas: the video, then the titles.
        let mut title_items: Vec<(u64, Affine2, f32)> = Vec::new();
        let showing: Vec<Title> = montage.titles_at(t).cloned().collect();
        for title in &showing {
            let (w, h) = (self.width as f32, self.height as f32);
            let Some(tex) = self.title_texture(title) else {
                continue;
            };
            let size = Vec2::new(tex.size.0 as f32, tex.size.1 as f32);
            let x = (w - size.x) / 2.0;
            let y = match title.anchor {
                Anchor::Top => h * 0.08,
                Anchor::Center => (h - size.y) / 2.0,
                Anchor::Bottom => h * 0.88 - size.y,
            };
            // Quick fades in and out.
            let edge = 0.25;
            let into = (t - title.start) / edge;
            let out = (title.start + title.dur - t) / edge;
            let opacity = into.min(out).clamp(0.0, 1.0) as f32 * fade;
            title_items.push((
                title.id,
                Affine2::from_scale_angle_translation(size, 0.0, Vec2::new(x, y)),
                opacity,
            ));
        }
        let mut items = Vec::with_capacity(1 + title_items.len());
        if has_video {
            items.push(DrawItem {
                binding: &self.video_binding,
                matrix: Affine2::from_scale(canvas_size),
                uv_min: Vec2::ZERO,
                uv_max: Vec2::ONE,
                opacity: fade,
                blend: BlendMode::Normal,
            });
        }
        for (id, matrix, opacity) in &title_items {
            if let Some(tex) = self.titles.get(id) {
                items.push(DrawItem {
                    binding: &tex.binding,
                    matrix: *matrix,
                    uv_min: Vec2::ZERO,
                    uv_max: Vec2::ONE,
                    opacity: *opacity,
                    blend: BlendMode::Normal,
                });
            }
        }
        self.canvas_pass.render(
            &self.gpu,
            &mut encoder,
            &self.canvas_view,
            canvas_size,
            black,
            &items,
        );
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Writes the canvas into `target` (from [`estream_gpu::preview_target_view`]) for display.
    pub fn to_preview(&self, target: &wgpu::TextureView) {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.converter
            .to_srgb(&mut encoder, &self.convert_input, target);
        self.gpu.queue.submit([encoder.finish()]);
    }

    /// The canvas as tightly packed NV12 (for the encoder). Waits for the GPU.
    pub fn read_nv12(&self) -> Result<Vec<u8>, RenderError> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.converter
            .to_nv12(&mut encoder, &self.convert_input, &self.nv12);
        self.gpu.queue.submit([encoder.finish()]);
        let mut data = estream_gpu::readback::read_texture(&self.gpu, &self.nv12.y)?;
        data.extend(estream_gpu::readback::read_texture(
            &self.gpu,
            &self.nv12.uv,
        )?);
        Ok(data)
    }

    /// The canvas as sRGB RGBA8 pixels (tests, thumbnails). Waits for the GPU.
    pub fn read_rgba(&self) -> Result<Vec<u8>, RenderError> {
        let texture = estream_gpu::create_preview_texture(&self.gpu, self.width, self.height);
        self.to_preview(&estream_gpu::preview_target_view(&texture));
        Ok(estream_gpu::readback::read_texture(&self.gpu, &texture)?)
    }
}
