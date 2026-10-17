#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Video filters on real canvases (headless) and through the command bus.

use estream_core::{Alignment, BoundsMode, ItemTransform, Rational, Rgba8, SourceId, Timestamp};
use estream_engine::graph::RenderGraph;
use estream_engine::{Command, Engine, EngineConfig, SourceRegistry, VideoCore};
use estream_gpu::{GpuContext, GpuOptions, readback};
use estream_project::{FilterDef, Project, SceneItem, SourceDef};
use glam::Vec2;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const W: u32 = 32;
const H: u32 = 18;

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

fn filter(kind: &str, settings: serde_json::Value) -> FilterDef {
    FilterDef {
        id: format!("{kind}-1"),
        kind: kind.into(),
        enabled: true,
        settings,
    }
}

/// A canvas filled by one solid-color source; returns the project and the source id.
fn solid(color: Rgba8, filters: Vec<FilterDef>) -> (Project, SourceId) {
    let mut p = Project::new_default(W, H, Rational::fps(30));
    let mut src = SourceDef::new(
        "Solid",
        "color",
        serde_json::json!({ "width": 16, "height": 16, "color": color }),
    );
    src.filters = filters;
    let mut item = SceneItem::source(src.id);
    item.transform = ItemTransform {
        bounds_mode: BoundsMode::Stretch,
        bounds: Vec2::new(W as f32, H as f32),
        bounds_alignment: Alignment::TopLeft,
        ..Default::default()
    };
    p.scenes[0].items = vec![item];
    let id = src.id;
    p.sources = vec![src];
    (p, id)
}

fn center(gpu: &GpuContext, core: &mut VideoCore, t: u64) -> [u8; 4] {
    let outputs = core.render_now(Timestamp(t));
    let frame = outputs[0].latest().unwrap();
    let pixels = readback::read_texture(gpu, frame.preview).unwrap();
    let i = ((H / 2 * W + W / 2) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 4)
}

const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);

#[test]
fn chroma_key_shows_the_background_and_disabling_restores() {
    let gpu = gpu();
    let (mut project, _) = solid(GREEN, vec![filter("chroma_key", serde_json::json!({}))]);
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    assert!(
        near(center(&gpu, &mut core, 0), [0, 0, 0, 255]),
        "keyed to the black background"
    );

    project.sources[0].filters[0].enabled = false;
    core.sync(Arc::new(RenderGraph::build(&project, 2)), Timestamp::ZERO);
    assert!(
        near(center(&gpu, &mut core, 1), [0, 255, 0, 255]),
        "unfiltered again"
    );
}

#[test]
fn crop_pad_resizes_the_drawn_source() {
    let gpu = gpu();
    // Pad 16 px on the right: the 16 px source becomes 32 px wide; stretched to the canvas,
    // its right half is transparent.
    let (project, _) = solid(
        GREEN,
        vec![filter("crop_pad", serde_json::json!({ "right": -16.0 }))],
    );
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    let outputs = core.render_now(Timestamp::ZERO);
    let pixels = readback::read_texture(&gpu, outputs[0].latest().unwrap().preview).unwrap();
    let px = |x: u32| -> [u8; 4] {
        let i = ((H / 2 * W + x) * 4) as usize;
        pixels[i..i + 4].try_into().unwrap()
    };
    assert!(near(px(4), [0, 255, 0, 255]), "left: {:?}", px(4));
    assert!(
        near(px(W - 4), [0, 0, 0, 255]),
        "right is padding: {:?}",
        px(W - 4)
    );
}

#[test]
fn lut_file_loads_off_thread_then_applies() {
    let dir = std::env::temp_dir().join(format!("estream-lut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("invert.cube");
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{} {} {}\n", 1 - r, 1 - g, 1 - b));
            }
        }
    }
    std::fs::write(&path, cube).unwrap();

    let gpu = gpu();
    let (project, _) = solid(
        Rgba8::new(255, 0, 0, 255),
        vec![filter(
            "lut",
            serde_json::json!({ "file": path.to_string_lossy() }),
        )],
    );
    let mut core = VideoCore::new(gpu.clone(), SourceRegistry::with_builtins());
    core.sync(Arc::new(RenderGraph::build(&project, 1)), Timestamp::ZERO);
    // The first frames may still be unfiltered while the file loads.
    let until = Instant::now() + Duration::from_secs(5);
    let mut t = 0;
    let mut px = center(&gpu, &mut core, t);
    while !near(px, [0, 255, 255, 255]) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
        t += 1;
        px = center(&gpu, &mut core, t);
    }
    assert!(near(px, [0, 255, 255, 255]), "red inverted to cyan: {px:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn engine(project: Project) -> Engine {
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

#[test]
fn filter_edits_are_undoable_coalesced_and_validated() {
    let (project, source) = solid(GREEN, Vec::new());
    let engine = engine(project);
    // A slider drag: several quick changes become one undo step.
    for radius in [2.0, 4.0, 8.0] {
        engine
            .execute(Command::SetSourceFilters {
                source,
                filters: vec![filter("blur", serde_json::json!({ "radius": radius }))],
            })
            .unwrap();
    }
    assert_eq!(
        engine.snapshot().sources[0].filters[0].settings["radius"],
        8.0
    );
    engine.undo().unwrap();
    assert!(
        engine.snapshot().sources[0].filters.is_empty(),
        "one step back to none"
    );

    let unknown = engine.execute(Command::SetSourceFilters {
        source,
        filters: vec![filter("sparkle", serde_json::json!({}))],
    });
    assert!(unknown.unwrap_err().to_string().contains("unknown filter"));

    let broken = engine.execute(Command::SetSourceFilters {
        source,
        filters: vec![filter(
            "custom_shader",
            serde_json::json!({ "code": "fn effect(uv: vec2<f32>) -> vec4<f32> {\n  return nope;\n}" }),
        )],
    });
    assert!(broken.unwrap_err().to_string().contains("Line 2"));
    assert!(engine.snapshot().sources[0].filters.is_empty());
}
