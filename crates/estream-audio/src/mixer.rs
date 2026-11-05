//! The mixer: owns audio source instances and turns them into six track mixes per canvas,
//! a monitor mix and meters, one 10 ms block at a time. Each source is read and filtered
//! once, then added to the mix of every canvas that shows it.

use crate::block::Block;
use crate::filters::{self, AudioFilter};
use crate::meter::Meter;
use crate::source::AudioSource;
use crate::{BLOCK, SAMPLE_RATE, TRACKS, db};
use estream_core::{CanvasId, SourceId, SourceStatus, Timestamp};
use estream_project::{AudioConfig, Monitoring};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// One source to mix.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub source: SourceId,
    pub kind: String,
    pub settings: Value,
    pub config: AudioConfig,
    /// Canvases whose mix includes this source (empty = every mix).
    pub canvases: Vec<CanvasId>,
}

/// The set of sources to mix, rebuilt by the engine whenever the project changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioGraph {
    pub nodes: Vec<AudioNode>,
    pub monitor_device: String,
    /// One mix per canvas, in this order (empty = a single mix of every node).
    pub canvases: Vec<CanvasId>,
}

/// At most this many canvas mixes (bits of a channel's bus mask).
const MAX_BUSES: usize = 64;

/// Creates the audio part of a source, or `None` if the kind has no audio.
pub type SourceFactory =
    Arc<dyn Fn(&str, &Value) -> Option<Result<Box<dyn AudioSource>, String>> + Send + Sync>;

/// Receives the six track mixes every block (recording, streaming).
pub trait TrackSink: Send {
    fn push(&mut self, timestamp: Timestamp, tracks: &[Block; TRACKS]);

    /// Closed sinks are removed by the mixer.
    fn is_closed(&self) -> bool {
        false
    }
}

/// Receives the monitor mix every block.
pub trait MonitorSink: Send {
    fn push(&mut self, block: &Block);
}

struct FilterSlot {
    id: String,
    kind: String,
    settings: Value,
    enabled: bool,
    filter: Box<dyn AudioFilter>,
}

/// Fixed delay for sync offsets (allocated only when the offset changes).
struct Delay {
    buffer: Vec<[f32; 2]>,
    pos: usize,
}

impl Delay {
    fn new(ms: u32) -> Option<Self> {
        let frames = (ms.min(2000) as usize * SAMPLE_RATE as usize) / 1000;
        (frames > 0).then(|| Self {
            buffer: vec![[0.0; 2]; frames],
            pos: 0,
        })
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let out = self.buffer[self.pos];
            self.buffer[self.pos] = [block.l[i], block.r[i]];
            self.pos = (self.pos + 1) % self.buffer.len();
            block.l[i] = out[0];
            block.r[i] = out[1];
        }
    }
}

struct Channel {
    kind: String,
    settings: Value,
    config: AudioConfig,
    instance: Option<Box<dyn AudioSource>>,
    status: SourceStatus,
    filters: Vec<FilterSlot>,
    delay: Option<Delay>,
    gain: f32,
    meter: Arc<Meter>,
    block: Block,
    /// Canvases from the graph (empty = every mix) and the matching bus bits.
    canvases: Vec<CanvasId>,
    buses: u64,
}

/// One canvas's six track mixes.
struct Bus {
    /// `None`: the single mix of a graph without canvases.
    canvas: Option<CanvasId>,
    tracks: [Block; TRACKS],
    master: Arc<Meter>,
}

impl Bus {
    fn new(canvas: Option<CanvasId>) -> Self {
        Self {
            canvas,
            tracks: std::array::from_fn(|_| Block::SILENT),
            master: Arc::new(Meter::default()),
        }
    }
}

/// Meters for the current sources and the master mix (track 1) of each canvas.
#[derive(Clone, Debug, Default)]
pub struct MeterSet {
    pub sources: Vec<(SourceId, Arc<Meter>)>,
    /// Track 1 of the first canvas's mix.
    pub master: Arc<Meter>,
    /// Track 1 of each canvas's mix.
    pub masters: Vec<(CanvasId, Arc<Meter>)>,
    /// The canvases each source is heard on (absent or empty = every canvas).
    pub canvases: HashMap<SourceId, Vec<CanvasId>>,
}

