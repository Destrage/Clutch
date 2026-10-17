//! The `media` source on real canvases and through the engine's mixer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::gpu;
use estream_core::{Alignment, BoundsMode, ItemTransform, Rational, Timestamp};
use estream_engine::graph::RenderGraph;
use estream_engine::{Command, Engine, EngineConfig, SourceRegistry, VideoCore};
use estream_gpu::readback;
use estream_project::{Project, SceneItem, SourceDef};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 64;
const H: u32 = 36;

fn ffmpeg_cli() -> Option<PathBuf> {
    let exe = PathBuf::from(option_env!("FFMPEG_DIR")?)
        .join("bin")
        .join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
    exe.exists().then_some(exe)
}

fn generate(name: &str, args: &[&str]) -> Option<PathBuf> {
    let exe = ffmpeg_cli()?;
    let dir = std::env::temp_dir().join(format!("estream-media-src-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(name);
    if !out.exists() {
        let ok = Process::new(exe)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(&out)
            .status()
            .unwrap()
            .success();
        assert!(ok, "ffmpeg failed for {name}");
    }
    Some(out)
}

fn registry() -> SourceRegistry {
    let mut r = SourceRegistry::with_builtins();
    estream_media::media_source::register(&mut r);
    r
}

fn project_with(file: &Path) -> (Project, estream_core::SourceId) {
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let mut src = SourceDef::new(
        "Clip",
        "media",
        serde_json::json!({ "file": file.to_string_lossy() }),
    );
    src.audio.global = true;
    let mut item = SceneItem::source(src.id);
    item.transform = ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: [W as f32, H as f32].into(),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    };
    p.scenes[0].items = vec![item];
    let id = src.id;
    p.sources = vec![src];
    (p, id)
}

#[test]
fn alpha_video_composites_over_the_background() {
    let Some(file) = generate(
        "half-red.mov",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red@0.5:s=64x64:d=2,format=argb",
            "-c:v",
            "qtrle",
            "-pix_fmt",
            "argb",
        ],
    ) else {
        return;
    };
    let (project, _) = project_with(&file);
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), registry());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    let until = Instant::now() + Duration::from_secs(3);
    let mut t = 0;
    let center = loop {
        let outputs = core.render_now(Timestamp(t));
        let pixels = readback::read_texture(&gpu, outputs[0].latest().unwrap().preview).unwrap();
        let i = ((H / 2 * W + W / 2) * 4) as usize;
        let px: [u8; 4] = pixels[i..i + 4].try_into().unwrap();
        if px[0] > 50 || Instant::now() > until {
            break px;
        }
        std::thread::sleep(Duration::from_millis(20));
        t += 33_000_000;
    };
    // Half-transparent red over black: linear 0.5 → sRGB ~188.
    assert!(center[0].abs_diff(188) <= 6 && center[1] < 5, "{center:?}");
}

