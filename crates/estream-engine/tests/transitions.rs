#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Scene transitions and the studio Preview, rendered headless at exact timestamps.

use estream_core::{Alignment, BoundsMode, ItemTransform, Rational, Rgba8, Timestamp};
use estream_engine::graph::{ActiveTransition, RenderGraph, ViewState};
use estream_engine::{Command, Engine, EngineConfig, SourceRegistry, VideoCore};
use estream_gpu::{GpuContext, GpuOptions, readback};
use estream_project::{Project, Scene, SceneItem, SourceDef, Transition, TransitionKind};
use glam::Vec2;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const W: u32 = 64;
const H: u32 = 36;

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

fn fill() -> ItemTransform {
    ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: Vec2::new(W as f32, H as f32),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    }
}

/// A project with scene A (red) on program and scene B (blue).
fn red_and_blue() -> (Project, estream_core::SceneId, estream_core::SceneId) {
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let canvas = p.canvases[0].id;
    let red = SourceDef::new(
        "Red",
        "color",
        serde_json::json!({ "width": 16, "height": 16, "color": Rgba8::new(255, 0, 0, 255) }),
    );
    let blue = SourceDef::new(
        "Blue",
        "color",
        serde_json::json!({ "width": 16, "height": 16, "color": Rgba8::new(0, 0, 255, 255) }),
    );
    let a = p.scenes[0].id;
    let mut red_item = SceneItem::source(red.id);
    red_item.transform = fill();
    p.scenes[0].items = vec![red_item];
    let mut b = Scene::new("B", canvas);
    let mut blue_item = SceneItem::source(blue.id);
    blue_item.transform = fill();
    b.items = vec![blue_item];
    let b_id = b.id;
    p.scenes.push(b);
    p.sources = vec![red, blue];
    (p, a, b_id)
}