impl MeterSet {
    /// Whether `source` is in the mix of `canvas`.
    pub fn heard_on(&self, source: SourceId, canvas: CanvasId) -> bool {
        self.canvases
            .get(&source)
            .is_none_or(|c| c.is_empty() || c.contains(&canvas))
    }

    /// Track 1 of `canvas`'s mix (the first mix if it has none of its own).
    pub fn master_of(&self, canvas: CanvasId) -> &Arc<Meter> {
        self.masters
            .iter()
            .find(|(c, _)| *c == canvas)
            .map_or(&self.master, |(_, m)| m)
    }
}

pub struct AudioCore {
    factory: SourceFactory,
    channels: HashMap<SourceId, Channel>,
    order: Vec<SourceId>,
    /// Never empty.
    buses: Vec<Bus>,
    monitor: Block,
    /// Each sink takes the mix of its canvas (`None` = the first mix).
    sinks: Vec<(Option<CanvasId>, Box<dyn TrackSink>)>,
    monitor_sink: Option<Box<dyn MonitorSink>>,
    /// Blocks processed (for timing tests and stats).
    pub blocks: u64,
}

impl AudioCore {
    pub fn new(factory: SourceFactory) -> Self {
        Self {
            factory,
            channels: HashMap::new(),
            order: Vec::new(),
            buses: vec![Bus::new(None)],
            monitor: Block::SILENT,
            sinks: Vec::new(),
            monitor_sink: None,
            blocks: 0,
        }
    }

    /// Adds a sink for the mix of `canvas` (`None` = the first mix). A canvas without a mix
    /// gets silence.
    pub fn add_sink(&mut self, canvas: Option<CanvasId>, sink: Box<dyn TrackSink>) {
        self.sinks.push((canvas, sink));
    }

    pub fn set_monitor_sink(&mut self, sink: Option<Box<dyn MonitorSink>>) {
        self.monitor_sink = sink;
    }

    /// Reconciles sources with the graph. Allocates only for changes.
    pub fn sync(&mut self, graph: &AudioGraph) {
        self.sync_buses(&graph.canvases);
        self.channels
            .retain(|id, _| graph.nodes.iter().any(|n| n.source == *id));
        self.order.clear();
        for node in &graph.nodes {
            let rebuild = self
                .channels
                .get(&node.source)
                .is_none_or(|c| c.kind != node.kind);
            if rebuild {
                let (instance, status) = match (self.factory)(&node.kind, &node.settings) {
                    None => continue, // not an audio source
                    Some(Ok(instance)) => {
                        let status = instance.status();
                        (Some(instance), status)
                    }
                    Some(Err(e)) => (None, SourceStatus::Error(e)),
                };
                self.channels.insert(
                    node.source,
                    Channel {
                        kind: node.kind.clone(),
                        settings: node.settings.clone(),
                        config: AudioConfig {
                            sync_offset_ms: u32::MAX, // force delay setup below
                            ..node.config.clone()
                        },
                        instance,
                        status,
                        filters: Vec::new(),
                        delay: None,
                        gain: if node.config.muted {
                            0.0
                        } else {
                            db::to_gain(node.config.volume_db)
                        },
                        meter: Arc::new(Meter::default()),
                        block: Block::SILENT,
                        canvases: Vec::new(),
                        buses: 0,
                    },
                );
            }
            let Some(ch) = self.channels.get_mut(&node.source) else {
                continue;
            };
            if ch.settings != node.settings {
                ch.settings = node.settings.clone();
                if let Some(instance) = &mut ch.instance
                    && let Err(e) = instance.update(&node.settings)
                {
                    ch.status = SourceStatus::Error(e);
                }
            }
            if ch.config.sync_offset_ms != node.config.sync_offset_ms {
                ch.delay = Delay::new(node.config.sync_offset_ms);
            }
            sync_filters(&mut ch.filters, &node.config);
            ch.config = node.config.clone();
            if ch.canvases != node.canvases {
                ch.canvases = node.canvases.clone();
            }
            ch.buses = bus_mask(&self.buses, &ch.canvases);
            self.order.push(node.source);
        }
    }

