//! The video pipeline: keeps runtime sources in sync with the render graph,
//! renders each canvas on its own frame clock, and publishes the results.

use crate::feed::{ReadbackRing, VideoConsumer};
use crate::filters::Filters;
use crate::frame_clock::{FrameClock, FrameTick, wait_until};
use crate::graph::{CanvasGraph, FlatItem, RenderGraph};
use crate::source::{SourceContext, SourceRegistry, SourceStatus, VideoSource};
use crate::watchdog::{FinishGuard, Stage, VideoProgress};
use estream_core::color::srgb_to_linear;
use estream_core::{CanvasId, EngineClock, Rgba8, SourceId, Timestamp};
use estream_gpu::{
    BuiltinKind, CANVAS_FORMAT, CanvasRenderer, Compositor, ConvertInput, DrawItem, FilterOutput,
    GpuContext, OutputConverter, TransitionParams, TransitionRenderer, TransitionShader,
    TransitionTargets, wgpu,
};
use estream_project::TransitionKind;
use glam::Vec2;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Frames kept per canvas so readers can use one while the next is rendered.
pub const OUTPUT_RING: usize = 3;
const NO_FRAME: u64 = u64::MAX;

#[derive(Debug)]
struct OutputSlot {
    canvas: wgpu::Texture,
    preview: wgpu::Texture,
}

/// Performance counters for a canvas.
#[derive(Debug, Default)]
pub struct CanvasStats {
    pub rendered: AtomicU64,
    pub lagged: AtomicU64,
    /// CPU time spent recording the last frame, in nanoseconds.
    pub last_render_ns: AtomicU64,
    /// Frames not delivered to an encoder (readback or encoder queue busy).
    pub encoder_dropped: AtomicU64,
}

/// The published output of one canvas. Shared with the UI and outputs.
#[derive(Debug)]
pub struct CanvasOutput {
    pub id: CanvasId,
    pub width: u32,
    pub height: u32,
    slots: [OutputSlot; OUTPUT_RING],
    /// `(sequence << 8) | slot` of the most recent finished frame.
    latest: AtomicU64,
    timestamp: AtomicU64,
    pub stats: CanvasStats,
}

/// A finished canvas frame.
#[derive(Clone, Copy, Debug)]
pub struct CanvasFrame<'a> {
    /// Linear-light `Rgba16Float` canvas.
    pub canvas: &'a wgpu::Texture,
    /// sRGB 8-bit preview of the same frame.
    pub preview: &'a wgpu::Texture,
    pub sequence: u64,
    pub timestamp: Timestamp,
}

impl CanvasOutput {
    fn new(gpu: &GpuContext, id: CanvasId, width: u32, height: u32) -> Self {
        let slots = std::array::from_fn(|_| OutputSlot {
            canvas: gpu.create_render_texture("canvas", width, height, CANVAS_FORMAT),
            preview: estream_gpu::create_preview_texture(gpu, width, height),
        });
        Self {
            id,
            width,
            height,
            slots,
            latest: AtomicU64::new(NO_FRAME),
            timestamp: AtomicU64::new(0),
            stats: CanvasStats::default(),
        }
    }

    /// The latest completed frame, if any has been rendered yet.
    pub fn latest(&self) -> Option<CanvasFrame<'_>> {
        let packed = self.latest.load(Ordering::Acquire);
        if packed == NO_FRAME {
            return None;
        }
        let slot = &self.slots[(packed & 0xff) as usize];
        Some(CanvasFrame {
            canvas: &slot.canvas,
            preview: &slot.preview,
            sequence: packed >> 8,
            timestamp: Timestamp(self.timestamp.load(Ordering::Acquire)),
        })
    }

    fn publish(&self, sequence: u64, slot: usize, timestamp: Timestamp) {
        self.timestamp.store(timestamp.0, Ordering::Release);
        self.latest
            .store((sequence << 8) | slot as u64, Ordering::Release);
    }
}

/// Runtime information about a source, published to consumers.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub status: SourceStatus,
    /// Size of the latest frame, if one exists.
    pub size: Option<estream_core::Size>,
}

struct SourceSlot {
    kind: String,
    settings: Value,
    instance: Option<Box<dyn VideoSource>>,
    status: SourceStatus,
}