/// Center pixel (sRGB 8-bit) of the latest frame of an output.
fn center(gpu: &GpuContext, output: &estream_engine::CanvasOutput) -> [u8; 4] {
    let frame = output.latest().unwrap();
    let pixels = readback::read_texture(gpu, frame.preview).unwrap();
    let i = ((H / 2 * W + W / 2) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn fade_blends_in_linear_light_and_ends_on_the_new_scene() {
    let (mut project, a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    project.canvases[0].active_scene = Some(b); // program is already B, A is leaving
    let start = Timestamp(1_000_000_000);
    let mut view = ViewState::default();
    view.transitions.insert(
        canvas,
        ActiveTransition {
            from: Some(a),
            transition: Transition {
                kind: TransitionKind::Fade,
                duration_ms: 1000,
            },
            start,
        },
    );
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(
        Arc::new(RenderGraph::build_with(&project, 1, &view, start)),
        start,
    );

    let at = |core: &mut VideoCore, ms: u64| {
        let outputs = core.render_now(Timestamp(start.0 + ms * 1_000_000));
        center(&gpu, &outputs[0])
    };
    assert!(near(at(&mut core, 0), RED), "start shows the old scene");
    // 50 % in linear light = 0.5 → sRGB 188 for both channels.
    let mid = at(&mut core, 500);
    assert!(near(mid, [188, 0, 188, 255]), "{mid:?}");
    assert!(near(at(&mut core, 1000), BLUE), "end shows the new scene");
    assert!(near(at(&mut core, 5000), BLUE));
}

#[test]
fn slide_shows_both_scenes_halfway() {
    let (mut project, a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    project.canvases[0].active_scene = Some(b);
    let mut view = ViewState::default();
    view.transitions.insert(
        canvas,
        ActiveTransition {
            from: Some(a),
            transition: Transition {
                kind: TransitionKind::Slide {
                    direction: estream_project::Direction::Left,
                },
                duration_ms: 1000,
            },
            start: Timestamp::ZERO,
        },
    );
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(
        Arc::new(RenderGraph::build_with(&project, 1, &view, Timestamp::ZERO)),
        Timestamp::ZERO,
    );
    let outputs = core.render_now(Timestamp(500_000_000));
    let frame = outputs[0].latest().unwrap();
    let pixels = readback::read_texture(&gpu, frame.preview).unwrap();
    let px = |x: u32| -> [u8; 4] {
        let i = ((H / 2 * W + x) * 4) as usize;
        pixels[i..i + 4].try_into().unwrap()
    };
    assert!(near(px(4), RED), "left half still old: {:?}", px(4));
    assert!(
        near(px(W - 4), BLUE),
        "right half already new: {:?}",
        px(W - 4)
    );
}

#[test]
fn studio_preview_renders_its_own_scene() {
    let (project, _a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    let mut view = ViewState::default();
    view.preview.insert(canvas, b);
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(
        Arc::new(RenderGraph::build_with(&project, 1, &view, Timestamp::ZERO)),
        Timestamp::ZERO,
    );
    let outputs = core.render_now(Timestamp::ZERO);
    let previews = core.preview_outputs();
    assert_eq!(previews.len(), 1);
    assert!(near(center(&gpu, &outputs[0]), RED), "program unchanged");
    assert!(near(center(&gpu, &previews[0]), BLUE), "preview shows B");

    // Studio mode off: the preview render goes away.
    core.sync(Arc::new(RenderGraph::build(&project, 2)), Timestamp::ZERO);
    assert!(core.preview_outputs().is_empty());
}

#[test]
fn graph_keeps_both_scenes_audio_until_the_transition_ends() {
    let (mut project, a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    project.canvases[0].active_scene = Some(b);
    let mut view = ViewState::default();
    view.transitions.insert(
        canvas,
        ActiveTransition {
            from: Some(a),
            transition: Transition {
                kind: TransitionKind::Fade,
                duration_ms: 300,
            },
            start: Timestamp::ZERO,
        },
    );
    let during = RenderGraph::build_with(&project, 1, &view, Timestamp(100_000_000));
    assert!(during.canvases[0].transition.is_some());
    assert_eq!(during.audio.nodes.len(), 2, "both scenes' sources mix");
    let after = RenderGraph::build_with(&project, 2, &view, Timestamp(300_000_000));
    assert!(after.canvases[0].transition.is_none());
    assert_eq!(after.audio.nodes.len(), 1, "only the new scene");
}

fn engine_with(project: Project) -> Engine {
    Engine::start(
        project,
        EngineConfig {
            gpu: Some(gpu()),
            audio: false,
            ..Default::default()
        },
    )
    .unwrap()
}

fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn engine_transition_switches_program_and_expires() {
    let (project, _a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    let engine = engine_with(project);
    engine
        .transition_to(
            canvas,
            b,
            Some(Transition {
                kind: TransitionKind::Fade,
                duration_ms: 400,
            }),
        )
        .unwrap();
    assert_eq!(engine.snapshot().canvases[0].active_scene, Some(b));
    assert!(engine.is_transitioning(canvas));
    // Mid-transition frames are a mix; afterwards only blue.
    let gpu = gpu();
    let output = engine.canvas_output(canvas).unwrap();
    assert!(wait_for(
        || output.latest().is_some(),
        Duration::from_secs(2)
    ));
    assert!(wait_for(
        || {
            let c = center(&gpu, &output);
            c[0] > 40 && c[2] > 40
        },
        Duration::from_secs(2)
    ));
    assert!(wait_for(
        || !engine.is_transitioning(canvas),
        Duration::from_secs(2)
    ));
    assert!(wait_for(
        || near(center(&gpu, &output), BLUE),
        Duration::from_secs(2)
    ));
}

#[test]
fn plain_scene_change_cuts_a_running_transition() {
    let (project, a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    let engine = engine_with(project);
    engine
        .transition_to(
            canvas,
            b,
            Some(Transition {
                kind: TransitionKind::Fade,
                duration_ms: 5_000,
            }),
        )
        .unwrap();
    assert!(engine.is_transitioning(canvas));
    engine
        .execute(Command::SetActiveScene {
            canvas,
            scene: Some(a),
        })
        .unwrap();
    assert!(!engine.is_transitioning(canvas), "a direct change is a cut");
}

#[test]
fn cut_transition_is_instant_and_preview_follows_requests() {
    let (project, _a, b) = red_and_blue();
    let canvas = project.canvases[0].id;
    let engine = engine_with(project);
    engine
        .transition_to(canvas, b, Some(Transition::CUT))
        .unwrap();
    assert!(!engine.is_transitioning(canvas));

    engine.set_preview_scene(canvas, Some(b));
    assert!(wait_for(
        || engine.preview_output(canvas).is_some(),
        Duration::from_secs(2)
    ));
    engine.set_preview_scene(canvas, None);
    assert!(wait_for(
        || engine.preview_output(canvas).is_none(),
        Duration::from_secs(2)
    ));
}

#[test]
fn transition_setting_is_undoable_and_shaders_are_validated() {
    let (project, _, _) = red_and_blue();
    let engine = engine_with(project);
    let wipe = Transition {
        kind: TransitionKind::Wipe {
            direction: estream_project::Direction::Down,
            softness: 0.1,
        },
        duration_ms: 800,
    };
    engine
        .execute(Command::SetTransition {
            transition: wipe.clone(),
        })
        .unwrap();
    assert_eq!(engine.snapshot().transition, wipe);
    engine.undo().unwrap();
    assert_eq!(engine.snapshot().transition, Transition::default());

    let broken = Transition {
        kind: TransitionKind::Shader {
            code: "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {\n  return nope;\n}"
                .into(),
        },
        duration_ms: 500,
    };
    let err = engine
        .execute(Command::SetTransition { transition: broken })
        .unwrap_err();
    assert!(err.to_string().contains("Line 2"), "{err}");
    assert_eq!(engine.snapshot().transition, Transition::default());
}
