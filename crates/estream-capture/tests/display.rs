#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_capture::windows::enumerate;
use estream_core::{Rational, Timestamp};
use estream_engine::graph::RenderGraph;
use estream_engine::{SourceRegistry, SourceStatus, VideoCore};
use estream_gpu::{GpuContext, GpuOptions, readback};
use estream_project::{Project, SceneItem, SourceDef};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// One GPU device for all tests (the app also uses a single device).
fn gpu() -> GpuContext {
    static GPU: OnceLock<GpuContext> = OnceLock::new();
    GPU.get_or_init(|| GpuContext::new_headless(&GpuOptions::default()).unwrap())
        .clone()
}

/// Captures the primary monitor end-to-end (WGC → shared texture → compositor).
#[test]
fn display_capture_delivers_native_resolution_frames() {
    let Some(primary) = enumerate::monitors().into_iter().find(|m| m.primary) else {
        eprintln!("skipping: no monitor");
        return;
    };
    let gpu = gpu();
    let mut registry = SourceRegistry::with_builtins();
    estream_capture::register_all(&mut registry);

    let mut project = Project::new_default(1280, 720, Rational::fps(30));
    let source = SourceDef::new(
        "Desktop",
        "display_capture",
        serde_json::json!({ "monitor": primary.id }),
    );
    let source_id = source.id;
    let mut item = SceneItem::source(source.id);
    item.transform = estream_core::ItemTransform {
        bounds_mode: estream_core::BoundsMode::Stretch,
        bounds: [1280.0, 720.0].into(),
        ..Default::default()
    };
    project.scenes[0].items.push(item);
    project.sources.push(source);

    let mut core = VideoCore::new(gpu.clone(), registry);
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);

    let start = Instant::now();
    let mut info = None;
    while start.elapsed() < Duration::from_secs(5) {
        core.render_now(Timestamp::ZERO);
        info = core.statuses().get(&source_id).cloned();
        if info.as_ref().is_some_and(|i| i.size.is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let info = info.expect("source exists");
    assert_eq!(info.status, SourceStatus::Active, "{:?}", info.status);
    let size = info.size.expect("a captured frame within 5 s");
    assert_eq!(
        (size.width as i32, size.height as i32),
        (primary.width, primary.height)
    );

    // The composited canvas shows real desktop content (not uniformly black).
    let outputs = core.render_now(Timestamp::ZERO);
    let frame = outputs[0].latest().unwrap();
    let pixels = readback::read_texture(&gpu, frame.preview).unwrap();
    let distinct: std::collections::HashSet<[u8; 4]> = pixels
        .chunks(4)
        .step_by(97)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect();
    assert!(
        distinct.len() > 8,
        "canvas looks empty ({} distinct colors)",
        distinct.len()
    );
}

#[test]
fn missing_display_is_unavailable() {
    let gpu = gpu();
    let mut registry = SourceRegistry::with_builtins();
    estream_capture::register_all(&mut registry);
    let mut project = Project::new_default(320, 180, Rational::fps(30));
    let source = SourceDef::new(
        "Gone",
        "display_capture",
        serde_json::json!({ "monitor": "\\\\.\\DISPLAY99" }),
    );
    let id = source.id;
    project.sources.push(source);
    let mut core = VideoCore::new(gpu, registry);
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    // Capture start-up runs on a helper thread; wait for its verdict.
    let start = Instant::now();
    let mut status = SourceStatus::Active;
    while start.elapsed() < Duration::from_secs(3) {
        core.render_now(Timestamp::ZERO);
        status = core.statuses().get(&id).unwrap().status.clone();
        if matches!(status, SourceStatus::Unavailable(ref r) if r.contains("not connected")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        matches!(status, SourceStatus::Unavailable(ref r) if r.contains("not connected")),
        "{status:?}"
    );
}
