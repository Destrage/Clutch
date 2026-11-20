//! GPU state of item masks, kept per scene item: the mask texture and the effect layers.
//!
//! Baking runs on a worker thread. The video thread sends a job when a mask or an item's size on
//! the canvas changes, uploads finished images, and meanwhile keeps drawing the previous textures
//! (they cover the item's unit square, so they stay in place, only a little soft or sharp).
//! A newly masked item is left out until its first mask is ready.

use crate::graph::{FlatItem, RenderGraph};
use crate::mask::{Baked, Image, Layer, bake};
use crossbeam_channel::{Receiver, Sender};
use estream_core::ItemId;
use estream_gpu::{Compositor, GpuContext, upload_texture, wgpu};
use estream_project::ItemMask;
use glam::Vec2;
use std::collections::HashMap;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) struct Tex {
    _texture: wgpu::Texture,
    pub binding: wgpu::BindGroup,
}

/// What an item draws with: its mask, and effect layers grown by `pad` (item units) per side.
pub(crate) struct Drawn {
    pub mask: Option<Tex>,
    pub under: Option<(Tex, Vec2)>,
    pub over: Option<(Tex, Vec2)>,
    /// The shape's distance field, for morphs (`None` when the mask doesn't cut the item).
    pub field: Option<Tex>,
}

struct Job {
    item: ItemId,
    generation: u64,
    mask: ItemMask,
    size: Vec2,
}

struct Done {
    item: ItemId,
    generation: u64,
    baked: Baked,
}

struct Slot {
    mask: ItemMask,
    /// The last job sent: its generation and the item size it was for.
    requested: Option<(u64, [u32; 2])>,
    /// Generation of the textures in `drawn`.
    shown: u64,
    drawn: Option<Drawn>,
    used: bool,
}

pub(crate) struct Masks {
    slots: HashMap<ItemId, Slot>,
    jobs: Option<Sender<Job>>,
    done: Receiver<Done>,
    worker: Option<JoinHandle<()>>,
    generation: u64,
}

impl Default for Masks {
    fn default() -> Self {
        let (jobs, inbox) = crossbeam_channel::unbounded::<Job>();
        let (outbox, done) = crossbeam_channel::unbounded();
        let worker = std::thread::Builder::new()
            .name("clutch-masks".into())
            .spawn(move || {
                while let Ok(first) = inbox.recv() {
                    // Only the newest job per item matters (a drag sends one per frame).
                    let mut latest: Vec<Job> = vec![first];
                    for job in inbox.try_iter() {
                        latest.retain(|j| j.item != job.item);
                        latest.push(job);
                    }
                    for job in latest {
                        let baked = bake(&job.mask, job.size);
                        let done = Done {
                            item: job.item,
                            generation: job.generation,
                            baked,
                        };
                        if outbox.send(done).is_err() {
                            return;
                        }
                    }
                }
            })
            .map_err(|e| tracing::error!(error = %e, "mask worker didn't start; masks are off"))
            .ok();
        Self {
            slots: HashMap::new(),
            jobs: worker.is_some().then_some(jobs),
            done,
            worker,
            generation: 0,
        }
    }
}

impl Drop for Masks {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Masks {
    /// Matches the slots to the masked items in `graph` (program, Preview and transition scenes).
    pub fn sync(&mut self, graph: &RenderGraph) {
        for slot in self.slots.values_mut() {
            slot.used = false;
        }
        for flat in flats(graph) {
            let Some(mask) = &flat.mask else { continue };
            let slot = self.slots.entry(flat.item).or_insert_with(|| Slot {
                mask: mask.clone(),
                requested: None,
                shown: 0,
                drawn: None,
                used: true,
            });
            slot.used = true;
            if slot.mask != *mask {
                slot.mask = mask.clone();
                slot.requested = None;
            }
        }
        self.slots.retain(|_, s| s.used);
    }

