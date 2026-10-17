//! Stats and health: turns raw counters (sampled once a second by the app) into rates,
//! a 5-minute history and plain-language alerts with advice.

pub mod system;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

pub use system::{SystemSample, SystemSampler};

/// One canvas's cumulative counters.
#[derive(Clone, Debug, Default)]
pub struct CanvasCounters {
    pub name: String,
    pub target_fps: f64,
    pub rendered: u64,
    pub lagged: u64,
    pub encoder_dropped: u64,
    /// Time to render the last frame.
    pub render_ms: f64,
}

/// One encoder's cumulative counters.
#[derive(Clone, Debug, Default)]
pub struct OutputCounters {
    /// Stable while the output runs ("record", "stream:0", …).
    pub key: String,
    pub name: String,
    pub encoder: String,
    pub frames: u64,
    pub dropped: u64,
    pub bytes: u64,
    /// Writes to the recording drive (counts toward the disk check).
    pub recording: bool,
}

/// One live destination's counters.
#[derive(Clone, Debug, Default)]
pub struct DestCounters {
    pub key: String,
    pub name: String,
    /// "Live", "Connecting", "Reconnecting", …
    pub state: String,
    pub reconnecting: bool,
    pub bytes: u64,
    pub dropped: u64,
    pub queue_ms: u64,
    pub reconnects: u64,
    /// Frame rate of the canvas it streams (for drop percentages).
    pub fps: f64,
}

/// Everything sampled at one moment.
#[derive(Clone, Debug)]
pub struct Sample {
    pub at: Instant,
    pub canvases: Vec<CanvasCounters>,
    pub outputs: Vec<OutputCounters>,
    pub destinations: Vec<DestCounters>,
    pub system: SystemSample,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    #[default]
    Ok,
    Warning,
    Problem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    /// Rule plus subject, e.g. `"encoder:record"`.
    pub id: String,
    pub severity: Severity,
    pub title: String,
    /// The numbers behind it.
    pub detail: String,
    pub advice: String,
}

/// One second of history (main canvas and totals).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub fps: f64,
    pub target_fps: f64,
    pub render_ms: f64,
    pub skipped_pct: f64,
    pub cpu: f64,
    pub cpu_process: f64,
    pub gpu: Option<f64>,
    pub memory_mb: f64,
    pub disk_free_gb: Option<f64>,
    pub upload_kbps: f64,
    pub encoder_dropped_pct: f64,
    pub network_dropped_pct: f64,
    pub queue_ms: f64,
}

/// An encoder's current rates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputRate {
    pub key: String,
    pub name: String,
    pub encoder: String,
    pub fps: f64,
    pub dropped_pct: f64,
    pub kbps: f64,
}

/// A destination's current rates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DestRate {
    pub key: String,
    pub name: String,
    pub state: String,
    pub kbps: f64,
    pub dropped_pct: f64,
    pub queue_ms: u64,
    pub reconnects: u64,
}

/// What a rule found: severity, title, detail, advice.
type Finding = (Severity, String, String, String);
/// A rule's verdict for this second: id, finding (if any), raise and clear delays in seconds.
type Condition = (String, Option<Finding>, u64, u64);

pub const HISTORY: usize = 300;
const WINDOW: usize = 10;

#[derive(Default)]
struct RuleState {
    true_since: Option<Instant>,
    false_since: Option<Instant>,
    active: Option<Alert>,
}

/// Rolling sums over the last `WINDOW` seconds of (numerator, denominator) pairs.
#[derive(Default)]
struct Window(VecDeque<(f64, f64)>);

impl Window {
    fn push(&mut self, num: f64, den: f64) {
        self.0.push_back((num, den));
        while self.0.len() > WINDOW {
            self.0.pop_front();
        }
    }

    fn ratio(&self) -> f64 {
        let (n, d) = self
            .0
            .iter()
            .fold((0.0, 0.0), |(a, b), (n, d)| (a + n, b + d));
        if d > 0.0 { n / d } else { 0.0 }
    }
}

/// Health monitor: feed it one [`Sample`] per second.
#[derive(Default)]
pub struct Monitor {
    prev: Option<Sample>,
    history: VecDeque<Point>,
    outputs: Vec<OutputRate>,
    destinations: Vec<DestRate>,
    windows: HashMap<String, Window>,
    rules: HashMap<String, RuleState>,
    latest_system: SystemSample,
}

