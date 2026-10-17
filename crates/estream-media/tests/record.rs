#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_core::{Alignment, BoundsMode, ItemTransform, Rational};
use estream_engine::{Command, Engine, EngineConfig};
use estream_gpu::{GpuContext, GpuOptions};
use estream_media::ffmpeg;
use estream_media::{Container, Recording, RecordingSettings, ReplayBuffer, VideoCodec, encode};
use estream_project::{Project, SceneItem, SourceDef};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

fn gpu() -> GpuContext {
    static GPU: OnceLock<GpuContext> = OnceLock::new();
    GPU.get_or_init(|| GpuContext::new_headless(&GpuOptions::default()).unwrap())
        .clone()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("estream-rec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Engine with a 640x360@30 canvas showing a test pattern and a 1 kHz tone on tracks 1 and 2.
fn engine() -> Engine {
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

struct Probe {
    duration: f64,
    video: Option<(String, u32, u32)>,
    audio_streams: usize,
    stream_durations: Vec<f64>,
}

fn probe(path: &Path) -> Probe {
    estream_media::init();
    let mut ictx = ffmpeg::format::input(path).unwrap();
    let mut video = None;
    let mut audio_streams = 0;
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
            }
            ffmpeg::media::Type::Audio => audio_streams += 1,
            _ => {}
        }
    }
    let duration = ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;
    // Per-stream duration from packet timestamps.
    let mut ends = vec![0.0f64; tbs.len()];
    for (s, p) in ictx.packets() {
        let tb = tbs[s.index()];
        let end = (p.pts().unwrap_or(0) + p.duration()) as f64 * tb.0 as f64 / tb.1 as f64;
        ends[s.index()] = ends[s.index()].max(end);
    }
    Probe {
        duration,
        video,
        audio_streams,
        stream_durations: ends,
    }
}

fn settings(dir: &Path, container: Container) -> RecordingSettings {
    RecordingSettings {
        folder: dir.to_path_buf(),
        container,
        tracks: 0b11,
        ..Default::default()
    }
}

#[test]
fn hardware_encoder_is_picked_automatically() {
    let list = encode::available(VideoCodec::H264);
    eprintln!(
        "H.264 encoders: {:?}",
        list.iter().map(|e| e.name).collect::<Vec<_>>()
    );
    assert!(!list.is_empty(), "at least the software encoder opens");
    let chosen = encode::choose(&RecordingSettings::default()).unwrap();
    if list.iter().any(|e| e.hardware) {
        assert!(
            chosen.hardware,
            "auto picks hardware first: {}",
            chosen.name
        );
    }
}

#[test]
fn records_video_and_two_audio_tracks_in_sync() {
    let engine = engine();
    let dir = temp_dir("mkv");
    std::thread::sleep(Duration::from_millis(300));
    let rec = Recording::start(&engine, &settings(&dir, Container::Mkv)).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let frames = rec
        .stats()
        .frames
        .load(std::sync::atomic::Ordering::Relaxed);
    let path = rec.stop(&engine).unwrap();
    let p = probe(&path);
    eprintln!(
        "{} frames, duration {:.3}s, streams {:?}",
        frames, p.duration, p.stream_durations
    );
    let (codec, w, h) = p.video.unwrap();
    assert_eq!((w, h), (640, 360));
    assert!(codec.contains("H264"), "{codec}");
    assert_eq!(p.audio_streams, 2);
    assert!((p.duration - 3.0).abs() < 0.3, "duration {}", p.duration);
    let v = p.stream_durations[0];
    for a in &p.stream_durations[1..] {
        assert!(
            (v - a).abs() < 0.1,
            "A/V duration mismatch: video {v} audio {a}"
        );
    }
}

#[test]
fn every_container_is_playable_and_mkv_remuxes() {
    let engine = engine();
    let dir = temp_dir("containers");
    std::thread::sleep(Duration::from_millis(300));
    for container in Container::ALL {
        let rec = Recording::start(&engine, &settings(&dir, container)).unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let path = rec.stop(&engine).unwrap();
        let p = probe(&path);
        assert!(p.duration > 0.8, "{container:?}: {}", p.duration);
        assert_eq!(p.audio_streams, 2, "{container:?}");
        // Distinct names (same second) must not overwrite each other.
        std::thread::sleep(Duration::from_millis(50));
    }
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            auto_remux: true,
            ..settings(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let mp4 = rec.stop(&engine).unwrap();
    assert_eq!(mp4.extension().unwrap(), "mp4");
    let p = probe(&mp4);
    assert!(p.video.is_some() && p.audio_streams == 2 && p.duration > 0.8);
}

#[test]
fn replay_buffer_saves_the_last_seconds() {
    let engine = engine();
    let dir = temp_dir("replay");
    std::thread::sleep(Duration::from_millis(300));
    let replay = ReplayBuffer::start(
        &engine,
        &RecordingSettings {
            replay_seconds: 5,
            ..settings(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(9));
    let path = replay.save().unwrap();
    // The buffer keeps running after a save.
    std::thread::sleep(Duration::from_millis(500));
    assert!(replay.save().is_ok());
    replay.stop(&engine).unwrap();
    let p = probe(&path);
    eprintln!("replay duration {:.2}s", p.duration);
    assert!(
        p.duration >= 4.5 && p.duration <= 7.5,
        "replay {}",
        p.duration
    );
    assert!(p.video.is_some() && p.audio_streams == 2);
}