    /// One bus per graph canvas (or a single one), keeping the meters of canvases that stay.
    fn sync_buses(&mut self, canvases: &[CanvasId]) {
        let wanted: Vec<Option<CanvasId>> = if canvases.is_empty() {
            vec![None]
        } else {
            canvases.iter().take(MAX_BUSES).copied().map(Some).collect()
        };
        if self
            .buses
            .iter()
            .map(|b| b.canvas)
            .eq(wanted.iter().copied())
        {
            return;
        }
        let mut old = std::mem::take(&mut self.buses);
        self.buses = wanted
            .into_iter()
            .map(|canvas| match old.iter().position(|b| b.canvas == canvas) {
                Some(i) => old.swap_remove(i),
                None => Bus::new(canvas),
            })
            .collect();
    }

    /// Processes one block at `now`.
    pub fn process(&mut self, now: Timestamp) {
        for bus in &mut self.buses {
            for t in &mut bus.tracks {
                t.clear();
            }
        }
        self.monitor.clear();
        for id in &self.order {
            let Some(ch) = self.channels.get_mut(id) else {
                continue;
            };
            ch.block.clear();
            if let Some(instance) = &mut ch.instance {
                instance.read(&mut ch.block, now);
            }
            if let Some(delay) = &mut ch.delay {
                delay.process(&mut ch.block);
            }
            for f in ch.filters.iter_mut().filter(|f| f.enabled) {
                f.filter.process(&mut ch.block);
            }
            let cfg = &ch.config;
            if cfg.mono {
                for i in 0..BLOCK {
                    let m = (ch.block.l[i] + ch.block.r[i]) * 0.5;
                    ch.block.l[i] = m;
                    ch.block.r[i] = m;
                }
            }
            // Balance: linear pan, centre = unity on both sides.
            let bal = cfg.balance.clamp(0.0, 1.0);
            let (bl, br) = ((2.0 * (1.0 - bal)).min(1.0), (2.0 * bal).min(1.0));
            // Smooth volume changes across the block to avoid zipper noise.
            let target = if cfg.muted {
                0.0
            } else {
                db::to_gain(cfg.volume_db)
            };
            let start = ch.gain;
            for i in 0..BLOCK {
                let g = start + (target - start) * ((i + 1) as f32 / BLOCK as f32);
                ch.block.l[i] *= g * bl;
                ch.block.r[i] *= g * br;
            }
            ch.gain = target;
            ch.meter.store(ch.block.peak(), ch.block.rms());

            if cfg.monitoring != Monitoring::Off {
                self.monitor.add(&ch.block);
            }
            if cfg.monitoring != Monitoring::MonitorOnly {
                for (b, bus) in self.buses.iter_mut().enumerate() {
                    if ch.buses & (1 << b) == 0 {
                        continue;
                    }
                    for (t, track) in bus.tracks.iter_mut().enumerate() {
                        if cfg.tracks & (1 << t) != 0 {
                            track.add(&ch.block);
                        }
                    }
                }
            }
        }
        for bus in &self.buses {
            bus.master.store(bus.tracks[0].peak(), bus.tracks[0].rms());
        }
        self.sinks.retain(|(_, s)| !s.is_closed());
        for (canvas, sink) in &mut self.sinks {
            let tracks = match canvas {
                None => &self.buses[0].tracks,
                Some(c) => self
                    .buses
                    .iter()
                    .find(|b| b.canvas == Some(*c))
                    .map_or(&SILENT_TRACKS, |b| &b.tracks),
            };
            sink.push(now, tracks);
        }
        if let Some(m) = &mut self.monitor_sink {
            m.push(&self.monitor);
        }
        self.blocks += 1;
    }