/// Per-second delta of a counter (0 after a reset).
fn delta(now: u64, before: Option<u64>) -> f64 {
    match before {
        Some(b) if now >= b => (now - b) as f64,
        _ => 0.0,
    }
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn history(&self) -> &VecDeque<Point> {
        &self.history
    }

    pub fn latest(&self) -> Point {
        self.history.back().copied().unwrap_or_default()
    }

    pub fn outputs(&self) -> &[OutputRate] {
        &self.outputs
    }

    pub fn destinations(&self) -> &[DestRate] {
        &self.destinations
    }

    pub fn system(&self) -> &SystemSample {
        &self.latest_system
    }

    /// Active alerts, worst first.
    pub fn alerts(&self) -> Vec<Alert> {
        let mut list: Vec<Alert> = self
            .rules
            .values()
            .filter_map(|r| r.active.clone())
            .collect();
        list.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.id.cmp(&b.id)));
        list
    }

    pub fn health(&self) -> Severity {
        self.alerts().first().map_or(Severity::Ok, |a| a.severity)
    }

    /// Adds a sample; returns the alerts that were raised by it (each occurrence once).
    pub fn push(&mut self, sample: Sample) -> Vec<Alert> {
        let prev = self.prev.take();
        let secs = prev
            .as_ref()
            .map(|p| sample.at.duration_since(p.at).as_secs_f64())
            .filter(|s| *s > 0.05)
            .unwrap_or(1.0);
        self.latest_system = sample.system.clone();

        // Canvases (the first is the main one in the history).
        let mut point = Point {
            cpu: sample.system.cpu_total,
            cpu_process: sample.system.cpu_process,
            gpu: sample.system.gpu,
            memory_mb: sample.system.memory_bytes as f64 / 1_048_576.0,
            disk_free_gb: sample.system.disk_free.map(|b| b as f64 / 1e9),
            ..Default::default()
        };
        let mut conditions: Vec<Condition> = Vec::new();
        for (i, c) in sample.canvases.iter().enumerate() {
            let before = prev
                .as_ref()
                .and_then(|p| p.canvases.iter().find(|x| x.name == c.name));
            let rendered = delta(c.rendered, before.map(|b| b.rendered));
            let lagged = delta(c.lagged, before.map(|b| b.lagged));
            let expected = (c.target_fps * secs).max(1.0);
            let window = self.windows.entry(format!("lag:{}", c.name)).or_default();
            window.push(lagged, expected);
            let skipped = window.ratio() * 100.0;
            if i == 0 {
                point.fps = rendered / secs;
                point.target_fps = c.target_fps;
                point.render_ms = c.render_ms;
                point.skipped_pct = lagged / expected * 100.0;
            }
            let condition = (skipped > 2.0).then(|| {
                (
                    if skipped > 10.0 {
                        Severity::Problem
                    } else {
                        Severity::Warning
                    },
                    format!("Rendering can't keep up on “{}”", c.name),
                    format!(
                        "{skipped:.0}% of frames skipped · {:.1} ms per frame (target {:.0} fps)",
                        c.render_ms, c.target_fps
                    ),
                    "Lower the canvas frame rate or resolution, hide heavy sources, or close GPU-heavy apps.".to_owned(),
                )
            });
            conditions.push((format!("render-lag:{}", c.name), condition, 5, 10));
        }

        // Encoders.
        self.outputs.clear();
        let mut worst_encoder = 0.0f64;
        let mut recording_bytes_per_sec = 0.0;
        for o in &sample.outputs {
            let before = prev
                .as_ref()
                .and_then(|p| p.outputs.iter().find(|x| x.key == o.key));
            let frames = delta(o.frames, before.map(|b| b.frames));
            let dropped = delta(o.dropped, before.map(|b| b.dropped));
            let bytes = delta(o.bytes, before.map(|b| b.bytes));
            let window = self.windows.entry(format!("enc:{}", o.key)).or_default();
            window.push(dropped, frames + dropped);
            let pct = window.ratio() * 100.0;
            worst_encoder = worst_encoder.max(pct);
            if o.recording {
                recording_bytes_per_sec += bytes / secs;
            }
            self.outputs.push(OutputRate {
                key: o.key.clone(),
                name: o.name.clone(),
                encoder: o.encoder.clone(),
                fps: frames / secs,
                dropped_pct: pct,
                kbps: bytes * 8.0 / secs / 1000.0,
            });
            let condition = (pct > 1.0).then(|| {
                (
                    if pct > 5.0 {
                        Severity::Problem
                    } else {
                        Severity::Warning
                    },
                    format!("The {} encoder is overloaded", o.name.to_lowercase()),
                    format!("{pct:.1}% of frames dropped before encoding ({})", o.encoder),
                    "Use a hardware encoder (NVENC, AMF, Quick Sync) or a faster preset, or lower the resolution.".to_owned(),
                )
            });
            conditions.push((format!("encoder:{}", o.key), condition, 5, 10));
        }
        point.encoder_dropped_pct = worst_encoder;

        // Destinations.
        self.destinations.clear();
        let mut upload = 0.0;
        let mut worst_net = 0.0f64;
        let mut worst_queue = 0u64;
        for d in &sample.destinations {
            let before = prev
                .as_ref()
                .and_then(|p| p.destinations.iter().find(|x| x.key == d.key));
            let bytes = delta(d.bytes, before.map(|b| b.bytes));
            let dropped = delta(d.dropped, before.map(|b| b.dropped));
            let window = self.windows.entry(format!("net:{}", d.key)).or_default();
            window.push(dropped, (d.fps * secs).max(1.0));
            let pct = window.ratio() * 100.0;
            let kbps = bytes * 8.0 / secs / 1000.0;
            upload += kbps;
            worst_net = worst_net.max(pct);
            worst_queue = worst_queue.max(d.queue_ms);
            self.destinations.push(DestRate {
                key: d.key.clone(),
                name: d.name.clone(),
                state: d.state.clone(),
                kbps,
                dropped_pct: pct,
                queue_ms: d.queue_ms,
                reconnects: d.reconnects,
            });
            let congested = (pct > 1.0 || d.queue_ms > 2000).then(|| {
                (
                    Severity::Problem,
                    format!("The network can't keep up with {}", d.name),
                    format!(
                        "{pct:.1}% of frames dropped · {:.1} s queued · {kbps:.0} kb/s sent",
                        d.queue_ms as f64 / 1000.0
                    ),
                    "Lower the stream bitrate in Settings, or check your upload connection (Wi-Fi, other uploads).".to_owned(),
                )
            });
            conditions.push((format!("network:{}", d.key), congested, 3, 10));
            let reconnecting = d.reconnecting.then(|| {
                (
                    Severity::Problem,
                    format!("Reconnecting to {}", d.name),
                    format!("{} reconnects so far", d.reconnects),
                    "eStream keeps retrying on its own. Check your connection and the server address.".to_owned(),
                )
            });
            conditions.push((format!("reconnecting:{}", d.key), reconnecting, 2, 5));
        }
        point.upload_kbps = upload;
        point.network_dropped_pct = worst_net;
        point.queue_ms = worst_queue as f64;

        // System.
        let s = &sample.system;
        conditions.push((
            "cpu".into(),
            (s.cpu_total > 90.0).then(|| {
                (
                    Severity::Warning,
                    "The CPU is almost fully busy".to_owned(),
                    format!("{:.0}% total · eStream {:.0}%", s.cpu_total, s.cpu_process),
                    "Close other programs, or switch to a hardware encoder to free the CPU."
                        .to_owned(),
                )
            }),
            15,
            15,
        ));
        conditions.push((
            "gpu".into(),
            s.gpu.filter(|g| *g > 95.0).map(|g| {
                (
                    Severity::Warning,
                    "The GPU is almost fully busy".to_owned(),
                    format!("{g:.0}% 3D load"),
                    "Lower the game's graphics settings or cap its frame rate so eStream can render in time.".to_owned(),
                )
            }),
            15,
            15,
        ));
        let recording = sample.outputs.iter().any(|o| o.recording);
        let disk = s.disk_free.filter(|_| recording).and_then(|free| {
            let gb = free as f64 / 1e9;
            let minutes = (recording_bytes_per_sec > 0.0)
                .then(|| free as f64 / recording_bytes_per_sec / 60.0);
            let low_time = minutes.is_some_and(|m| m < 10.0);
            (gb < 5.0 || low_time).then(|| {
                (
                    if gb < 1.0 || minutes.is_some_and(|m| m < 3.0) {
                        Severity::Problem
                    } else {
                        Severity::Warning
                    },
                    "The recording drive is almost full".to_owned(),
                    match minutes {
                        Some(m) => {
                            format!("{gb:.1} GB free · about {m:.0} min left at this bitrate")
                        }
                        None => format!("{gb:.1} GB free"),
                    },
                    "Free some space or choose another recording folder in Settings.".to_owned(),
                )
            })
        });
        conditions.push(("disk".into(), disk, 0, 30));

        // Apply hysteresis.
        let now = sample.at;
        let mut raised = Vec::new();
        let seen: HashSet<String> = conditions.iter().map(|c| c.0.clone()).collect();
        for (id, condition, raise, clear) in conditions {
            let state = self.rules.entry(id.clone()).or_default();
            match condition {
                Some((severity, title, detail, advice)) => {
                    state.false_since = None;
                    let since = *state.true_since.get_or_insert(now);
                    if now.duration_since(since) >= Duration::from_secs(raise) {
                        let alert = Alert {
                            id: id.clone(),
                            severity,
                            title,
                            detail,
                            advice,
                        };
                        let new = state.active.is_none();
                        state.active = Some(alert.clone());
                        if new {
                            raised.push(alert);
                        }
                    }
                }
                None => {
                    state.true_since = None;
                    if state.active.is_some() {
                        let since = *state.false_since.get_or_insert(now);
                        if now.duration_since(since) >= Duration::from_secs(clear) {
                            state.active = None;
                            state.false_since = None;
                        }
                    }
                }
            }
        }
        // Subjects that went away (stopped outputs) take their alerts with them.
        self.rules.retain(|id, _| seen.contains(id));
        self.windows.retain(|key, _| {
            let subject = key.split_once(':').map_or("", |(_, s)| s);
            seen.iter().any(|id| id.ends_with(&format!(":{subject}")))
        });

        // The first sample has no baseline for its rates: keep it out of the history.
        if prev.is_some() {
            self.history.push_back(point);
        }
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.prev = Some(sample);
        raised
    }
}