struct CanvasSlot {
    output: Arc<CanvasOutput>,
    renderer: CanvasRenderer,
    clock: FrameClock,
    canvas_views: [wgpu::TextureView; OUTPUT_RING],
    preview_views: [wgpu::TextureView; OUTPUT_RING],
    inputs: [ConvertInput; OUTPUT_RING],
    sequence: u64,
    background: wgpu::Color,
    /// NV12 readback for encoders (only while a consumer is attached).
    readback: Option<ReadbackRing>,
    /// Renders the outgoing scene during transitions (separate uniform buffer).
    from_renderer: Option<CanvasRenderer>,
    /// Outgoing / incoming scratch canvases, allocated on the first transition.
    transition_targets: Option<TransitionTargets>,
    /// Studio-mode Preview render (only while studio mode is on).
    preview: Option<StudioPreview>,
}

/// A second render of the canvas showing the studio Preview scene. UI only; never encoded.
struct StudioPreview {
    output: Arc<CanvasOutput>,
    renderer: CanvasRenderer,
    canvas_views: [wgpu::TextureView; OUTPUT_RING],
    preview_views: [wgpu::TextureView; OUTPUT_RING],
    inputs: [ConvertInput; OUTPUT_RING],
    sequence: u64,
    /// Slot rendered this frame, published with the program frame.
    pending: Option<usize>,
}

impl StudioPreview {
    fn publish(&mut self, timestamp: Timestamp) {
        if let Some(slot) = self.pending.take() {
            self.sequence += 1;
            self.output.publish(self.sequence, slot, timestamp);
            self.output.stats.rendered.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Collects drawable items for `flats` into `out` (sources without a frame draw nothing).
fn push_items<'a>(
    sources: &'a HashMap<SourceId, SourceSlot>,
    filters: &'a Filters,
    flats: &[FlatItem],
    out: &mut Vec<DrawItem<'a>>,
) {
    for flat in flats {
        let Some((binding, size)) = source_frame(sources, filters, flat.source) else {
            continue;
        };
        let layout = flat.transform.layout(flat.crop, size);
        if layout.size.x <= 0.0 || layout.size.y <= 0.0 {
            continue;
        }
        out.push(DrawItem {
            binding,
            matrix: flat.parent * layout.matrix,
            uv_min: layout.uv_min,
            uv_max: layout.uv_max,
            opacity: flat.opacity,
            blend: flat.blend,
        });
    }
}

/// What to draw for a source: its filtered frame if it has filters, else its raw frame.
fn source_frame<'a>(
    sources: &'a HashMap<SourceId, SourceSlot>,
    filters: &'a Filters,
    id: SourceId,
) -> Option<(&'a wgpu::BindGroup, estream_core::Size)> {
    match filters.output(id) {
        Some(f) => Some((
            f.binding,
            estream_core::Size::new(f.width as f32, f.height as f32),
        )),
        None => sources
            .get(&id)
            .and_then(|s| s.instance.as_deref())
            .and_then(|s| s.frame())
            .map(|f| (f.binding, f.size)),
    }
}

/// Reuses a cleared `Vec<DrawItem>` allocation with a new lifetime (in-place collect
/// keeps capacity, so steady-state frames don't allocate).
fn reuse<'a>(scratch: &mut Vec<DrawItem<'static>>) -> Vec<DrawItem<'a>> {
    std::mem::take(scratch)
        .into_iter()
        .map(|_| -> DrawItem<'a> { unreachable!("scratch is always empty") })
        .collect()
}

