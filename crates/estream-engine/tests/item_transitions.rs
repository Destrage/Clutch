#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Item transitions rendered headless at exact timestamps: items moving to their place in the
//! next scene (with shape morphs), and show and hide animations.

use estream_core::{Alignment, BoundsMode, ItemTransform, Rational, Rgba8, Timestamp};
use estream_engine::graph::{ActiveTransition, ItemAnimationRun, RenderGraph, ViewState};
use estream_engine::{
    Command, Engine, EngineConfig, SourceRegistry, VideoCore, command::ItemPatch,
};
use estream_gpu::{GpuContext, GpuOptions, readback};
use estream_project::{
    AnimationKind, Direction, Ease, ItemAnimation, ItemMask, MoveSettings, Project, Scene,
    SceneItem, SourceDef, Transition, TransitionKind,
};
use glam::Vec2;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const W: u32 = 128;
const H: u32 = 72;
const START: Timestamp = Timestamp(1_000_000_000);

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

/// A box of `size` with its top-left corner at `at` (the source is stretched into it).
fn placed(at: Vec2, size: Vec2) -> ItemTransform {
    ItemTransform {
        position: at,
        bounds_mode: BoundsMode::Stretch,
        bounds: size,
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    }
}

fn white() -> SourceDef {
    SourceDef::new(
        "Camera",
        "color",
        serde_json::json!({ "width": 16, "height": 16, "color": Rgba8::new(255, 255, 255, 255) }),
    )
}

/// Scene A (on program first) holding `a`, scene B holding `b`, both on a black canvas.
fn two_scenes(
    source: SourceDef,
    a: SceneItem,
    b: SceneItem,
) -> (Project, [estream_core::SceneId; 2]) {
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let canvas = p.canvases[0].id;
    p.scenes[0].items = vec![a];
    let mut scene_b = Scene::new("B", canvas);
    scene_b.items = vec![b];
    let ids = [p.scenes[0].id, scene_b.id];
    p.scenes.push(scene_b);
    p.sources = vec![source];
    (p, ids)
}

/// Renders the switch from A to B (1000 ms Fade, Linear moves) and reads frames from it.
struct Switch {
    gpu: GpuContext,
    core: VideoCore,
}

impl Switch {
    fn new(mut project: Project, [a, b]: [estream_core::SceneId; 2]) -> Self {
        let canvas = project.canvases[0].id;
        project.canvases[0].active_scene = Some(b);
        let mut view = ViewState::default();
        view.transitions.insert(
            canvas,
            ActiveTransition {
                from: Some(a),
                transition: Transition {
                    kind: TransitionKind::Fade,
                    duration_ms: 1000,
                    moves: Some(MoveSettings {
                        ease: Ease::Linear,
                        curved: false,
                    }),
                },
                start: START,
            },
        );
        Self::with_view(project, view)
    }

    fn with_view(project: Project, view: ViewState) -> Self {
        let gpu = gpu();
        let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
        core.sync(
            Arc::new(RenderGraph::build_with(&project, 1, &view, START)),
            START,
        );
        Self { gpu, core }
    }

    /// The frame `ms` after the start, as sRGB pixels.
    fn at(&mut self, ms: u64) -> Vec<u8> {
        let outputs = self.core.render_now(Timestamp(START.0 + ms * 1_000_000));
        let frame = outputs[0].latest().unwrap();
        readback::read_texture(&self.gpu, frame.preview).unwrap()
    }
}

/// Red channel of pixel (x, y): 255 where the white camera is, 0 on the black canvas.
fn lum(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * W + x) * 4) as usize]
}