    /// Uploads finished bakes and asks for new ones where an item's size on the canvas changed.
    /// `size_of` gives an item's size in canvas pixels (`None` while its source has no frame).
    pub fn update(
        &mut self,
        gpu: &GpuContext,
        compositor: &Compositor,
        graph: &RenderGraph,
        size_of: impl Fn(&FlatItem) -> Option<Vec2>,
    ) {
        self.receive(gpu, compositor, None);
        if self.slots.is_empty() {
            return;
        }
        let Some(jobs) = &self.jobs else { return };
        for flat in flats(graph) {
            let Some(slot) = self.slots.get_mut(&flat.item) else {
                continue;
            };
            let Some(size) = size_of(flat).filter(|s| s.is_finite() && s.min_element() >= 1.0)
            else {
                continue;
            };
            let key = [size.x.round() as u32, size.y.round() as u32];
            if slot.requested.is_some_and(|(_, k)| k == key) {
                continue;
            }
            self.generation += 1;
            let job = Job {
                item: flat.item,
                generation: self.generation,
                mask: slot.mask.clone(),
                size: Vec2::new(key[0] as f32, key[1] as f32),
            };
            if jobs.send(job).is_ok() {
                slot.requested = Some((self.generation, key));
            }
        }
    }

    /// Waits up to `timeout` for every asked-for bake (headless rendering and tests).
    pub fn settle(&mut self, gpu: &GpuContext, compositor: &Compositor, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.waiting() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !self.receive(gpu, compositor, Some(left)) {
                break;
            }
        }
    }

    /// What `item` draws with; `None` while its first mask is still baking.
    pub fn get(&self, item: ItemId) -> Option<&Drawn> {
        self.slots.get(&item)?.drawn.as_ref()
    }

    fn waiting(&self) -> bool {
        self.slots
            .values()
            .any(|s| s.requested.is_some_and(|(g, _)| g > s.shown))
    }

    /// Uploads finished bakes; with a `wait`, blocks up to that long for the first one.
    /// Returns whether anything arrived.
    fn receive(
        &mut self,
        gpu: &GpuContext,
        compositor: &Compositor,
        wait: Option<Duration>,
    ) -> bool {
        let mut first = wait.and_then(|w| self.done.recv_timeout(w).ok());
        let mut any = false;
        while let Some(done) = first.take().or_else(|| self.done.try_recv().ok()) {
            any = true;
            let Some(slot) = self.slots.get_mut(&done.item) else {
                continue;
            };
            if done.generation <= slot.shown {
                continue;
            }
            slot.shown = done.generation;
            let tex = |image: &Image, format| {
                let texture = upload_texture(
                    gpu,
                    "item mask",
                    image.width,
                    image.height,
                    format,
                    &image.pixels,
                );
                let binding = compositor
                    .create_source_binding(gpu, &texture.create_view(&Default::default()));
                Tex {
                    _texture: texture,
                    binding,
                }
            };
            let layer = |layer: &Layer| {
                (
                    tex(&layer.image, wgpu::TextureFormat::Rgba8UnormSrgb),
                    layer.pad,
                )
            };
            let baked = &done.baked;
            slot.drawn = Some(Drawn {
                mask: baked
                    .mask
                    .as_ref()
                    .map(|m| tex(m, wgpu::TextureFormat::R8Unorm)),
                under: baked.under.as_ref().map(layer),
                over: baked.over.as_ref().map(layer),
                field: baked
                    .field
                    .as_ref()
                    .map(|f| tex(f, wgpu::TextureFormat::R16Float)),
            });
        }
        any
    }
}

/// Every item list of every canvas: program, Preview, the scene a transition leaves, and the
/// masked items of other scenes (baked ahead).
fn flats(graph: &RenderGraph) -> impl Iterator<Item = &FlatItem> {
    graph.canvases.iter().flat_map(|cg| {
        [Some(&cg.items), cg.preview.as_ref()]
            .into_iter()
            .flatten()
            .chain(cg.transition.as_ref().map(|t| &t.from))
            .chain(std::iter::once(&cg.standby))
            .flatten()
    })
}