    pub fn meters(&self) -> MeterSet {
        let channels = || {
            self.order
                .iter()
                .filter_map(|id| self.channels.get(id).map(|c| (*id, c)))
        };
        MeterSet {
            sources: channels().map(|(id, c)| (id, c.meter.clone())).collect(),
            master: self.buses[0].master.clone(),
            masters: self
                .buses
                .iter()
                .filter_map(|b| b.canvas.map(|c| (c, b.master.clone())))
                .collect(),
            canvases: channels()
                .filter(|(_, c)| !c.canvases.is_empty())
                .map(|(id, c)| (id, c.canvases.clone()))
                .collect(),
        }
    }

    pub fn statuses(&self) -> HashMap<SourceId, SourceStatus> {
        self.channels
            .iter()
            .map(|(id, c)| {
                let status = match (&c.status, &c.instance) {
                    (SourceStatus::Error(_), _) | (_, None) => c.status.clone(),
                    (_, Some(i)) => i.status(),
                };
                (*id, status)
            })
            .collect()
    }

    /// Track mixes of the last processed block (the first canvas's mix).
    pub fn tracks(&self) -> &[Block; TRACKS] {
        &self.buses[0].tracks
    }

    /// Track mixes of `canvas` in the last processed block.
    pub fn tracks_of(&self, canvas: CanvasId) -> Option<&[Block; TRACKS]> {
        self.buses
            .iter()
            .find(|b| b.canvas == Some(canvas))
            .map(|b| &b.tracks)
    }
}

static SILENT_TRACKS: [Block; TRACKS] = [Block::SILENT; TRACKS];

/// Bus bits for a source heard on `canvases` (empty = every bus).
fn bus_mask(buses: &[Bus], canvases: &[CanvasId]) -> u64 {
    buses
        .iter()
        .enumerate()
        .filter(|(_, b)| canvases.is_empty() || b.canvas.is_some_and(|c| canvases.contains(&c)))
        .fold(0, |mask, (i, _)| mask | 1 << i)
}