fn recycle(mut items: Vec<DrawItem<'_>>, scratch: &mut Vec<DrawItem<'static>>) {
    items.clear();
    *scratch = items
        .into_iter()
        .map(|_| -> DrawItem<'static> { unreachable!("cleared above") })
        .collect();
}

/// Shader and parameters for a transition kind at `progress`.
fn transition_setup(
    kind: &TransitionKind,
    progress: f32,
) -> (TransitionShader<'_>, TransitionParams) {
    let mut params = TransitionParams {
        progress,
        ..Default::default()
    };
    let shader = match kind {
        // Stingers are drawn as an overlay; Fade is only a fallback.
        TransitionKind::Cut | TransitionKind::Fade | TransitionKind::Stinger { .. } => {
            TransitionShader::Builtin(BuiltinKind::Fade)
        }
        TransitionKind::FadeToColor { color } => {
            params.color = linear_premultiplied(*color);
            TransitionShader::Builtin(BuiltinKind::FadeToColor)
        }
        TransitionKind::Slide { direction } => {
            params.direction = direction.vector();
            TransitionShader::Builtin(BuiltinKind::Slide)
        }
        TransitionKind::Swipe { direction } => {
            params.direction = direction.vector();
            TransitionShader::Builtin(BuiltinKind::Swipe)
        }
        TransitionKind::Wipe {
            direction,
            softness,
        } => {
            params.direction = direction.vector();
            params.softness = softness.clamp(0.0, 1.0);
            TransitionShader::Builtin(BuiltinKind::Wipe)
        }
        TransitionKind::Shader { code } => TransitionShader::Custom(code),
    };
    (shader, params)
}

fn linear_premultiplied(c: Rgba8) -> [f32; 4] {
    let a = c.a as f32 / 255.0;
    let lin = |v: u8| srgb_to_linear(v as f32 / 255.0) * a;
    [lin(c.r), lin(c.g), lin(c.b), a]
}

/// Owns sources and canvas renderers. Used by the video thread and by headless rendering.
pub struct VideoCore {
    gpu: GpuContext,
    compositor: Arc<Compositor>,
    converter: Arc<OutputConverter>,
    registry: SourceRegistry,
    sources: HashMap<SourceId, SourceSlot>,
    canvases: Vec<CanvasSlot>,
    graph: Arc<RenderGraph>,
    scratch: Vec<DrawItem<'static>>,
    scratch_extra: Vec<DrawItem<'static>>,
    transitions: TransitionRenderer,
    filters: Filters,
    progress: Arc<VideoProgress>,
}

impl VideoCore {
    pub fn new(gpu: GpuContext, registry: SourceRegistry) -> Self {
        let compositor = Compositor::new(&gpu);
        Self {
            filters: Filters::new(&gpu, compositor.clone()),
            compositor,
            converter: OutputConverter::new(&gpu),
            transitions: TransitionRenderer::new(&gpu),
            gpu,
            registry,
            sources: HashMap::new(),
            canvases: Vec::new(),
            graph: Arc::new(RenderGraph::default()),
            scratch: Vec::new(),
            scratch_extra: Vec::new(),
            progress: Arc::new(VideoProgress::new(EngineClock::new())),
        }
    }

    /// Where the video loop is, for the stall watchdog.
    pub(crate) fn progress(&self) -> Arc<VideoProgress> {
        self.progress.clone()
    }

    /// Studio-mode Preview outputs of canvases that have one.
    pub fn preview_outputs(&self) -> Vec<Arc<CanvasOutput>> {
        self.canvases
            .iter()
            .filter_map(|c| c.preview.as_ref().map(|p| p.output.clone()))
            .collect()
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    pub fn outputs(&self) -> Vec<Arc<CanvasOutput>> {
        self.canvases.iter().map(|c| c.output.clone()).collect()
    }

    /// Status and current frame size of every source.
    pub fn statuses(&self) -> HashMap<SourceId, SourceInfo> {
        self.sources
            .iter()
            .map(|(id, s)| {
                let size = s
                    .instance
                    .as_deref()
                    .and_then(|i| i.frame())
                    .map(|f| f.size);
                (
                    *id,
                    SourceInfo {
                        // Live status from the source unless its settings failed to apply.
                        status: match (&s.status, s.instance.as_deref()) {
                            (SourceStatus::Error(_), _) | (_, None) => s.status.clone(),
                            (_, Some(instance)) => instance.status(),
                        },
                        size,
                    },
                )
            })
            .collect()
    }

    /// Reconciles sources and canvases with `graph`. Allocates only when something changed.
    pub fn sync(&mut self, graph: Arc<RenderGraph>, now: Timestamp) {
        self.sources
            .retain(|id, _| graph.sources.iter().any(|s| s.id == *id));
        for def in &graph.sources {
            let ctx = SourceContext {
                gpu: &self.gpu,
                compositor: &self.compositor,
                source: def.id,
            };
            // Audio-only types (microphones…) have no video part.
            if self.registry.get(&def.kind).is_some_and(|t| !t.has_video()) {
                self.sources.remove(&def.id);
                continue;
            }
            match self.sources.get_mut(&def.id) {
                Some(slot) if slot.kind == def.kind => {
                    if slot.settings != def.settings {
                        slot.settings = def.settings.clone();
                        if let Some(instance) = &mut slot.instance {
                            slot.status = match instance.update(&ctx, &def.settings) {
                                Ok(()) => instance.status(),
                                Err(e) => SourceStatus::Error(e.to_string()),
                            };
                        }
                    }
                }
                _ => {
                    let (instance, status) =
                        match self.registry.create(&def.kind, &ctx, &def.settings) {
                            Ok(instance) => {
                                let status = instance.status();
                                (Some(instance), status)
                            }
                            Err(crate::source::SourceError::UnknownKind(kind)) => (
                                None,
                                SourceStatus::Unavailable(format!(
                                    "source type '{kind}' is not installed"
                                )),
                            ),
                            Err(crate::source::SourceError::Unavailable(reason)) => {
                                (None, SourceStatus::Unavailable(reason))
                            }
                            Err(e) => (None, SourceStatus::Error(e.to_string())),
                        };
                    if !matches!(status, SourceStatus::Active) {
                        tracing::warn!(source = %def.name, ?status, "source not active");
                    }
                    self.sources.insert(
                        def.id,
                        SourceSlot {
                            kind: def.kind.clone(),
                            settings: def.settings.clone(),
                            instance,
                            status,
                        },
                    );
                }
            }
        }

        let mut old = std::mem::take(&mut self.canvases);
        for cg in &graph.canvases {
            let slot = match old.iter().position(|c| c.output.id == cg.id) {
                Some(i) => {
                    let mut slot = old.swap_remove(i);
                    if slot.output.width != cg.width || slot.output.height != cg.height {
                        slot = self.create_canvas(cg, now);
                    } else if slot.clock.rate() != cg.fps {
                        slot.clock = FrameClock::new(cg.fps, now);
                    }
                    slot.background = background(cg);
                    slot
                }
                None => self.create_canvas(cg, now),
            };
            let mut slot = slot;
            match (&cg.preview, &slot.preview) {
                (Some(_), None) => slot.preview = Some(self.create_preview(cg)),
                (None, Some(_)) => slot.preview = None,
                _ => {}
            }
            // Compile custom transition shaders before the first frame needs them.
            if let Some(t) = &cg.transition
                && let TransitionKind::Shader { code } = &t.kind
            {
                self.transitions.prepare_custom(&self.gpu, code);
            }
            self.canvases.push(slot);
        }
        self.filters.sync(&self.gpu, &graph.sources);
        self.graph = graph;
    }

    fn create_preview(&self, cg: &CanvasGraph) -> StudioPreview {
        let output = Arc::new(CanvasOutput::new(&self.gpu, cg.id, cg.width, cg.height));
        let canvas_views =
            std::array::from_fn(|i| output.slots[i].canvas.create_view(&Default::default()));
        let preview_views =
            std::array::from_fn(|i| estream_gpu::preview_target_view(&output.slots[i].preview));
        let inputs =
            std::array::from_fn(|i| self.converter.create_input(&self.gpu, &canvas_views[i]));
        StudioPreview {
            renderer: CanvasRenderer::new(&self.gpu, self.compositor.clone()),
            output,
            canvas_views,
            preview_views,
            inputs,
            sequence: 0,
            pending: None,
        }
    }

    fn create_canvas(&self, cg: &CanvasGraph, now: Timestamp) -> CanvasSlot {
        let output = Arc::new(CanvasOutput::new(&self.gpu, cg.id, cg.width, cg.height));
        let canvas_views =
            std::array::from_fn(|i| output.slots[i].canvas.create_view(&Default::default()));
        let preview_views =
            std::array::from_fn(|i| estream_gpu::preview_target_view(&output.slots[i].preview));
        let inputs =
            std::array::from_fn(|i| self.converter.create_input(&self.gpu, &canvas_views[i]));
        CanvasSlot {
            renderer: CanvasRenderer::new(&self.gpu, self.compositor.clone()),
            clock: FrameClock::new(cg.fps, now),
            output,
            canvas_views,
            preview_views,
            inputs,
            sequence: 0,
            background: background(cg),
            readback: None,
            from_renderer: None,
            transition_targets: None,
            preview: None,
        }
    }

    /// Earliest frame deadline across canvases.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.canvases.iter().map(|c| c.clock.next_deadline()).min()
    }

    /// Advances every source to `now`.
    pub fn tick_sources(&mut self, encoder: &mut wgpu::CommandEncoder, now: Timestamp) {
        for (id, slot) in &mut self.sources {
            if let Some(instance) = &mut slot.instance {
                self.progress.enter_source(*id);
                let ctx = SourceContext {
                    gpu: &self.gpu,
                    compositor: &self.compositor,
                    source: *id,
                };
                instance.tick(&ctx, encoder, now);
            }
        }
        // Video filters run on the fresh source frames.
        self.progress.enter(Stage::Filters);
        let Self {
            gpu,
            sources,
            filters,
            ..
        } = self;
        filters.run(gpu, encoder, now.0 as f32 / 1e9, |id| {
            let frame = sources.get(&id)?.instance.as_deref()?.frame()?;
            Some(FilterOutput {
                binding: frame.binding,
                width: frame.size.width.round().max(1.0) as u32,
                height: frame.size.height.round().max(1.0) as u32,
                texture: None,
            })
        });
    }

    /// Renders canvas `index` for the frame at `now` into its next output slot (and the
    /// studio Preview, if on). Returns the program slot used.
    fn render_canvas(
        &mut self,
        index: usize,
        encoder: &mut wgpu::CommandEncoder,
        now: Timestamp,
    ) -> usize {
        let Self {
            gpu,
            compositor,
            converter,
            sources,
            canvases,
            graph,
            scratch,
            scratch_extra,
            transitions,
            filters,
            ..
        } = self;
        let canvas = &mut canvases[index];
        let Some(cg) = graph.canvases.iter().find(|c| c.id == canvas.output.id) else {
            return 0;
        };
        let size = Vec2::new(cg.width as f32, cg.height as f32);
        let slot = ((canvas.sequence + 1) % OUTPUT_RING as u64) as usize;

        let mut items = reuse(scratch);
        let running = cg
            .transition
            .as_ref()
            .and_then(|t| t.progress(now).map(|p| (t, p)));
        // Stinger: the outgoing scene until the switch point, then the program scene, with
        // the stinger video stretched over the canvas on top. One normal render pass.
        let stinger = running.and_then(|(t, _)| t.overlay.map(|o| (t, o)));
        match stinger {
            Some((t, (overlay, point))) => {
                let before = now.0.saturating_sub(t.start.0) < point;
                push_items(
                    sources,
                    filters,
                    if before { &t.from } else { &cg.items },
                    &mut items,
                );
                if let Some((binding, _)) = source_frame(sources, filters, overlay) {
                    items.push(DrawItem {
                        binding,
                        matrix: glam::Affine2::from_scale(size),
                        uv_min: Vec2::ZERO,
                        uv_max: Vec2::ONE,
                        opacity: 1.0,
                        blend: estream_core::BlendMode::Normal,
                    });
                }
            }
            None => push_items(sources, filters, &cg.items, &mut items),
        }
        let running = if stinger.is_some() { None } else { running };
        match running {
            Some((t, progress)) => {
                // Outgoing scene → scratch A, program scene → scratch B, then blend.
                let targets = match &mut canvas.transition_targets {
                    Some(targets) if targets.width == cg.width && targets.height == cg.height => {
                        targets
                    }
                    other => other.insert(transitions.create_targets(gpu, cg.width, cg.height)),
                };
                let from_renderer = canvas
                    .from_renderer
                    .get_or_insert_with(|| CanvasRenderer::new(gpu, compositor.clone()));
                let mut from = reuse(scratch_extra);
                push_items(sources, filters, &t.from, &mut from);
                from_renderer.render(
                    gpu,
                    encoder,
                    &targets.from_view,
                    size,
                    canvas.background,
                    &from,
                );
                recycle(from, scratch_extra);
                canvas.renderer.render(
                    gpu,
                    encoder,
                    &targets.to_view,
                    size,
                    canvas.background,
                    &items,
                );
                let (shader, params) = transition_setup(&t.kind, progress);
                transitions.render(
                    gpu,
                    encoder,
                    targets,
                    &canvas.canvas_views[slot],
                    &shader,
                    &params,
                );
            }
            None => canvas.renderer.render(
                gpu,
                encoder,
                &canvas.canvas_views[slot],
                size,
                canvas.background,
                &items,
            ),
        }
        converter.to_srgb(encoder, &canvas.inputs[slot], &canvas.preview_views[slot]);
        recycle(items, scratch);

        // Studio Preview (UI only).
        if let (Some(preview), Some(flats)) = (&mut canvas.preview, &cg.preview) {
            let pslot = ((preview.sequence + 1) % OUTPUT_RING as u64) as usize;
            let mut items = reuse(scratch_extra);
            push_items(sources, filters, flats, &mut items);
            preview.renderer.render(
                gpu,
                encoder,
                &preview.canvas_views[pslot],
                size,
                canvas.background,
                &items,
            );
            recycle(items, scratch_extra);
            converter.to_srgb(
                encoder,
                &preview.inputs[pslot],
                &preview.preview_views[pslot],
            );
            preview.pending = Some(pslot);
        }
        slot
    }

    /// Renders every canvas once at `now` (ignoring frame clocks) and waits for the GPU.
    /// Used for headless rendering, tests and screenshots.
    pub fn render_now(&mut self, now: Timestamp) -> Vec<Arc<CanvasOutput>> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.tick_sources(&mut encoder, now);
        let slots: Vec<usize> = (0..self.canvases.len())
            .map(|i| self.render_canvas(i, &mut encoder, now))
            .collect();
        self.gpu.queue.submit([encoder.finish()]);
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
        for (canvas, slot) in self.canvases.iter_mut().zip(slots) {
            canvas.sequence += 1;
            canvas.output.publish(canvas.sequence, slot, now);
            canvas.output.stats.rendered.fetch_add(1, Ordering::Relaxed);
            if let Some(preview) = &mut canvas.preview {
                preview.publish(now);
            }
        }
        self.outputs()
    }

    /// Records NV12 conversion + readback when an encoder is attached to canvas `i`.
    fn record_readback(
        &mut self,
        i: usize,
        slot: usize,
        consumers: &[VideoConsumer],
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<usize> {
        let Self {
            gpu,
            converter,
            canvases,
            ..
        } = self;
        let canvas = &mut canvases[i];
        if !consumers.iter().any(|c| c.canvas == canvas.output.id) {
            canvas.readback = None;
            return None;
        }
        let (w, h) = (canvas.output.width, canvas.output.height);
        if canvas
            .readback
            .as_ref()
            .is_none_or(|r| r.width != w || r.height != h)
        {
            canvas.readback = ReadbackRing::new(gpu, w, h);
        }
        let ring = canvas.readback.as_mut()?;
        let index = ring.record(converter, &canvas.inputs[slot], encoder);
        if index.is_none() {
            canvas
                .output
                .stats
                .encoder_dropped
                .fetch_add(1, Ordering::Relaxed);
        }
        index
    }

    /// Renders all canvases whose deadline has passed. Returns how many rendered.
    fn render_due(&mut self, now: Timestamp, consumers: &[VideoConsumer]) -> usize {
        let started = Instant::now();
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.tick_sources(&mut encoder, now);
        self.progress.enter(Stage::Render);
        // Up to 64 canvases are tracked in fixed arrays to stay allocation-free.
        let mut ticks = [None::<(FrameTick, usize)>; 64];
        let mut maps = [None::<(usize, Timestamp)>; 64];
        let count = self.canvases.len().min(ticks.len());
        for (i, due) in ticks.iter_mut().enumerate().take(count) {
            if self.canvases[i].clock.next_deadline() <= now {
                let tick = self.canvases[i].clock.advance(now);
                let slot = self.render_canvas(i, &mut encoder, tick.timestamp);
                *due = Some((tick, slot));
                self.progress.enter(Stage::Convert);
                maps[i] = self
                    .record_readback(i, slot, consumers, &mut encoder)
                    .map(|idx| (idx, tick.timestamp));
                self.progress.enter(Stage::Render);
            }
        }
        self.progress.enter(Stage::Submit);
        self.gpu.queue.submit([encoder.finish()]);
        self.progress.enter(Stage::Poll);
        for (canvas, map) in self.canvases.iter_mut().zip(maps) {
            if let (Some(ring), Some((idx, ts))) = (&mut canvas.readback, map) {
                ring.request_map(idx, ts);
            }
        }
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        self.progress.enter(Stage::Deliver);
        for canvas in &mut self.canvases {
            if let Some(ring) = &mut canvas.readback {
                ring.deliver(
                    consumers,
                    canvas.output.id,
                    &canvas.output.stats.encoder_dropped,
                );
            }
        }
        let elapsed = started.elapsed().as_nanos() as u64;
        self.progress.end_frame();
        let mut rendered = 0;
        for (canvas, due) in self.canvases.iter_mut().zip(ticks) {
            if let Some((tick, slot)) = due {
                rendered += 1;
                canvas.sequence += 1;
                canvas.output.publish(canvas.sequence, slot, tick.timestamp);
                if let Some(preview) = &mut canvas.preview {
                    preview.publish(tick.timestamp);
                }
                let stats = &canvas.output.stats;
                stats.rendered.fetch_add(1, Ordering::Relaxed);
                stats.lagged.fetch_add(tick.skipped, Ordering::Relaxed);
                stats.last_render_ns.store(elapsed, Ordering::Relaxed);
                if tick.skipped > 0 {
                    tracing::debug!(canvas = ?canvas.output.id, skipped = tick.skipped, "frames lagged");
                }
            }
        }
        rendered
    }
}

fn background(cg: &CanvasGraph) -> wgpu::Color {
    let c = cg.background;
    let lin = |v: u8| srgb_to_linear(v as f32 / 255.0) as f64;
    let a = c.a as f64 / 255.0;
    // Premultiplied, like the rest of the canvas.
    wgpu::Color {
        r: lin(c.r) * a,
        g: lin(c.g) * a,
        b: lin(c.b) * a,
        a,
    }
}

/// Callbacks from the video thread to the engine's shared state.
pub(crate) trait VideoSink: Send + Sync + 'static {
    fn graph(&self) -> Arc<RenderGraph>;
    fn publish(&self, outputs: Vec<Arc<CanvasOutput>>, statuses: HashMap<SourceId, SourceInfo>);
    fn publish_previews(&self, previews: Vec<Arc<CanvasOutput>>);
    fn shutdown_requested(&self) -> &AtomicBool;
    fn consumers(&self) -> Arc<Vec<VideoConsumer>>;
}