#[test]
fn stinger_overlays_and_switches_at_the_point() {
    use estream_engine::graph::{ActiveTransition, ViewState};
    use estream_project::{Scene, Transition, TransitionKind};
    let Some(stinger) = generate(
        "stinger.mov",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=green@0.5:s=64x36:d=3,format=argb",
            "-c:v",
            "qtrle",
            "-pix_fmt",
            "argb",
        ],
    ) else {
        return;
    };
    // Scene A red (program before), scene B blue (program after).
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let canvas = p.canvases[0].id;
    let solid = |name: &str, rgb: [u8; 3]| {
        SourceDef::new(
            name,
            "color",
            serde_json::json!({ "width": 8, "height": 8,
                "color": { "r": rgb[0], "g": rgb[1], "b": rgb[2], "a": 255 } }),
        )
    };
    let (red, blue) = (solid("Red", [255, 0, 0]), solid("Blue", [0, 0, 255]));
    let fill = |id| {
        let mut item = SceneItem::source(id);
        item.transform = ItemTransform {
            bounds_mode: BoundsMode::Stretch,
            bounds: [W as f32, H as f32].into(),
            bounds_alignment: Alignment::TopLeft,
            ..Default::default()
        };
        item
    };
    let a = p.scenes[0].id;
    p.scenes[0].items = vec![fill(red.id)];
    let mut b = Scene::new("B", canvas);
    b.items = vec![fill(blue.id)];
    p.canvases[0].active_scene = Some(b.id);
    p.scenes.push(b);
    p.sources = vec![red, blue];

    let start = Timestamp(1_000_000_000);
    let mut view = ViewState::default();
    view.transitions.insert(
        canvas,
        ActiveTransition {
            from: Some(a),
            transition: Transition {
                kind: TransitionKind::Stinger {
                    file: stinger.to_string_lossy().into_owned(),
                    point_ms: 500,
                },
                duration_ms: 2000,
            },
            start,
        },
    );
    let gpu = gpu();
    let mut core = VideoCore::new(gpu.clone(), registry());
    core.sync(
        Arc::new(RenderGraph::build_with(&p, 1, &view, start)),
        start,
    );
    let at = |core: &mut VideoCore, ms: u64| -> [u8; 4] {
        let outputs = core.render_now(Timestamp(start.0 + ms * 1_000_000));
        let pixels = readback::read_texture(&gpu, outputs[0].latest().unwrap().preview).unwrap();
        let i = ((H / 2 * W + W / 2) * 4) as usize;
        pixels[i..i + 4].try_into().unwrap()
    };
    // Wait for the stinger's first frame (decoding runs on its own thread).
    let until = Instant::now() + Duration::from_secs(3);
    let mut before = at(&mut core, 100);
    while before[1] < 50 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
        before = at(&mut core, 100);
    }
    eprintln!("before the point: {before:?}");
    assert!(before[1] > 50, "stinger drawn on top: {before:?}");
    assert!(
        before[0] > 100 && before[2] < 30,
        "old scene (red) under it: {before:?}"
    );
    let after = at(&mut core, 700);
    eprintln!("after the point: {after:?}");
    assert!(
        after[1] > 50 && after[2] > 100 && after[0] < 30,
        "new scene (blue) under it: {after:?}"
    );
    let done = at(&mut core, 2500);
    assert!(
        done[2] > 200 && done[1] < 10,
        "after the transition only the new scene: {done:?}"
    );
}

#[test]
fn video_and_audio_sides_share_one_player_through_the_engine() {
    let Some(file) = generate(
        "tone.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=30:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
            "-c:v",
            "libopenh264",
            "-c:a",
            "aac",
            "-shortest",
        ],
    ) else {
        return;
    };
    let (project, source) = project_with(&file);
    let engine = Engine::start(
        project,
        EngineConfig {
            gpu: Some(gpu()),
            registry: registry(),
            ..Default::default()
        },
    )
    .unwrap();
    // The audio side delivers the tone into the mixer.
    let until = Instant::now() + Duration::from_secs(3);
    let mut peak = 0.0f32;
    while Instant::now() < until && peak < 0.05 {
        std::thread::sleep(Duration::from_millis(50));
        let meters = engine.audio_meters();
        if let Some((_, m)) = meters.sources.iter().find(|(id, _)| *id == source) {
            peak = peak.max(m.read().peak[0]);
        }
    }
    // ffmpeg's sine source is 1/8 full scale; the mixer's pan law lowers it a little more.
    assert!(peak > 0.05, "tone reaches the mixer: {peak}");
    // One player serves both sides, and it is playing.
    let player = estream_media::player::get(&source.to_string()).expect("shared player");
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until && player.sequence() <= 5 {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(player.sequence() > 5, "video frames advance");
    assert_eq!(player.status().item, "tone.mp4");

    // Settings changes reach the shared player (no file → nothing to play).
    engine
        .execute(Command::SetSourceSettings {
            source,
            settings: serde_json::json!({ "file": "" }),
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until && player.status().count != 0 {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(player.status().count, 0);
}