/// Keeps filter instances (and their state) when only settings change.
fn sync_filters(slots: &mut Vec<FilterSlot>, config: &AudioConfig) {
    let unchanged = slots.len() == config.filters.len()
        && slots.iter().zip(&config.filters).all(|(s, d)| {
            s.id == d.id && s.kind == d.kind && s.settings == d.settings && s.enabled == d.enabled
        });
    if unchanged {
        return;
    }
    let mut old: Vec<FilterSlot> = std::mem::take(slots);
    for def in &config.filters {
        let reused = old
            .iter()
            .position(|s| s.id == def.id && s.kind == def.kind)
            .map(|i| old.swap_remove(i));
        let slot = match reused {
            Some(mut s) => {
                if s.settings != def.settings {
                    s.filter.update(&def.settings);
                    s.settings = def.settings.clone();
                }
                s.enabled = def.enabled;
                s
            }
            None => match filters::create(&def.kind, &def.settings) {
                Some(filter) => FilterSlot {
                    id: def.id.clone(),
                    kind: def.kind.clone(),
                    settings: def.settings.clone(),
                    enabled: def.enabled,
                    filter,
                },
                None => continue,
            },
        };
        slots.push(slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::ToneSource;
    use serde_json::json;
    use std::sync::Mutex;

    fn factory() -> SourceFactory {
        Arc::new(|kind, settings| match kind {
            "tone" => {
                let mut t = ToneSource::default();
                Some(
                    t.update(settings)
                        .map(|_| Box::new(t) as Box<dyn AudioSource>),
                )
            }
            _ => None,
        })
    }

    fn node(level_db: f64, config: AudioConfig) -> AudioNode {
        AudioNode {
            source: SourceId::new(),
            kind: "tone".into(),
            settings: json!({ "frequency": 1000.0, "level_db": level_db }),
            config,
            canvases: Vec::new(),
        }
    }

    struct Capture(Arc<Mutex<Vec<[f32; TRACKS]>>>);
    impl TrackSink for Capture {
        fn push(&mut self, _: Timestamp, tracks: &[Block; TRACKS]) {
            let peaks = std::array::from_fn(|t| tracks[t].peak()[0]);
            if let Ok(mut v) = self.0.lock() {
                v.push(peaks);
            }
        }
    }

    fn run(core: &mut AudioCore, blocks: usize) {
        for _ in 0..blocks {
            core.process(Timestamp::ZERO);
        }
    }

    #[test]
    fn volume_in_db_shows_on_meter() {
        let mut core = AudioCore::new(factory());
        let n = node(
            0.0,
            AudioConfig {
                volume_db: -6.0,
                ..Default::default()
            },
        );
        core.sync(&AudioGraph {
            nodes: vec![n],
            ..Default::default()
        });
        run(&mut core, 5);
        let reading = core.meters().sources[0].1.read();
        let peak_db = db::from_gain(reading.peak[0]);
        assert!((peak_db + 6.0).abs() < 0.1, "{peak_db}");
    }

    #[test]
    fn mute_and_routing_and_monitor_only() {
        let peaks = Arc::new(Mutex::new(Vec::new()));
        let mut core = AudioCore::new(factory());
        core.add_sink(None, Box::new(Capture(peaks.clone())));
        let routed = node(
            -6.0,
            AudioConfig {
                tracks: 0b101,
                ..Default::default()
            },
        ); // tracks 1 & 3
        let muted = node(
            0.0,
            AudioConfig {
                muted: true,
                ..Default::default()
            },
        );
        let monitor_only = node(
            0.0,
            AudioConfig {
                monitoring: Monitoring::MonitorOnly,
                ..Default::default()
            },
        );
        core.sync(&AudioGraph {
            nodes: vec![routed, muted.clone(), monitor_only],
            ..Default::default()
        });
        run(&mut core, 5);
        let last = *peaks.lock().unwrap().last().unwrap();
        let expected = db::to_gain(-6.0);
        assert!((last[0] - expected).abs() < 0.01, "track 1: {}", last[0]);
        assert!(last[1] == 0.0, "track 2 empty");
        assert!((last[2] - expected).abs() < 0.01, "track 3");
        assert!(last[3..].iter().all(|p| *p == 0.0));
        // Muted source meters silence.
        let muted_meter = core
            .meters()
            .sources
            .iter()
            .find(|(id, _)| *id == muted.source)
            .unwrap()
            .1
            .read();
        assert_eq!(muted_meter.peak, [0.0, 0.0]);
        // Monitor-only is in the monitor mix but not in any track (checked above: only -6 dB present).
        assert!(core.monitor.peak()[0] > 0.9);
    }

    #[test]
    fn removed_source_stops_contributing() {
        let peaks = Arc::new(Mutex::new(Vec::new()));
        let mut core = AudioCore::new(factory());
        core.add_sink(None, Box::new(Capture(peaks.clone())));
        let music = node(0.0, AudioConfig::default());
        core.sync(&AudioGraph {
            nodes: vec![music],
            ..Default::default()
        });
        run(&mut core, 2);
        core.sync(&AudioGraph::default());
        run(&mut core, 2);
        assert_eq!(peaks.lock().unwrap().last().unwrap()[0], 0.0);
    }

    #[test]
    fn balance_mono_and_delay() {
        let mut core = AudioCore::new(factory());
        let n = node(
            0.0,
            AudioConfig {
                balance: 0.0,
                sync_offset_ms: 20,
                ..Default::default()
            },
        );
        core.sync(&AudioGraph {
            nodes: vec![n],
            ..Default::default()
        });
        core.process(Timestamp::ZERO);
        // First 20 ms are delayed silence.
        assert!(core.tracks()[0].is_silent());
        run(&mut core, 3);
        let p = core.tracks()[0].peak();
        assert!(p[0] > 0.9 && p[1] == 0.0, "hard left {p:?}");
    }

    #[test]
    fn filters_keep_state_and_apply_in_order() {
        let mut core = AudioCore::new(factory());
        let mut cfg = AudioConfig::default();
        cfg.filters.push(estream_project::AudioFilterDef {
            id: "a".into(),
            kind: "gain".into(),
            enabled: true,
            settings: json!({ "gain_db": -12.0 }),
        });
        let mut n = node(0.0, cfg);
        core.sync(&AudioGraph {
            nodes: vec![n.clone()],
            ..Default::default()
        });
        run(&mut core, 2);
        assert!((db::from_gain(core.tracks()[0].peak()[0]) + 12.0).abs() < 0.1);
        // Disable the filter: full level again.
        n.config.filters[0].enabled = false;
        core.sync(&AudioGraph {
            nodes: vec![n],
            ..Default::default()
        });
        run(&mut core, 2);
        assert!(db::from_gain(core.tracks()[0].peak()[0]).abs() < 0.1);
    }

    /// Counts how often a tone is read, to prove a shared source is read once per block.
    struct Counted(ToneSource, Arc<std::sync::atomic::AtomicUsize>);
    impl AudioSource for Counted {
        fn update(&mut self, settings: &Value) -> Result<(), String> {
            self.0.update(settings)
        }
        fn read(&mut self, block: &mut Block, now: Timestamp) -> usize {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.0.read(block, now)
        }
        fn status(&self) -> SourceStatus {
            self.0.status()
        }
    }

    #[test]
    fn each_canvas_hears_only_its_own_sources() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reads.clone();
        let mut core = AudioCore::new(Arc::new(move |kind, settings| {
            (kind == "tone").then(|| {
                let mut t = ToneSource::default();
                t.update(settings)
                    .map(|_| Box::new(Counted(t, counter.clone())) as Box<dyn AudioSource>)
            })
        }));
        let (main, big, gone) = (CanvasId::new(), CanvasId::new(), CanvasId::new());
        let main_peaks = Arc::new(Mutex::new(Vec::new()));
        let big_peaks = Arc::new(Mutex::new(Vec::new()));
        let gone_peaks = Arc::new(Mutex::new(Vec::new()));
        core.add_sink(Some(main), Box::new(Capture(main_peaks.clone())));
        core.add_sink(Some(big), Box::new(Capture(big_peaks.clone())));
        core.add_sink(Some(gone), Box::new(Capture(gone_peaks.clone())));
        let mut discord = node(-6.0, AudioConfig::default());
        discord.canvases = vec![big];
        let mut shared = node(-12.0, AudioConfig::default());
        shared.canvases = vec![main, big];
        let mic = node(-20.0, AudioConfig::default()); // global: every canvas
        core.sync(&AudioGraph {
            nodes: vec![discord.clone(), shared, mic],
            canvases: vec![main, big],
            ..Default::default()
        });
        run(&mut core, 3);
        assert_eq!(
            reads.load(std::sync::atomic::Ordering::Relaxed),
            9,
            "3 sources × 3 blocks"
        );
        let level =
            |peaks: &Arc<Mutex<Vec<[f32; TRACKS]>>>| peaks.lock().unwrap().last().unwrap()[0];
        let (m, b) = (level(&main_peaks), level(&big_peaks));
        let tone = |db: f32| db::to_gain(db);
        // Tones of the same frequency and phase add up linearly.
        assert!((m - (tone(-12.0) + tone(-20.0))).abs() < 0.01, "main {m}");
        assert!(
            (b - (tone(-6.0) + tone(-12.0) + tone(-20.0))).abs() < 0.01,
            "big {b}"
        );
        assert_eq!(level(&gone_peaks), 0.0, "a canvas without a mix is silent");
        let meters = core.meters();
        assert!(!meters.heard_on(discord.source, main));
        assert!(meters.heard_on(discord.source, big));
        assert!(meters.master_of(big).read().peak[0] > meters.master_of(main).read().peak[0]);
    }
}
