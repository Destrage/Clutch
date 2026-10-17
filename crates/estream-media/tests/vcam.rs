//! The virtual camera feed, with the test playing the camera source's side of the shared memory.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use estream_core::Rational;
use estream_engine::{Engine, EngineConfig};
use estream_media::vcam::{VcamState, VirtualCamOutput};
use estream_project::{Project, SceneItem, SourceDef};
use estream_vcam::shared::Shared;
use std::time::{Duration, Instant};

fn red_engine() -> Engine {
    let mut project = Project::new_default(640, 360, Rational::fps(30));
    let red = SourceDef::new(
        "Red",
        "color",
        serde_json::json!({ "width": 640, "height": 360, "color": { "r": 255, "g": 0, "b": 0, "a": 255 } }),
    );
    project.scenes[0].items.push(SceneItem::source(red.id));
    project.sources.push(red);
    Engine::start(
        project,
        EngineConfig {
            gpu: Some(common::gpu()),
            audio: false,
            ..Default::default()
        },
    )
    .unwrap()
}

fn wait_state(out: &VirtualCamOutput, want: &VcamState, consumer: Option<&Shared>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while out.state() != *want {
        assert!(
            Instant::now() < deadline,
            "state {:?}, wanted {want:?}",
            out.state()
        );
        if let Some(c) = consumer {
            c.touch_consumer();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn feeds_frames_only_while_watched() {
    let engine = red_engine();
    let canvas = engine.snapshot().canvases[0].id;
    let mapping = format!("Local\\estream-vcam-feed-{}", std::process::id());
    let out = VirtualCamOutput::start_with(engine.video_tap(), canvas, &mapping, false);
    wait_state(&out, &VcamState::On, None);

    // An app opens the camera at 1280×720: the source creates the mapping and asks.
    let consumer = Shared::create(&mapping).unwrap();
    consumer.set_wanted(1280, 720);
    consumer.touch_consumer();
    wait_state(&out, &VcamState::InUse, Some(&consumer));

    let mut frame = Vec::new();
    let mut seq = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut reads = 0;
    let (w, h) = loop {
        consumer.touch_consumer();
        if let Some(size) = consumer.read_frame(&mut frame, &mut seq) {
            reads += 1;
            if frame[0] > 20 {
                break size;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no frame from the feed (reads {reads}, seq {seq}, y {:?}, alive {})",
            frame.first(),
            consumer.producer_alive(1000)
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((w, h), (1280, 720), "scaled to what the app asked for");
    let (y, u, v) = (
        frame[640 * 1280 / 2],
        frame[1280 * 720],
        frame[1280 * 720 + 1],
    );
    assert!(
        (50..=90).contains(&y) && u < 128 && v > 200,
        "red: {y} {u} {v}"
    );
    assert!(consumer.producer_alive(1000));

    // The app closes the camera: the feed lets go of the engine.
    wait_state(&out, &VcamState::On, None);
    out.stop();
}
