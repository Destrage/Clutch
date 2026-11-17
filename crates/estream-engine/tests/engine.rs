#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end engine tests on a real (or software) GPU.

use estream_core::{Alignment, BoundsMode, EngineClock, ItemTransform, Rational, Rgba8, Timestamp};
use estream_engine::frame_clock::{FrameClock, wait_until};
use estream_engine::graph::RenderGraph;
use estream_engine::{
    Command, Engine, EngineConfig, Event, ItemPatch, NOT_ON_THIS_SYSTEM, SourceRegistry,
    SourceStatus, SourceType, VideoCore,
};
use estream_gpu::{GpuContext, GpuOptions, readback};
use estream_project::{Canvas, Project, Scene, SceneItem, SourceDef, persist};
use glam::Vec2;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

fn gpu() -> GpuContext {
    static GPU: OnceLock<GpuContext> = OnceLock::new();
    GPU.get_or_init(|| {
        GpuContext::new_headless(&GpuOptions::default())
            .or_else(|_| {
                GpuContext::new_headless(&GpuOptions {
                    force_software: true,
                })
            })
            .expect("GPU adapter")
    })
    .clone()
}

fn config() -> EngineConfig {
    EngineConfig {
        gpu: Some(gpu()),
        ..Default::default()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("estream-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fill(width: f32, height: f32) -> ItemTransform {
    ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: Vec2::new(width, height),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    }
}

/// Adds a source and a full-canvas item for it; returns (source, item).
fn add_fullscreen(
    engine: &Engine,
    kind: &str,
    settings: serde_json::Value,
) -> (estream_core::SourceId, estream_core::ItemId) {
    let project = engine.snapshot();
    let canvas = &project.canvases[0];
    let scene = canvas.active_scene.unwrap();
    let source = SourceDef::new(kind, kind, settings);
    let mut item = SceneItem::source(source.id);
    item.transform = fill(canvas.width as f32, canvas.height as f32);
    let ids = (source.id, item.id);
    engine
        .execute(Command::Batch {
            commands: vec![
                Command::AddSource {
                    source,
                    index: usize::MAX,
                },
                Command::AddItem {
                    scene,
                    parent: None,
                    index: usize::MAX,
                    item,
                },
            ],
        })
        .unwrap();
    ids
}

fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn engine_renders_live_frames_and_applies_edits() {
    let _turn = gpu_turn();
    let engine =
        Engine::start(Project::new_default(320, 180, Rational::fps(60)), config()).unwrap();
    let canvas = engine.snapshot().canvases[0].id;
    let (_, item) = add_fullscreen(
        &engine,
        "color",
        serde_json::json!({ "color": Rgba8::new(255, 0, 0, 255) }),
    );

    let output = engine.canvas_output(canvas).expect("canvas output");
    let rev = engine.revision();
    // Wait until a frame rendered after the edit is available, then check its color.
    std::thread::sleep(Duration::from_millis(100));
    assert!(wait_for(
        || output.latest().is_some_and(|f| f.sequence > 3),
        Duration::from_secs(2)
    ));
    let frame = output.latest().unwrap();
    let pixels = readback::read_texture(engine.gpu().unwrap(), frame.preview).unwrap();
    assert_eq!(&pixels[..4], &[255, 0, 0, 255], "revision {rev}");

    // Hide the item through the command bus: the canvas returns to black.
    let scene = engine.snapshot().scenes[0].id;
    engine
        .execute(Command::UpdateItem {
            scene,
            item,
            patch: ItemPatch {
                visible: Some(false),
                ..Default::default()
            },
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let frame = output.latest().unwrap();
    let pixels = readback::read_texture(engine.gpu().unwrap(), frame.preview).unwrap();
    assert_eq!(&pixels[..4], &[0, 0, 0, 255]);

    // And undo brings it back.
    assert!(engine.undo().unwrap());
    std::thread::sleep(Duration::from_millis(100));
    let frame = output.latest().unwrap();
    let pixels = readback::read_texture(engine.gpu().unwrap(), frame.preview).unwrap();
    assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
}

#[test]
fn two_canvases_render_at_their_own_size_and_rate() {
    let _turn = gpu_turn();
    let mut project = Project::new_default(1920, 1080, Rational::fps(60));
    let mut vertical = Canvas::new("Vertical", 1080, 1920, Rational::fps(30));
    let scene = Scene::new("Vertical scene", vertical.id);
    vertical.active_scene = Some(scene.id);
    project.canvases.push(vertical);
    project.scenes.push(scene);
    let engine = Engine::start(project, config()).unwrap();

    let outputs = engine.canvas_outputs();
    assert_eq!(outputs.len(), 2);
    assert_eq!((outputs[0].width, outputs[0].height), (1920, 1080));
    assert_eq!((outputs[1].width, outputs[1].height), (1080, 1920));

    let start: Vec<u64> = outputs
        .iter()
        .map(|o| o.stats.rendered.load(std::sync::atomic::Ordering::Relaxed))
        .collect();
    std::thread::sleep(Duration::from_secs(2));
    let rendered: Vec<u64> = outputs
        .iter()
        .zip(&start)
        .map(|(o, s)| o.stats.rendered.load(std::sync::atomic::Ordering::Relaxed) - s)
        .collect();
    // Never more than the rate allows. On a shared runner, stalls may cost frames.
    let (min60, min30) = if shared_runner() { (90, 45) } else { (115, 57) };
    assert!(
        (min60..=125).contains(&rendered[0]),
        "60 fps canvas rendered {}",
        rendered[0]
    );
    assert!(
        (min30..=63).contains(&rendered[1]),
        "30 fps canvas rendered {}",
        rendered[1]
    );
}

/// CI runners are shared VMs whose GPU tests render on the CPU (WARP, lavapipe), so a thread
/// can lose the CPU for tens of milliseconds. Timing tests keep their strict limits on real
/// machines and check only what such stalls can't explain on CI (GitHub sets `CI`).
fn shared_runner() -> bool {
    std::env::var_os("CI").is_some()
}

/// Live-engine timing tests and headless full renders take turns. On CI's software GPU,
/// building a video core (compiling every pipeline) and rendering it next to a live engine can
/// stall that engine's frames for over a second.
fn gpu_turn() -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn frame_pacing_is_stable() {
    let clock = EngineClock::new();
    let rate = Rational::fps(60);
    let mut frames = FrameClock::new(rate, clock.now());
    let mut errors = Vec::new();
    for _ in 0..300 {
        let deadline = frames.next_deadline();
        wait_until(&clock, deadline);
        let woke = clock.now();
        frames.advance(woke);
        errors.push(woke.0.saturating_sub(deadline.0));
    }
    errors.sort_unstable();
    let at = |q: usize| Duration::from_nanos(errors[errors.len() * q / 100]);
    let (p50, p90, p99) = (at(50), at(90), at(99));
    let spread = format!("wake-up error p50 {p50:?}, p90 {p90:?}, p99 {p99:?}");
    // The wait sleeps instead of spinning, so wakes run a little late (frames keep their
    // scheduled timestamps); a late wake must still leave most of a 60 fps frame. A coarse
    // timer would make most wakes late, which shows in the median even on a shared runner.
    assert!(p50 < Duration::from_millis(2), "{spread}");
    if !shared_runner() {
        assert!(p99 < Duration::from_millis(2), "{spread}");
        assert_eq!(frames.lagged(), 0, "{spread}");
    }
}

/// Long-running pacing check from the spec (10 minutes). Run with `--ignored`.
#[test]
#[ignore]
fn frame_pacing_ten_minutes() {
    let engine = Engine::start(
        Project::new_default(1920, 1080, Rational::fps(60)),
        config(),
    )
    .unwrap();
    add_fullscreen(&engine, "test_pattern", serde_json::Value::Null);
    let output = engine.canvas_outputs()[0].clone();
    let start = Instant::now();
    let base = output
        .stats
        .rendered
        .load(std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(600));
    let rendered = output
        .stats
        .rendered
        .load(std::sync::atomic::Ordering::Relaxed)
        - base;
    let expected = (start.elapsed().as_secs_f64() * 60.0) as u64;
    assert!(
        rendered.abs_diff(expected) <= 1,
        "rendered {rendered}, expected {expected}"
    );
    assert_eq!(
        output
            .stats
            .lagged
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn slow_consumer_does_not_stall_rendering() {
    let _turn = gpu_turn();
    let engine =
        Engine::start(Project::new_default(320, 180, Rational::fps(60)), config()).unwrap();
    let sub = engine.subscribe(); // never read during the burst
    let scene = engine.snapshot().scenes[0].id;
    let output = engine.canvas_outputs()[0].clone();
    let before = output.latest().map(|f| f.sequence).unwrap_or(0);
    for i in 0..3000 {
        engine
            .submit(Command::RenameScene {
                scene,
                name: format!("n{i}"),
            })
            .unwrap();
    }
    std::thread::sleep(Duration::from_millis(500));
    let after = output.latest().unwrap().sequence;
    // 30 frames are due in 500 ms; a CPU renderer on a shared runner may manage half.
    let min = if shared_runner() { 5 } else { 20 };
    assert!(
        after > before + min,
        "rendering continued ({before} -> {after})"
    );
    assert!(sub.take_lagged(), "consumer is told it missed events");
    assert!(wait_for(
        || engine.snapshot().scenes[0].name == "n2999",
        Duration::from_secs(2)
    ));
}

#[test]
fn unknown_source_type_is_unavailable_not_fatal() {
    let engine =
        Engine::start(Project::new_default(320, 180, Rational::fps(30)), config()).unwrap();
    let (source, _) = add_fullscreen(&engine, "holo_projector", serde_json::Value::Null);
    assert!(wait_for(
        || matches!(
            engine.source_status(source),
            Some(SourceStatus::Unavailable(_))
        ),
        Duration::from_secs(2)
    ));
}

#[test]
fn other_systems_source_type_loads_as_not_available() {
    let mut registry = SourceRegistry::with_builtins();
    registry.register(SourceType::unavailable(
        "game_capture",
        "Game capture",
        "",
        true,
    ));
    registry.register(SourceType::unavailable(
        "app_audio",
        "Application audio",
        "",
        false,
    ));
    let engine = Engine::start(
        Project::new_default(320, 180, Rational::fps(30)),
        EngineConfig {
            registry,
            ..config()
        },
    )
    .unwrap();
    let settings = serde_json::json!({ "mode": "auto", "cursor": false });
    let (source, _) = add_fullscreen(&engine, "game_capture", settings.clone());
    assert!(wait_for(
        || engine.source_status(source)
            == Some(SourceStatus::Unavailable(NOT_ON_THIS_SYSTEM.into())),
        Duration::from_secs(2)
    ));
    // Kept as it was, so the project still works on the system that has it.
    let snapshot = engine.snapshot();
    let def = snapshot.source(source).unwrap();
    assert_eq!(
        (def.kind.as_str(), &def.settings),
        ("game_capture", &settings)
    );

    let audio = engine
        .registry()
        .create_audio("app_audio", &settings)
        .unwrap()
        .unwrap();
    assert_eq!(
        audio.status(),
        SourceStatus::Unavailable(NOT_ON_THIS_SYSTEM.into())
    );
    assert!(!engine.registry().get("app_audio").unwrap().is_available());
}

#[test]
fn autosave_writes_recovery_and_clean_shutdown_saves() {
    let dir = temp_dir("autosave");
    let path = dir.join("show.json");
    let engine = Engine::start(
        Project::new_default(320, 180, Rational::fps(30)),
        EngineConfig {
            collection_path: Some(path.clone()),
            autosave_delay: Duration::from_millis(100),
            video: false,
            ..Default::default()
        },
    )
    .unwrap();
    let events = engine.subscribe();
    let scene = engine.snapshot().scenes[0].id;
    engine
        .execute(Command::RenameScene {
            scene,
            name: "Live".into(),
        })
        .unwrap();

    assert!(wait_for(
        || persist::find_recovery(&path).is_some(),
        Duration::from_secs(2)
    ));
    let recovered = persist::load(&persist::recovery_path(&path)).unwrap();
    assert_eq!(recovered.project.scenes[0].name, "Live");
    let mut saw_recovery_event = false;
    while let Some(e) = events.recv_timeout(Duration::from_millis(50)) {
        saw_recovery_event |= matches!(e, Event::RecoverySaved { .. });
    }
    assert!(saw_recovery_event);

    engine.shutdown();
    assert_eq!(persist::load(&path).unwrap().project.scenes[0].name, "Live");
    assert!(
        persist::find_recovery(&path).is_none(),
        "clean shutdown removes recovery file"
    );
}

#[test]
fn engine_rejects_invalid_commands_without_changes() {
    let engine = Engine::start(
        Project::new_default(320, 180, Rational::fps(30)),
        EngineConfig {
            video: false,
            ..Default::default()
        },
    )
    .unwrap();
    let before = engine.snapshot();
    let scene = before.scenes[0].id;
    let err = engine
        .execute(Command::AddItem {
            scene,
            parent: None,
            index: 0,
            item: SceneItem::source(estream_core::SourceId::new()),
        })
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"));
    assert_eq!(*engine.snapshot(), *before);
    assert!(!engine.undo().unwrap(), "nothing recorded");
}

fn render_once(project: &Project) -> Vec<u8> {
    let _turn = gpu_turn();
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(project, 1)), Timestamp::ZERO);
    let outputs = core.render_now(Timestamp::ZERO);
    let frame = outputs[0].latest().unwrap();
    readback::read_texture(&gpu, frame.preview).unwrap()
}

/// Checks a row of rendered pixels against the gradient's own colors at each pixel center
/// (the dither and the trip through the canvas allow a few levels).
fn assert_row_matches(pixels: &[u8], width: u32, paint: &estream_core::Paint) {
    for x in (0..width).step_by(7).chain([width - 1]) {
        let i = ((8 * width + x) * 4) as usize;
        let got = &pixels[i..i + 4];
        let want = paint.color_at((x as f32 + 0.5) / width as f32).to_array();
        assert!(
            got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 4),
            "x {x}: {got:?} vs {want:?}"
        );
    }
}

#[test]
fn gradient_canvas_background_shows_where_no_item_covers() {
    let mut project = Project::new_default(256, 16, Rational::fps(30));
    let paint = estream_core::Paint::linear(Rgba8::new(255, 0, 0, 255), Rgba8::new(0, 0, 255, 255));
    project.canvases[0].background_paint = Some(paint.clone());
    assert_row_matches(&render_once(&project), 256, &paint);
}

#[test]
fn gradient_color_source_renders_its_gradient() {
    let mut project = Project::new_default(128, 16, Rational::fps(30));
    let paint = estream_core::Paint::linear(Rgba8::WHITE, Rgba8::BLACK);
    let source = SourceDef::new(
        "Fade",
        "color",
        serde_json::json!({ "width": 128, "height": 16, "style": "linear", "gradient": paint }),
    );
    let mut item = SceneItem::source(source.id);
    item.transform = fill(128.0, 16.0);
    project.scenes[0].items = vec![item];
    project.sources = vec![source];
    assert_row_matches(&render_once(&project), 128, &paint);
}

#[test]
fn fill_paints_the_empty_part_of_a_fitted_box() {
    use estream_project::ItemFill;
    // 16:9 bars fitted in a 9:16 canvas: bars in the middle, empty space above and below.
    let render = |fill: ItemFill| {
        let mut project = Project::new_default(90, 160, Rational::fps(30));
        let bars = SourceDef::new(
            "Bars",
            "test_pattern",
            serde_json::json!({ "width": 160, "height": 90 }),
        );
        let mut item = SceneItem::source(bars.id);
        item.transform = ItemTransform {
            bounds_mode: BoundsMode::Fit,
            bounds: Vec2::new(90.0, 160.0),
            bounds_alignment: Alignment::Center,
            ..Default::default()
        };
        item.fill = fill;
        project.scenes[0].items = vec![item];
        project.sources = vec![bars];
        render_once(&project)
    };
    let at = |p: &[u8], x: u32, y: u32| {
        let i = ((y * 90 + x) * 4) as usize;
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    };
    let none = render(ItemFill::None);
    assert_eq!(
        at(&none, 45, 10),
        [0, 0, 0, 255],
        "no fill: the black background"
    );

    let grey = Rgba8::new(40, 60, 80, 255);
    let color = render(ItemFill::Paint {
        paint: estream_core::Paint::solid(grey),
    });
    let top = at(&color, 45, 10);
    assert!(
        top.iter()
            .zip(grey.to_array())
            .all(|(a, b)| a.abs_diff(b) <= 2),
        "{top:?}"
    );
    assert_eq!(
        at(&color, 45, 80),
        at(&none, 45, 80),
        "the source itself is unchanged"
    );

    let frosted = render(ItemFill::FROSTED);
    // ESTREAM_FILL_PNG=<folder> writes the three pictures for a look.
    if let Some(dir) = std::env::var_os("ESTREAM_FILL_PNG") {
        let dir = PathBuf::from(dir);
        for (name, p) in [("none", &none), ("color", &color), ("frosted", &frosted)] {
            readback::save_png(&dir.join(format!("fill-{name}.png")), 90, 160, p).unwrap();
        }
    }
    let top = at(&frosted, 45, 10);
    let bottom = at(&frosted, 45, 150);
    assert!(
        top[..3].iter().map(|&v| v as u32).sum::<u32>() > 30,
        "frost shows the blurred source above it: {top:?}"
    );
    assert!(
        bottom[..3].iter().map(|&v| v as u32).sum::<u32>() > 30,
        "and below: {bottom:?}"
    );
    assert_eq!(at(&frosted, 45, 80), at(&none, 45, 80));
}

/// What Frosted costs per frame on a 1080 × 1920 canvas with a 1920 × 1080 source fitted in it.
/// Prints the time with and without it. Run with `--ignored --nocapture` on the machine to
/// measure; the budget check is loose so a slow software adapter doesn't fail it.
#[test]
#[ignore]
fn frosted_fill_cost() {
    let _turn = gpu_turn();
    use estream_project::ItemFill;
    let gpu = gpu();
    let measure = |fill: ItemFill| {
        let mut project = Project::new_default(1080, 1920, Rational::fps(60));
        let bars = SourceDef::new(
            "Bars",
            "test_pattern",
            serde_json::json!({ "width": 1920, "height": 1080 }),
        );
        let mut item = SceneItem::source(bars.id);
        item.transform = ItemTransform {
            bounds_mode: BoundsMode::Fit,
            bounds: Vec2::new(1080.0, 1920.0),
            bounds_alignment: Alignment::Center,
            ..Default::default()
        };
        item.fill = fill;
        project.scenes[0].items = vec![item];
        project.sources = vec![bars];
        let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
        core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
        for i in 0..30 {
            core.render_now(Timestamp(i * 16_666_667));
        }
        let frames = 240u64;
        let start = Instant::now();
        for i in 0..frames {
            core.render_now(Timestamp((30 + i) * 16_666_667));
        }
        start.elapsed().as_secs_f64() * 1000.0 / frames as f64
    };
    let none = measure(ItemFill::None);
    let frosted = measure(ItemFill::FROSTED);
    let max_blur = measure(ItemFill::Frosted {
        blur: 1.0,
        dim: 0.25,
        saturation: 0.9,
    });
    println!(
        "per frame: none {none:.3} ms, frosted {frosted:.3} ms (+{:.3}), blur 100% {max_blur:.3} ms (+{:.3})",
        frosted - none,
        max_blur - none
    );
    assert!(
        max_blur - none < 4.0,
        "Frosted costs {:.3} ms a frame",
        max_blur - none
    );
}

/// Golden image of a fixed composite scene rendered headless at t = 0.
/// Set `ESTREAM_BLESS=1` to (re)generate the reference image.
#[test]
fn golden_composite_scene() {
    let _turn = gpu_turn();
    let mut project = Project::new_default(320, 180, Rational::fps(30));
    let bars = SourceDef::new(
        "Bars",
        "test_pattern",
        serde_json::json!({ "width": 640, "height": 360 }),
    );
    let tint = SourceDef::new(
        "Tint",
        "color",
        serde_json::json!({ "width": 100, "height": 100, "color": Rgba8::new(0, 128, 255, 255) }),
    );
    let mut bars_item = SceneItem::source(bars.id);
    bars_item.transform = fill(320.0, 180.0);
    let mut tint_item = SceneItem::source(tint.id);
    tint_item.transform = ItemTransform {
        position: Vec2::new(200.0, 20.0),
        rotation: 15.0,
        scale: Vec2::splat(0.8),
        ..Default::default()
    };
    tint_item.opacity = 0.6;
    let mut grouped = SceneItem::source(tint.id);
    grouped.transform.position = Vec2::new(10.0, 10.0);
    grouped.blend = estream_core::BlendMode::Screen;
    let mut group = SceneItem::group("G", vec![grouped]);
    group.transform.position = Vec2::new(20.0, 90.0);
    group.transform.scale = Vec2::splat(0.5);
    project.scenes[0].items = vec![bars_item, tint_item, group];
    project.sources = vec![bars, tint];

    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    let outputs = core.render_now(Timestamp::ZERO);
    let frame = outputs[0].latest().unwrap();
    let pixels = readback::read_texture(&gpu, frame.preview).unwrap();

    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/composite.png");
    if std::env::var_os("ESTREAM_BLESS").is_some() || !golden.exists() {
        readback::save_png(&golden, 320, 180, &pixels).unwrap();
        eprintln!("wrote golden image {}", golden.display());
    }
    let (w, h, expected) = readback::load_png(&golden).unwrap();
    assert_eq!((w, h), (320, 180));
    let mismatched = pixels
        .chunks(4)
        .zip(expected.chunks(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 3))
        .count();
    // Allow a few edge pixels to differ across GPU vendors' rasterizers.
    let allowed = (w * h) as usize / 500;
    if mismatched > allowed {
        let actual = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/output/composite.actual.png");
        readback::save_png(&actual, w, h, &pixels).unwrap();
        panic!(
            "{mismatched} pixels differ (allowed {allowed}); actual written to {}",
            actual.display()
        );
    }
}

#[test]
fn invalid_source_settings_are_rejected_by_schema() {
    let engine = Engine::start(
        Project::new_default(320, 180, Rational::fps(30)),
        EngineConfig {
            video: false,
            ..Default::default()
        },
    )
    .unwrap();
    let (source, _) = add_fullscreen(&engine, "color", serde_json::json!({ "width": 100 }));
    let before = engine.snapshot();
    let err = engine
        .execute(Command::SetSourceSettings {
            source,
            settings: serde_json::json!({ "width": 999_999 }),
        })
        .unwrap_err();
    assert!(err.to_string().contains("between 1 and 16384"), "{err}");
    assert_eq!(*engine.snapshot(), *before);
    // Adding a source with bad settings is rejected too (also inside batches).
    let bad = SourceDef::new("Bad", "color", serde_json::json!({ "color": "red" }));
    assert!(
        engine
            .execute(Command::Batch {
                commands: vec![Command::AddSource {
                    source: bad,
                    index: 0
                }]
            })
            .is_err()
    );
}

mod audio {
    use super::*;
    use estream_engine::audio_engine::{Block, TRACKS, TrackSink, db};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Probe {
        blocks: Arc<AtomicU64>,
        last_peak: Arc<Mutex<f32>>,
    }

    impl TrackSink for Probe {
        fn push(&mut self, _: Timestamp, tracks: &[Block; TRACKS]) {
            self.blocks.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut p) = self.last_peak.lock() {
                *p = tracks[0].peak()[0];
            }
        }
    }

    fn engine_with_tone() -> (
        Engine,
        estream_core::SourceId,
        Arc<AtomicU64>,
        Arc<Mutex<f32>>,
    ) {
        let engine = Engine::start(
            Project::new_default(320, 180, Rational::fps(30)),
            EngineConfig {
                video: false,
                ..Default::default()
            },
        )
        .unwrap();
        let (source, _) = add_fullscreen(
            &engine,
            "audio_test_tone",
            serde_json::json!({ "frequency": 1000.0, "level_db": 0.0 }),
        );
        let blocks = Arc::new(AtomicU64::new(0));
        let last_peak = Arc::new(Mutex::new(0.0));
        let canvas = engine.snapshot().canvases[0].id;
        engine.add_track_sink(
            canvas,
            Box::new(Probe {
                blocks: blocks.clone(),
                last_peak: last_peak.clone(),
            }),
        );
        (engine, source, blocks, last_peak)
    }

    #[test]
    fn audio_runs_at_100_blocks_per_second() {
        let (_engine, _, blocks, _) = engine_with_tone();
        std::thread::sleep(Duration::from_millis(200));
        let start = blocks.load(Ordering::Relaxed);
        let t = Instant::now();
        std::thread::sleep(Duration::from_secs(2));
        let produced = blocks.load(Ordering::Relaxed) - start;
        let expected = (t.elapsed().as_secs_f64() * 100.0).round() as u64;
        assert!(
            produced.abs_diff(expected) <= 2,
            "{produced} blocks, expected {expected}"
        );
    }

    #[test]
    fn volume_change_meters_and_undoes() {
        let (engine, source, _, last_peak) = engine_with_tone();
        let audio = estream_project::AudioConfig {
            volume_db: -6.0,
            ..Default::default()
        };
        engine
            .execute(Command::UpdateSourceAudio { source, audio })
            .unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let meters = engine.audio_meters();
        let meter = meters
            .sources
            .iter()
            .find(|(id, _)| *id == source)
            .expect("metered")
            .1
            .read();
        assert!(
            (db::from_gain(meter.peak[0]) + 6.0).abs() < 0.2,
            "{:?}",
            meter.peak
        );
        assert!(engine.undo().unwrap());
        std::thread::sleep(Duration::from_millis(150));
        let peak = *last_peak.lock().unwrap();
        assert!(db::from_gain(peak).abs() < 0.2, "back to 0 dB: {peak}");
    }

    #[test]
    fn switching_scene_removes_scene_audio_but_not_global() {
        let (engine, _tone, _, last_peak) = engine_with_tone();
        std::thread::sleep(Duration::from_millis(100));
        assert!(*last_peak.lock().unwrap() > 0.9);
        let canvas = engine.snapshot().canvases[0].id;
        let other = Scene::new("Other", canvas);
        let other_id = other.id;
        engine
            .execute(Command::AddScene {
                scene: other,
                index: 1,
            })
            .unwrap();
        engine
            .execute(Command::SetActiveScene {
                canvas,
                scene: Some(other_id),
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(*last_peak.lock().unwrap(), 0.0, "scene audio stopped");

        // A global source keeps playing in any scene.
        let mut global = SourceDef::new(
            "Global",
            "audio_test_tone",
            serde_json::json!({ "level_db": -12.0 }),
        );
        global.audio.global = true;
        engine
            .execute(Command::AddSource {
                source: global,
                index: 0,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let peak = db::from_gain(*last_peak.lock().unwrap());
        assert!((peak + 12.0).abs() < 0.2, "{peak}");
    }
}

#[test]
fn encoder_feed_delivers_nv12_frames_in_order() {
    let _turn = gpu_turn();
    let engine =
        Engine::start(Project::new_default(320, 180, Rational::fps(30)), config()).unwrap();
    add_fullscreen(
        &engine,
        "test_pattern",
        serde_json::json!({ "width": 320, "height": 180 }),
    );
    let canvas = engine.snapshot().canvases[0].id;
    let (tx, rx) = crossbeam_channel::bounded(16);
    let id = engine.add_video_consumer(canvas, tx);
    let start = Instant::now();
    let mut frames = Vec::new();
    while start.elapsed() < Duration::from_millis(1100) {
        if let Ok(f) = rx.recv_timeout(Duration::from_millis(100)) {
            frames.push((
                f.timestamp,
                f.width,
                f.height,
                f.y()[10 * 320 + 10],
                f.data.len(),
            ));
        }
    }
    engine.remove_video_consumer(id);
    let min = if shared_runner() { 5 } else { 28 };
    assert!(
        frames.len() >= min,
        "~30 frames in 1 s, got {}",
        frames.len()
    );
    assert!(
        frames.windows(2).all(|w| w[1].0 > w[0].0),
        "timestamps increase"
    );
    let (_, w, h, y, len) = frames[frames.len() - 1];
    assert_eq!((w, h, len), (320, 180, 320 * 180 * 3 / 2));
    // Top-left of the test pattern is a 75% white bar: Y = 16 + 219 * 0.75 ≈ 180.
    assert!((y as i32 - 180).abs() <= 3, "Y = {y}");
}

#[test]
fn ui_on_its_own_device_shows_the_engine_frames() {
    let (ui, engine_gpu) = GpuContext::new_pair(&GpuOptions::default()).expect("GPU adapter");
    let mut project = Project::new_default(64, 36, Rational::fps(60));
    let color = SourceDef::new(
        "Red",
        "color",
        serde_json::json!({ "color": Rgba8::new(255, 0, 0, 255) }),
    );
    let mut item = SceneItem::source(color.id);
    item.transform = fill(64.0, 36.0);
    project.scenes[0].items = vec![item];
    project.sources = vec![color];
    let mut core = VideoCore::new(engine_gpu, SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    let mut shown = estream_engine::UiPreview::default();
    // Several frames, so preview slots are reused after the UI's copies.
    for n in 0..6 {
        let outputs = core.render_now(Timestamp(n * 16_666_667));
        let frame = outputs[0].latest().unwrap();
        let picture = shown
            .show(&ui, &outputs[0], &frame)
            .expect("a preview each frame when the UI keeps up");
        let pixels = readback::read_texture(&ui, &picture).unwrap();
        assert_eq!(&pixels[..4], &[255, 0, 0, 255], "frame {n}");
    }
}

/// Source tracks cover every source the canvas can be heard with, not only the active scene.
#[test]
fn audio_sources_of_a_canvas_include_every_scene() {
    let mut project = Project::new_default(640, 360, Rational::fps(30));
    let canvas = project.canvases[0].id;
    let tone = |name: &str| {
        SourceDef::new(
            name,
            "audio_test_tone",
            serde_json::json!({ "frequency": 1000.0 }),
        )
    };
    let mut global = tone("Mic");
    global.audio.global = true;
    let active = tone("Music");
    let inactive = tone("Game");
    let mut monitored = tone("Cue");
    monitored.audio.monitoring = estream_project::Monitoring::MonitorOnly;
    let picture = SourceDef::new("Bars", "test_pattern", serde_json::json!({}));
    project.scenes[0].items.extend([
        SceneItem::source(active.id),
        SceneItem::source(monitored.id),
        SceneItem::source(picture.id),
    ]);
    let mut other = Scene::new("Game scene", canvas);
    other.items.push(SceneItem::source(inactive.id));
    other.items.push(SceneItem::source(active.id));
    project.scenes.push(other);
    project
        .sources
        .extend([global, active, inactive, monitored, picture]);
    let engine = Engine::start(project, config()).unwrap();
    let names: Vec<String> = engine
        .audio_sources_of(canvas)
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    assert_eq!(names, ["Mic", "Music", "Game"]);
}

/// A white 100 × 100 item with `mask` covering x 30..130 and y 10..110, over a blue background
/// item on a 160 × 120 canvas. `rotation` 90 turns it clockwise around its top-left corner, which
/// then sits at (130, 10).
fn render_masked(mask: Option<estream_project::ItemMask>, rotation: f32) -> Vec<u8> {
    let mut project = Project::new_default(160, 120, Rational::fps(30));
    let color = |name: &str, w: u32, h: u32, c: Rgba8| {
        SourceDef::new(
            name,
            "color",
            serde_json::json!({ "width": w, "height": h, "color": c }),
        )
    };
    let blue = color("Blue", 160, 120, Rgba8::new(0, 0, 255, 255));
    let white = color("White", 100, 100, Rgba8::WHITE);
    let mut item = SceneItem::source(white.id);
    item.transform.position = if rotation == 90.0 {
        Vec2::new(130.0, 10.0)
    } else {
        Vec2::new(30.0, 10.0)
    };
    item.transform.rotation = rotation;
    item.mask = mask;
    project.scenes[0].items = vec![SceneItem::source(blue.id), item];
    project.sources = vec![blue, white];
    let pixels = render_once(&project);
    // ESTREAM_MASK_PNG=<folder> writes every picture for a look.
    if let Some(dir) = std::env::var_os("ESTREAM_MASK_PNG") {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = PathBuf::from(dir).join(format!("mask-{n}.png"));
        readback::save_png(&path, 160, 120, &pixels).unwrap();
    }
    pixels
}

fn rgb(p: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * 160 + x) * 4) as usize;
    [p[i], p[i + 1], p[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3)
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLUE: [u8; 3] = [0, 0, 255];

#[track_caller]
fn assert_px(p: &[u8], x: u32, y: u32, want: [u8; 3], what: &str) {
    let got = rgb(p, x, y);
    assert!(
        near(got, want),
        "{what}: ({x}, {y}) is {got:?}, not {want:?}"
    );
}

#[test]
fn masks_cut_items_into_shapes() {
    use estream_project::{ItemMask, MaskGeometry, MaskShape};
    let plain = render_masked(None, 0.0);
    assert_px(&plain, 31, 11, WHITE, "unmasked corner");

    let circle = render_masked(Some(ItemMask::circle(1.0)), 0.0);
    assert_px(&circle, 80, 60, WHITE, "circle middle");
    assert_px(&circle, 32, 12, BLUE, "circle leaves the corner");
    assert_px(&circle, 127, 107, BLUE, "and the opposite corner");

    // Rounded rectangle with a circle cut out of it.
    let mut ring = ItemMask::rounded(20.0);
    ring.shapes.push(MaskShape::cut(MaskGeometry::Ellipse {
        center: Vec2::splat(0.5),
        radii: Vec2::splat(0.2),
    }));
    let ring = render_masked(Some(ring), 0.0);
    assert_px(&ring, 80, 60, BLUE, "cut circle");
    assert_px(&ring, 80, 13, WHITE, "between the cut and the edge");
    assert_px(&ring, 31, 11, BLUE, "rounded corner");

    // Feathered right half: the edge at x = 80 is halfway.
    let mut half = ItemMask::rounded(0.0);
    half.shapes[0].geometry = MaskGeometry::Rect {
        min: Vec2::new(0.5, -1.0),
        max: Vec2::new(2.0, 2.0),
        radius: 0.0,
    };
    half.shapes[0].feather = 20.0;
    let soft = render_masked(Some(half), 0.0);
    let edge = rgb(&soft, 80, 60);
    assert!(
        (100..235).contains(&edge[0]),
        "half blend at the edge: {edge:?}"
    );
    assert_px(&soft, 95, 60, WHITE, "inside the feather");
    assert_px(&soft, 65, 60, BLUE, "outside the feather");

    let mut inverted = ItemMask::circle(1.0);
    inverted.invert = true;
    let inverted = render_masked(Some(inverted), 0.0);
    assert_px(&inverted, 80, 60, BLUE, "inverted circle");
    assert_px(&inverted, 32, 12, WHITE, "inverted keeps the corner");

    let mut off = ItemMask::circle(1.0);
    off.enabled = false;
    assert_eq!(
        render_masked(Some(off), 0.0),
        plain,
        "a mask that's off draws the whole item"
    );

    // The mask turns with the item: turned 90° clockwise, the item's top edge is at x = 130 and
    // its top right corner lands at the bottom right.
    let mut turned = ItemMask::cut_corner();
    let rotated = render_masked(Some(turned.clone()), 90.0);
    assert_px(&rotated, 120, 100, BLUE, "cut corner, turned");
    assert_px(&rotated, 120, 20, WHITE, "unturned corner place stays");
    turned.shapes.clear();
    assert_eq!(
        render_masked(Some(turned), 90.0),
        render_masked(None, 90.0),
        "an empty mask changes nothing"
    );
}

#[test]
fn edge_effects_follow_the_shape() {
    use estream_project::{Border, BorderPlacement, Glow, GlowPlacement, ItemMask, Shadow};
    let red = Rgba8::new(255, 0, 0, 255);
    let green = Rgba8::new(0, 255, 0, 255);

    // Outside border on a circle (center (80, 60), radius 50: top at y = 10).
    let mut ringed = ItemMask::circle(1.0);
    ringed.effects.border = Some(Border {
        color: red,
        width: 4.0,
        placement: BorderPlacement::Outside,
    });
    let p = render_masked(Some(ringed), 0.0);
    assert_px(&p, 80, 8, [255, 0, 0], "border above the circle");
    assert_px(&p, 80, 30, WHITE, "inside stays");
    assert_px(&p, 80, 3, BLUE, "beyond the border");
    assert_px(&p, 33, 13, BLUE, "corner outside the border");

    // An outer glow reaches past the item's rectangle (left edge x = 30).
    let mut glowing = ItemMask::rounded(0.0);
    glowing.effects.glow = Some(Glow {
        color: red,
        size: 20.0,
        strength: 1.0,
        placement: GlowPlacement::Outside,
    });
    let p = render_masked(Some(glowing), 0.0);
    let glow = rgb(&p, 25, 60);
    assert!(glow[0] > 120, "glow outside the item: {glow:?}");
    assert!(rgb(&p, 15, 60)[0] < glow[0], "and fades");
    assert_px(&p, 4, 60, BLUE, "gone past its size");

    // Offset shadow below an item without shapes (bottom edge y = 110).
    let mut shadowed = ItemMask::default();
    shadowed.effects.shadow = Some(Shadow {
        color: Rgba8::BLACK,
        offset: Vec2::new(0.0, 6.0),
        softness: 0.0,
        opacity: 1.0,
    });
    let p = render_masked(Some(shadowed), 0.0);
    assert_px(&p, 80, 113, [0, 0, 0], "shadow under the item");
    assert_px(&p, 80, 7, BLUE, "nothing above");
    assert_px(&p, 80, 60, WHITE, "item on top");

    // A border on an item without shapes follows its rectangle.
    let mut framed = ItemMask::default();
    framed.effects.border = Some(Border {
        color: green,
        width: 4.0,
        placement: BorderPlacement::Outside,
    });
    let p = render_masked(Some(framed), 0.0);
    assert_px(&p, 28, 60, [0, 255, 0], "border left of the item");
    assert_px(&p, 80, 112, [0, 255, 0], "border below the item");
    assert_px(&p, 32, 60, WHITE, "item inside");
    assert_px(&p, 24, 60, BLUE, "beyond the border");
}