/// Main loop of the video thread.
pub(crate) fn run(mut core: VideoCore, clock: EngineClock, sink: Arc<dyn VideoSink>) {
    let mut revision = u64::MAX;
    let mut last_statuses = HashMap::new();
    let mut statuses_checked = Instant::now();
    let progress = core.progress();
    let _finished = FinishGuard(progress.clone());
    while !sink.shutdown_requested().load(Ordering::Acquire) {
        let graph = sink.graph();
        let changed = graph.revision != revision;
        if changed {
            progress.enter(Stage::Sync);
            revision = graph.revision;
            core.sync(graph, clock.now());
            sink.publish_previews(core.preview_outputs());
        }
        // Sources change state on their own (devices connect, windows close,
        // frame sizes change), so re-check a few times per second.
        if changed || statuses_checked.elapsed() >= Duration::from_millis(250) {
            statuses_checked = Instant::now();
            progress.enter(Stage::Publish);
            let statuses = core.statuses();
            if changed || statuses != last_statuses {
                sink.publish(core.outputs(), statuses.clone());
                last_statuses = statuses;
            }
        }
        let Some(deadline) = core.next_deadline() else {
            progress.enter(Stage::Idle);
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        progress.enter(Stage::Wait);
        wait_until(&clock, deadline);
        core.render_due(clock.now(), &sink.consumers());
    }
    // Let in-flight GPU work finish before resources drop.
    progress.enter(Stage::Shutdown);
    let _ = core.gpu.device.poll(wgpu::PollType::wait_indefinitely());
}
