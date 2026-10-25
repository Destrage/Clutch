//! GPU frame delivery through a `GpuExport`, with a fake export (no platform interop).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_core::Rational;
use estream_engine::gpu::{GpuContext, GpuOptions, Nv12Target};
use estream_engine::{Engine, EngineConfig, ExportPlanes, GpuExport, PlanesHandle, VideoFrame};
use estream_project::Project;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct FakeExport {
    works: bool,
    ready: AtomicU64,
}

impl GpuExport for FakeExport {
    fn create_planes(&self, gpu: &GpuContext, width: u32, height: u32) -> Option<ExportPlanes> {
        self.works.then(|| ExportPlanes {
            target: Nv12Target::new(gpu, width, height),
            handle: Arc::new(()),
        })
    }

    fn wait_released(&self, _: &GpuContext, _: &PlanesHandle) {}

    fn signal_ready(&self, _: &GpuContext) -> u64 {
        self.ready.fetch_add(1, Ordering::Relaxed) + 1
    }
}

fn engine(works: bool) -> Engine {
    Engine::start(
        Project::new_default(64, 36, Rational::fps(30)),
        EngineConfig {
            gpu: Some(GpuContext::new_headless(&GpuOptions::default()).unwrap()),
            gpu_export: Some(Arc::new(FakeExport {
                works,
                ready: AtomicU64::new(0),
            })),
            audio: false,
            ..Default::default()
        },
    )
    .unwrap()
}

fn recv(rx: &crossbeam_channel::Receiver<VideoFrame>) -> Option<VideoFrame> {
    rx.recv_timeout(Duration::from_secs(5)).ok()
}

#[test]
fn gpu_consumers_get_gpu_frames_in_order() {
    let engine = engine(true);
    let canvas = engine.snapshot().canvases[0].id;
    let (tx, rx) = crossbeam_channel::bounded(16);
    let (id, wants) = engine.video_tap().add_gpu(canvas, tx);
    let mut last = 0;
    for _ in 0..10 {
        let f = recv(&rx).expect("no GPU frame");
        let gpu = f.gpu.as_ref().expect("a CPU frame for a GPU consumer");
        assert!(f.data.is_empty());
        assert!(gpu.ready > last, "fence values must increase");
        last = gpu.ready;
    }
    assert!(wants.load(Ordering::Relaxed));
    engine.video_tap().remove(id);
}

#[test]
fn held_frames_drop_new_ones_instead_of_blocking() {
    let engine = engine(true);
    let canvas = engine.snapshot().canvases[0].id;
    let (tx, rx) = crossbeam_channel::bounded(64);
    let (id, _) = engine.video_tap().add_gpu(canvas, tx);
    // Hold every slot: delivery stops once the ring is used up.
    let mut held: Vec<VideoFrame> = Vec::new();
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        if let Ok(f) = rx.recv_timeout(Duration::from_millis(20)) {
            held.push(f);
        }
    }
    assert!(
        held.len() <= 4,
        "{} frames while every slot was held",
        held.len()
    );
    let dropped = || {
        engine
            .canvas_output(canvas)
            .unwrap()
            .stats
            .encoder_dropped
            .load(Ordering::Relaxed)
    };
    assert!(dropped() > 0);
    // Releasing them lets frames flow again.
    held.clear();
    assert!(recv(&rx).is_some());
    engine.video_tap().remove(id);
}

#[test]
fn consumers_fall_back_to_cpu_frames_without_planes() {
    let engine = engine(false);
    let canvas = engine.snapshot().canvases[0].id;
    let (tx, rx) = crossbeam_channel::bounded(16);
    let (id, wants) = engine.video_tap().add_gpu(canvas, tx);
    let f = recv(&rx).expect("no frame after falling back");
    assert!(f.gpu.is_none());
    assert_eq!(f.data.len(), 64 * 36 * 3 / 2);
    assert!(
        !wants.load(Ordering::Relaxed),
        "the consumer was not switched to CPU frames"
    );
    engine.video_tap().remove(id);
}
