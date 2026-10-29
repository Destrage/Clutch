//! Outputs on projects with several canvases (horizontal + vertical).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{gpu, probe, temp_dir, wait_frames};
use estream_core::{Alignment, BoundsMode, CanvasId, ItemTransform, Rational};
use estream_engine::{Engine, EngineConfig};
use estream_media::{Destination, MultiStream, Recording, RecordingSettings, StreamSettings};
use estream_project::{Canvas, Project, Scene, SceneItem, SourceDef};
use std::time::Duration;

fn fill(w: f32, h: f32) -> ItemTransform {
    ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: [w, h].into(),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    }
}

/// A 640x360 canvas and a 360x640 canvas, both showing one shared test pattern.
fn two_canvas_engine() -> (Engine, CanvasId, CanvasId) {
    let mut p = Project::new_default(640, 360, Rational::fps(30));
    let main = p.canvases[0].id;
    let bars = SourceDef::new(
        "Bars",
        "test_pattern",
        serde_json::json!({ "width": 640, "height": 360 }),
    );
    let mut item = SceneItem::source(bars.id);
    item.transform = fill(640.0, 360.0);
    p.scenes[0].items = vec![item];

    let mut vertical = Canvas::new("Vertical", 360, 640, Rational::fps(30));
    let mut scene = Scene::new("Vertical scene", vertical.id);
    let mut v_item = SceneItem::source(bars.id);
    v_item.transform = fill(360.0, 640.0);
    scene.items = vec![v_item];
    vertical.active_scene = Some(scene.id);
    let v = vertical.id;
    p.canvases.push(vertical);
    p.scenes.push(scene);
    let mut tone = SourceDef::new(
        "Tone",
        "audio_test_tone",
        serde_json::json!({ "frequency": 440.0, "level_db": -12.0 }),
    );
    tone.audio.global = true;
    p.sources = vec![bars, tone];
    let engine = Engine::start(
        p,
        EngineConfig {
            gpu: Some(gpu()),
            ..Default::default()
        },
    )
    .unwrap();
    (engine, main, v)
}

#[test]
fn recording_all_canvases_writes_one_file_per_canvas() {
    let (engine, _, _) = two_canvas_engine();
    let dir = temp_dir("all-canvases");
    std::thread::sleep(Duration::from_millis(300));
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            folder: dir.clone(),
            all_canvases: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rec.file_count(), 2);
    wait_frames(&rec, 30);
    let paths = rec.stop_all(&engine).unwrap();
    assert_eq!(paths.len(), 2);
    let main = probe(&paths[0]).video.unwrap();
    let vertical = probe(&paths[1]).video.unwrap();
    assert_eq!((main.1, main.2), (640, 360));
    assert_eq!((vertical.1, vertical.2), (360, 640));
    let name = paths[1].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.contains("(Vertical)"), "{name}");
}

#[test]
fn recording_one_canvas_and_unknown_canvas_falls_back() {
    let (engine, _, vertical) = two_canvas_engine();
    let dir = temp_dir("one-canvas");
    std::thread::sleep(Duration::from_millis(300));
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            folder: dir.clone(),
            canvas: Some(vertical),
            ..Default::default()
        },
    )
    .unwrap();
    wait_frames(&rec, 15);
    let path = rec.stop(&engine).unwrap();
    let v = probe(&path).video.unwrap();
    assert_eq!((v.1, v.2), (360, 640));

    // A canvas id from another collection records the first canvas instead of failing.
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            folder: dir,
            canvas: Some(CanvasId::new()),
            ..Default::default()
        },
    )
    .unwrap();
    wait_frames(&rec, 15);
    let path = rec.stop(&engine).unwrap();
    let v = probe(&path).video.unwrap();
    assert_eq!((v.1, v.2), (640, 360));
}

#[test]
fn destinations_on_different_canvases_use_separate_encoders() {
    let (engine, main, vertical) = two_canvas_engine();
    // Nothing listens on these ports; only the encoder layout matters.
    let dest = |name: &str, canvas: CanvasId| Destination {
        name: name.into(),
        settings: StreamSettings {
            server: "rtmp://127.0.0.1:9/live".into(),
            key: "k".into(),
            canvas: Some(canvas),
            ..Default::default()
        },
    };
    let stream = MultiStream::start(
        &engine,
        &[
            dest("Twitch", main),
            dest("YouTube", main),
            dest("TikTok", vertical),
        ],
    )
    .unwrap();
    assert_eq!(
        stream.encoder_count(),
        2,
        "the two horizontal destinations share one encoder, the vertical one has its own"
    );
    stream.stop(&engine).unwrap();
}