#[test]
fn a_matching_item_moves_whole_to_its_new_place() {
    let cam = white();
    let mut a = SceneItem::source(cam.id);
    a.transform = placed(Vec2::new(0.0, 0.0), Vec2::new(16.0, 16.0));
    let mut b = SceneItem::source(cam.id);
    b.transform = placed(Vec2::new(96.0, 40.0), Vec2::new(32.0, 32.0));
    let (p, scenes) = two_scenes(cam, a, b);
    let mut s = Switch::new(p, scenes);

    let start = s.at(0);
    assert_eq!(lum(&start, 8, 8), 255, "at its old place");
    // Halfway: centers (8, 8) and (112, 56) → (60, 32); size √(16·32) ≈ 22.6.
    let mid = s.at(500);
    assert_eq!(
        lum(&mid, 60, 32),
        255,
        "one whole camera halfway, not a crossfade"
    );
    assert_eq!(lum(&mid, 8, 8), 0, "nothing left at the old place");
    assert_eq!(lum(&mid, 112, 56), 0, "nothing yet at the new place");
    assert_eq!(lum(&mid, 60, 45), 0, "smaller than the new size");
    let end = s.at(1000);
    assert_eq!(lum(&end, 112, 56), 255);
    assert_eq!(lum(&end, 8, 8), 0);
}

#[test]
fn without_moves_the_item_crossfades() {
    let cam = white();
    let mut a = SceneItem::source(cam.id);
    a.transform = placed(Vec2::new(0.0, 0.0), Vec2::new(16.0, 16.0));
    let mut b = SceneItem::source(cam.id);
    b.transform = placed(Vec2::new(96.0, 40.0), Vec2::new(32.0, 32.0));
    b.animations.moves = false;
    let (p, scenes) = two_scenes(cam, a, b);
    let mut s = Switch::new(p, scenes);
    let mid = s.at(500);
    // 50 % in linear light is sRGB 188.
    assert!(lum(&mid, 8, 8).abs_diff(188) <= 3, "{}", lum(&mid, 8, 8));
    assert!(lum(&mid, 112, 56).abs_diff(188) <= 3);
    assert_eq!(lum(&mid, 60, 32), 0);
}

#[test]
fn a_rectangle_morphs_into_a_circle_with_one_outline() {
    let cam = white();
    let place = placed(Vec2::new(32.0, 4.0), Vec2::new(64.0, 64.0));
    let mut a = SceneItem::source(cam.id);
    a.transform = place;
    let mut b = SceneItem::source(cam.id);
    b.transform = place;
    b.mask = Some(ItemMask::circle(1.0));
    let (p, scenes) = two_scenes(cam, a, b);
    let mut s = Switch::new(p, scenes);
    // Item-local (8, 8): inside the square, 1.9 px outside the circle; (3, 3) is 9 px out.
    let (near_corner, corner) = ((32 + 8, 4 + 8), (32 + 3, 4 + 3));
    let start = s.at(0);
    assert_eq!(lum(&start, near_corner.0, near_corner.1), 255);
    assert_eq!(lum(&start, corner.0, corner.1), 255);
    let mid = s.at(500);
    assert!(
        lum(&mid, near_corner.0, near_corner.1) > 240,
        "still inside halfway"
    );
    assert!(
        lum(&mid, corner.0, corner.1) < 15,
        "the corner has already gone"
    );
    assert_eq!(lum(&mid, 64, 36), 255, "the middle stays");
    let end = s.at(1000);
    assert!(
        lum(&end, near_corner.0, near_corner.1) < 15,
        "the circle at the end"
    );
    assert_eq!(lum(&end, 64, 36), 255);
}

#[test]
fn a_circle_grows_back_into_the_whole_item() {
    let cam = white();
    let place = placed(Vec2::new(32.0, 4.0), Vec2::new(64.0, 64.0));
    let mut a = SceneItem::source(cam.id);
    a.transform = place;
    a.mask = Some(ItemMask::circle(1.0));
    let mut b = SceneItem::source(cam.id);
    b.transform = place;
    let (p, scenes) = two_scenes(cam, a, b);
    let mut s = Switch::new(p, scenes);
    let corner = (32 + 3, 4 + 3);
    assert!(lum(&s.at(0), corner.0, corner.1) < 15);
    assert!(lum(&s.at(500), corner.0, corner.1) < 15);
    assert_eq!(lum(&s.at(1000), corner.0, corner.1), 255);
}

