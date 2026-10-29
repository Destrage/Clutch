//! Helpers shared by the output integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use estream_core::{Alignment, BoundsMode, ItemTransform, Rational};
use estream_engine::{Command, Engine, EngineConfig};
use estream_gpu::{GpuContext, GpuOptions};
use estream_media::{Recording, ffmpeg};
use estream_project::{Project, SceneItem, SourceDef};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn gpu() -> GpuContext {
    static GPU: OnceLock<GpuContext> = OnceLock::new();
    GPU.get_or_init(|| GpuContext::new_headless(&GpuOptions::default()).unwrap())
        .clone()
}

/// CI runners are shared VMs whose GPU tests render on the CPU (WARP, lavapipe), so the first
/// video frame can reach the encoder well after audio starts (GitHub sets `CI`).
pub fn shared_runner() -> bool {
    std::env::var_os("CI").is_some()
}

/// Waits until `rec`'s first file has `frames` encoded frames. The file only exists once the
/// first frame reaches the encoder, which takes longer on CI runners that render on the CPU, so
/// a fixed sleep isn't enough there. A failed output panics with its own error.
pub fn wait_frames(rec: &Recording, frames: u64) {
    let stats = rec.stats();
    let start = Instant::now();
    while stats.frames.load(Ordering::Relaxed) < frames {
        if let Some(e) = stats.error.lock().unwrap().clone() {
            panic!("recording failed: {e}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{} of {frames} frames after 20 s (encoder {:?})",
            stats.frames.load(Ordering::Relaxed),
            stats.encoder.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("estream-rec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Engine with a 640x360@30 canvas showing a test pattern and a 1 kHz tone on tracks 1 and 2.
pub fn engine() -> Engine {
    let engine = Engine::start(
        Project::new_default(640, 360, Rational::fps(30)),
        EngineConfig {
            gpu: Some(gpu()),
            ..Default::default()
        },
    )
    .unwrap();
    let scene = engine.snapshot().scenes[0].id;
    let pattern = SourceDef::new(
        "Bars",
        "test_pattern",
        serde_json::json!({ "width": 640, "height": 360 }),
    );
    let mut item = SceneItem::source(pattern.id);
    item.transform = ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: [640.0, 360.0].into(),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    };
    let mut tone = SourceDef::new(
        "Tone",
        "audio_test_tone",
        serde_json::json!({ "frequency": 1000.0, "level_db": -12.0 }),
    );
    tone.audio.global = true;
    tone.audio.tracks = 0b11;
    engine
        .execute(Command::Batch {
            commands: vec![
                Command::AddSource {
                    source: pattern,
                    index: 0,
                },
                Command::AddSource {
                    source: tone,
                    index: 1,
                },
                Command::AddItem {
                    scene,
                    parent: None,
                    index: 0,
                    item,
                },
            ],
        })
        .unwrap();
    engine
}

pub struct Probe {
    pub duration: f64,
    pub video: Option<(String, u32, u32)>,
    pub audio: Vec<String>,
    pub stream_durations: Vec<f64>,
    /// Whether the first video packet in the file is a keyframe.
    pub starts_with_key: bool,
}

impl Probe {
    pub fn audio_streams(&self) -> usize {
        self.audio.len()
    }
}

pub fn probe(path: &Path) -> Probe {
    estream_media::init();
    let mut ictx = ffmpeg::format::input(path).unwrap();
    let mut video = None;
    let mut video_index = None;
    let mut audio = Vec::new();
    let mut tbs = Vec::new();
    for s in ictx.streams() {
        let p = s.parameters();
        tbs.push(s.time_base());
        match p.medium() {
            ffmpeg::media::Type::Video => {
                let dec = ffmpeg::codec::context::Context::from_parameters(p.clone())
                    .unwrap()
                    .decoder()
                    .video()
                    .unwrap();
                video = Some((format!("{:?}", p.id()), dec.width(), dec.height()));
                video_index = Some(s.index());
            }
            ffmpeg::media::Type::Audio => audio.push(format!("{:?}", p.id())),
            _ => {}
        }
    }
    let duration = ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;
    // Per-stream extent from packet timestamps.
    let mut first = vec![f64::MAX; tbs.len()];
    let mut ends = vec![0.0f64; tbs.len()];
    let mut starts_with_key = None;
    for (s, p) in ictx.packets() {
        let tb = tbs[s.index()];
        let at = |v: i64| v as f64 * tb.0 as f64 / tb.1 as f64;
        let pts = p.pts().or(p.dts()).unwrap_or(0);
        first[s.index()] = first[s.index()].min(at(pts));
        ends[s.index()] = ends[s.index()].max(at(pts + p.duration()));
        if Some(s.index()) == video_index && starts_with_key.is_none() {
            starts_with_key = Some(p.is_key());
        }
    }
    let stream_durations = ends
        .iter()
        .zip(&first)
        .map(|(e, f)| if *f == f64::MAX { 0.0 } else { e - f })
        .collect();
    Probe {
        duration,
        video,
        audio,
        stream_durations,
        starts_with_key: starts_with_key.unwrap_or(false),
    }
}
