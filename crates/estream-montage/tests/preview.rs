#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::write_y4m;
use estream_gpu::{GpuContext, GpuOptions};
use estream_montage::model::{Format, Montage, Segment};
use estream_montage::preview::Preview;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn seeks_plays_and_pauses() {
    let gpu = GpuContext::new_headless(&GpuOptions::default()).unwrap();
    let dir = std::env::temp_dir().join(format!("estream-preview-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (red, blue) = (dir.join("red.y4m"), dir.join("blue.y4m"));
    write_y4m(&red, 2, |_| [1.0, 0.0, 0.0]);
    write_y4m(&blue, 2, |_| [0.0, 0.0, 1.0]);
    let mut m = Montage::new(
        "preview",
        Format {
            width: 320,
            height: 180,
            fps: 30,
        },
    );
    for p in [&red, &blue] {
        let id = m.new_id();
        m.segments.push(Segment::new(id, p, 0.0, 2.0));
    }
    m.normalize();
    let mut preview = Preview::start(&gpu, Arc::new(m.clone()));

    // Scrubbing shows the frame at the new position.
    preview.seek(3.0);
    wait_for("a frame at 3 s", || {
        preview
            .latest()
            .is_some_and(|f| (f.time - 3.0).abs() < 0.05)
    });
    let frame = preview.latest().unwrap();
    let rgba = estream_gpu::readback::read_texture(&gpu, &frame.texture).unwrap();
    let mid = ((90 * 320 + 160) * 4) as usize;
    assert!(
        rgba[mid + 2] > 200 && rgba[mid] < 40,
        "blue at 3 s: {:?}",
        &rgba[mid..mid + 3]
    );

    // Playing moves the position on; pausing holds it.
    preview.seek(0.0);
    preview.play();
    wait_for("playback", || preview.is_playing());
    std::thread::sleep(Duration::from_millis(700));
    let pos = preview.position();
    assert!(pos > 0.3 && pos < 1.5, "position {pos}");
    preview.pause();
    wait_for("pause", || !preview.is_playing());
    let held = preview.position();
    std::thread::sleep(Duration::from_millis(200));
    assert!((preview.position() - held).abs() < 0.02);

    // An edit keeps the position and shows the new content; playing stops at the end.
    m.segments.truncate(1);
    m.normalize();
    preview.set_montage(Arc::new(m.clone()));
    assert!((preview.duration() - 2.0).abs() < 1e-9);
    preview.seek(1.9);
    preview.play();
    // play() is handled on the worker: wait for it to start, then for the end.
    wait_for("playback", || preview.is_playing());
    wait_for("the end", || !preview.is_playing());
    assert!(
        (preview.position() - 2.0).abs() < 0.05,
        "stops at the end: {}",
        preview.position()
    );
    assert!(preview.error().is_none());
}