/// One white 64×64 item in the middle of the canvas playing `kind` (Linear), shown or hidden.
fn animating(kind: AnimationKind, showing: bool) -> Switch {
    let cam = white();
    let mut item = SceneItem::source(cam.id);
    item.transform = placed(Vec2::new(32.0, 4.0), Vec2::new(64.0, 64.0));
    item.visible = showing;
    let id = item.id;
    let mut p = Project::new_default(W, H, Rational::fps(30));
    p.scenes[0].items = vec![item];
    p.sources = vec![cam];
    let mut view = ViewState::default();
    view.item_animations.insert(
        id,
        ItemAnimationRun::new(
            showing,
            ItemAnimation {
                kind,
                duration_ms: 1000,
                ease: Ease::Linear,
            },
            START,
        ),
    );
    Switch::with_view(p, view)
}

#[test]
fn zoom_grows_from_the_center() {
    let mut s = animating(AnimationKind::Zoom, true);
    let half = s.at(500); // 32×32 around (64, 36)
    assert_eq!(lum(&half, 64, 36), 255);
    assert_eq!(lum(&half, 36, 8), 0, "the corners aren't there yet");
    assert_eq!(lum(&s.at(1000), 36, 8), 255);
}

#[test]
fn wipe_reveals_in_its_direction_and_hides_ahead_of_it() {
    let right = AnimationKind::Wipe {
        direction: Direction::Right,
    };
    let mut show = animating(right, true);
    let half = show.at(500);
    assert_eq!(lum(&half, 40, 36), 255, "left half revealed");
    assert_eq!(lum(&half, 90, 36), 0);
    let mut hide = animating(right, false);
    let half = hide.at(500);
    assert_eq!(lum(&half, 40, 36), 0, "left half wiped away");
    assert_eq!(lum(&half, 90, 36), 255);
    assert_eq!(lum(&hide.at(1000), 90, 36), 0, "gone at the end");
}

#[test]
fn slide_comes_in_from_the_edge() {
    let mut s = animating(
        AnimationKind::Slide {
            edge: Direction::Left,
        },
        true,
    );
    let first = s.at(0);
    assert!(
        (0..W).all(|x| lum(&first, x, 36) == 0),
        "fully off the canvas"
    );
    assert_eq!(lum(&s.at(1000), 64, 36), 255);
}

#[test]
fn hiding_with_the_eye_button_animates_and_reverses_without_a_jump() {
    let cam = white();
    let mut item = SceneItem::source(cam.id);
    item.animations.hide = Some(ItemAnimation {
        kind: AnimationKind::Fade,
        duration_ms: 2000,
        ease: Ease::Linear,
    });
    item.animations.show = item.animations.hide;
    let id = item.id;
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let scene = p.scenes[0].id;
    p.scenes[0].items = vec![item];
    p.sources = vec![cam];
    let engine = Engine::start(
        p,
        EngineConfig {
            gpu: Some(gpu()),
            audio: false,
            ..Default::default()
        },
    )
    .unwrap();
    let visible = |v: bool| Command::UpdateItem {
        scene,
        item: id,
        patch: ItemPatch {
            visible: Some(v),
            ..Default::default()
        },
    };
    engine.execute(visible(false)).unwrap();
    let hide = engine.render_graph().canvases[0].items[0]
        .run
        .expect("hiding");
    assert!(!hide.showing);
    std::thread::sleep(Duration::from_millis(100));
    let before = engine.clock().now();
    engine.execute(visible(true)).unwrap();
    let after = engine.clock().now();
    assert!(hide.end() > after, "still fading out when shown again");
    let graph = engine.render_graph();
    let show = graph.canvases[0].items[0].run.expect("showing");
    assert!(show.showing);
    // The fade-in starts while the command runs, at the presence the fade-out had then (no
    // jump). Checked from the runs' own times, so a slow machine can't skew it, and right
    // after the engine starts (its clock near 0), where a back-dated start would underflow.
    assert!(show.start >= before && show.start <= after);
    let gap = (show.presence(show.start) - hide.presence(show.start)).abs();
    assert!(gap < 1e-3, "jumped by {gap}");
    assert!(show.from > 0.0, "carries on midway");
    assert!(show.end().0 < after.0 + 2_000_000_000, "a shorter way back");

    // Finished animations leave the graph.
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until && engine.render_graph().canvases[0].items[0].run.is_some() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(engine.render_graph().canvases[0].items[0].run.is_none());
}
