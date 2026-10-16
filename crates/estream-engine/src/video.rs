//! The video pipeline: keeps runtime sources in sync with the render graph,
//! renders each canvas on its own frame clock, and publishes the results.

use crate::frame_clock::{FrameClock, FrameTick, wait_until};
use crate::graph::{CanvasGraph, RenderGraph};
use crate::source::{SourceContext, SourceRegistry, SourceStatus, VideoSource};
use estream_core::color::srgb_to_linear;
use estream_core::{CanvasId, EngineClock, SourceId, Timestamp};
use estream_gpu::{
    CANVAS_FORMAT, CanvasRenderer, Compositor, ConvertInput, DrawItem, GpuContext, OutputConverter,
    wgpu,
};
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
}

impl VideoCore {
    pub fn new(gpu: GpuContext, registry: SourceRegistry) -> Self {
        Self {
            compositor: Compositor::new(&gpu),
            converter: OutputConverter::new(&gpu),
            gpu,
            registry,
            sources: HashMap::new(),
            canvases: Vec::new(),
            graph: Arc::new(RenderGraph::default()),
            scratch: Vec::new(),
        }
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
        let ctx = SourceContext {
            gpu: &self.gpu,
            compositor: &self.compositor,
        };

        self.sources
            .retain(|id, _| graph.sources.iter().any(|s| s.id == *id));
        for def in &graph.sources {
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
            self.canvases.push(slot);
        }
        self.graph = graph;
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
        }
    }

    /// Earliest frame deadline across canvases.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.canvases.iter().map(|c| c.clock.next_deadline()).min()
    }

    /// Advances every source to `now`.
    pub fn tick_sources(&mut self, encoder: &mut wgpu::CommandEncoder, now: Timestamp) {
        let ctx = SourceContext {
            gpu: &self.gpu,
            compositor: &self.compositor,
        };
        for slot in self.sources.values_mut() {
            if let Some(instance) = &mut slot.instance {
                instance.tick(&ctx, encoder, now);
            }
        }
    }

    /// Renders canvas `index` into its next output slot. Returns the slot used.
    fn render_canvas(&mut self, index: usize, encoder: &mut wgpu::CommandEncoder) -> usize {
        let Self {
            gpu,
            converter,
            sources,
            canvases,
            graph,
            scratch,
            ..
        } = self;
        let canvas = &mut canvases[index];
        let Some(cg) = graph.canvases.iter().find(|c| c.id == canvas.output.id) else {
            return 0;
        };

        // Reuse the scratch allocation across frames (in-place collect keeps capacity).
        let mut items: Vec<DrawItem<'_>> = std::mem::take(scratch)
            .into_iter()
            .map(|_| -> DrawItem<'_> { unreachable!("scratch is always empty") })
            .collect();
        for flat in &cg.items {
            let Some(frame) = sources
                .get(&flat.source)
                .and_then(|s| s.instance.as_deref())
                .and_then(|s| s.frame())
            else {
                continue; // no frame yet: draws nothing
            };
            let layout = flat.transform.layout(flat.crop, frame.size);
            if layout.size.x <= 0.0 || layout.size.y <= 0.0 {
                continue;
            }
            items.push(DrawItem {
                binding: frame.binding,
                matrix: flat.parent * layout.matrix,
                uv_min: layout.uv_min,
                uv_max: layout.uv_max,
                opacity: flat.opacity,
                blend: flat.blend,
            });
        }

        let slot = ((canvas.sequence + 1) % OUTPUT_RING as u64) as usize;
        canvas.renderer.render(
            gpu,
            encoder,
            &canvas.canvas_views[slot],
            Vec2::new(cg.width as f32, cg.height as f32),
            canvas.background,
            &items,
        );
        converter.to_srgb(encoder, &canvas.inputs[slot], &canvas.preview_views[slot]);

        items.clear();
        *scratch = items
            .into_iter()
            .map(|_| -> DrawItem<'static> { unreachable!("cleared above") })
            .collect();
        slot
    }

    /// Renders every canvas once at `now` (ignoring frame clocks) and waits for the GPU.
    /// Used for headless rendering, tests and screenshots.
    pub fn render_now(&mut self, now: Timestamp) -> Vec<Arc<CanvasOutput>> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.tick_sources(&mut encoder, now);
        let slots: Vec<usize> = (0..self.canvases.len())
            .map(|i| self.render_canvas(i, &mut encoder))
            .collect();
        self.gpu.queue.submit([encoder.finish()]);
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
        for (canvas, slot) in self.canvases.iter_mut().zip(slots) {
            canvas.sequence += 1;
            canvas.output.publish(canvas.sequence, slot, now);
            canvas.output.stats.rendered.fetch_add(1, Ordering::Relaxed);
        }
        self.outputs()
    }

    /// Renders all canvases whose deadline has passed. Returns how many rendered.
    fn render_due(&mut self, now: Timestamp) -> usize {
        let started = Instant::now();
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.tick_sources(&mut encoder, now);
        // Up to 64 canvases are tracked in a fixed array to stay allocation-free.
        let mut ticks = [None::<(FrameTick, usize)>; 64];
        let count = self.canvases.len().min(ticks.len());
        for (i, due) in ticks.iter_mut().enumerate().take(count) {
            if self.canvases[i].clock.next_deadline() <= now {
                let tick = self.canvases[i].clock.advance(now);
                *due = Some((tick, self.render_canvas(i, &mut encoder)));
            }
        }
        self.gpu.queue.submit([encoder.finish()]);
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        let elapsed = started.elapsed().as_nanos() as u64;
        let mut rendered = 0;
        for (canvas, due) in self.canvases.iter_mut().zip(ticks) {
            if let Some((tick, slot)) = due {
                rendered += 1;
                canvas.sequence += 1;
                canvas.output.publish(canvas.sequence, slot, tick.timestamp);
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
    fn shutdown_requested(&self) -> &AtomicBool;
}

/// Main loop of the video thread.
pub(crate) fn run(mut core: VideoCore, clock: EngineClock, sink: Arc<dyn VideoSink>) {
    let mut revision = u64::MAX;
    let mut last_statuses = HashMap::new();
    let mut statuses_checked = Instant::now();
    while !sink.shutdown_requested().load(Ordering::Acquire) {
        let graph = sink.graph();
        let changed = graph.revision != revision;
        if changed {
            revision = graph.revision;
            core.sync(graph, clock.now());
        }
        // Sources change state on their own (devices connect, windows close,
        // frame sizes change), so re-check a few times per second.
        if changed || statuses_checked.elapsed() >= Duration::from_millis(250) {
            statuses_checked = Instant::now();
            let statuses = core.statuses();
            if changed || statuses != last_statuses {
                sink.publish(core.outputs(), statuses.clone());
                last_statuses = statuses;
            }
        }
        let Some(deadline) = core.next_deadline() else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        wait_until(&clock, deadline);
        core.render_due(clock.now());
    }
    // Let in-flight GPU work finish before resources drop.
    let _ = core.gpu.device.poll(wgpu::PollType::wait_indefinitely());
}
