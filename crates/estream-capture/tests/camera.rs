#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Uses a real webcam, so it's opt-in: `cargo test -p estream-capture --test camera -- --ignored`.
//! Only frame sizes and status are checked; no pixels are stored.

use estream_capture::windows::camera;
use estream_core::{Rational, Timestamp};
use estream_engine::graph::RenderGraph;
use estream_engine::{SourceRegistry, SourceStatus, VideoCore};
use estream_gpu::{GpuContext, GpuOptions};
use estream_project::{Project, SourceDef};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
#[ignore = "turns on the webcam"]
fn camera_delivers_frames_in_selected_format() {
    let Some(device) = camera::devices().into_iter().next() else {
        eprintln!("skipping: no camera");
        return;
    };
    let formats = camera::formats(&device.id);
    eprintln!(
        "{}: {} formats, e.g. {:?}",
        device.name,
        formats.len(),
        formats.first().map(|f| f.label())
    );
    let chosen = camera::best_format(&formats).expect("at least one format");

    let gpu = GpuContext::new_headless(&GpuOptions::default()).unwrap();
    let mut registry = SourceRegistry::with_builtins();
    estream_capture::register_all(&mut registry);
    let mut project = Project::new_default(640, 360, Rational::fps(30));
    let source = SourceDef::new(
        "Cam",
        "camera",
        serde_json::json!({ "device": device.id, "format": chosen.key() }),
    );
    let id = source.id;
    project.sources.push(source);
    let mut core = VideoCore::new(gpu, registry);
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);

    let start = Instant::now();
    let mut info = None;
    while start.elapsed() < Duration::from_secs(10) {
        core.render_now(Timestamp::ZERO);
        info = core.statuses().get(&id).cloned();
        if info.as_ref().is_some_and(|i| i.size.is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let info = info.unwrap();
    assert_eq!(info.status, SourceStatus::Active, "{:?}", info.status);
    let size = info.size.expect("a camera frame within 10 s");
    assert_eq!(
        (size.width as u32, size.height as u32),
        (chosen.width & !1, chosen.height & !1)
    );
    eprintln!(
        "got {}x{} frames from {}",
        size.width,
        size.height,
        chosen.label()
    );
}
