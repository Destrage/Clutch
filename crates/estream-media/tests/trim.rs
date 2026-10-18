#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{engine, probe, temp_dir};
use estream_media::trim::{TrimMode, trim};
use estream_media::{Container, Recording, RecordingSettings};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// A 6 s MP4 (640x360@30, two audio tracks, keyframes every 2 s), recorded once.
fn source() -> PathBuf {
    static SOURCE: OnceLock<PathBuf> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            let engine = engine();
            let dir = temp_dir("trim-source");
            std::thread::sleep(Duration::from_millis(300));
            let rec = Recording::start(
                &engine,
                &RecordingSettings {
                    folder: dir,
                    container: Container::Mp4,
                    tracks: 0b11,
                    ..Default::default()
                },
            )
            .unwrap();
            std::thread::sleep(Duration::from_secs(6));
            rec.stop(&engine).unwrap()
        })
        .clone()
}

fn copy_of(name: &str) -> (PathBuf, PathBuf) {
    let dir = temp_dir(&format!("trim-{name}"));
    let input = dir.join("clip.mp4");
    std::fs::copy(source(), &input).unwrap();
    (dir, input)
}

fn assert_in_sync(path: &Path) {
    let p = probe(path);
    let v = p.stream_durations[0];
    for a in &p.stream_durations[1..] {
        assert!((v - a).abs() < 0.12, "A/V mismatch: video {v} audio {a}");
    }
}

#[test]
fn lossless_trim_starts_at_a_keyframe() {
    let (dir, input) = copy_of("lossless");
    let output = dir.join("out.mp4");
    let kept = trim(&input, &output, 2.5, 4.5, TrimMode::Lossless, &mut |_| true).unwrap();
    eprintln!("kept {kept:?}");
    // Keyframes are every 2 s, so the clip starts at the one at 2 s.
    assert!(
        kept.start <= 2.5 && kept.start >= 1.9,
        "start {}",
        kept.start
    );
    assert!(kept.end <= 4.5 && kept.end > 4.4, "end {}", kept.end);
    let p = probe(&output);
    assert!(
        (p.duration - (kept.end - kept.start)).abs() < 0.15,
        "file {} vs kept {kept:?}",
        p.duration
    );
    assert_eq!(p.audio_streams(), 2);
    assert_in_sync(&output);
}

#[test]
fn precise_trim_starts_at_the_in_point() {
    let (dir, input) = copy_of("precise");
    let output = dir.join("out.mp4");
    let mut last = 0.0;
    let kept = trim(&input, &output, 2.4, 4.4, TrimMode::Precise, &mut |p| {
        assert!(p >= last, "progress goes forward");
        last = p;
        true
    })
    .unwrap();
    eprintln!("kept {kept:?}");
    assert!(
        (kept.start - 2.4).abs() < 0.034,
        "first frame {}",
        kept.start
    );
    assert_eq!(last, 1.0);
    let p = probe(&output);
    assert!((p.duration - 2.0).abs() < 0.1, "duration {}", p.duration);
    let (_, w, h) = p.video.clone().unwrap();
    assert_eq!((w, h), (640, 360));
    assert_eq!(p.audio_streams(), 2);
    assert_in_sync(&output);
}

#[test]
fn trim_can_replace_the_original() {
    let (dir, input) = copy_of("replace");
    trim(&input, &input, 1.0, 3.0, TrimMode::Precise, &mut |_| true).unwrap();
    assert!((probe(&input).duration - 2.0).abs() < 0.1);
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().contains(".part."))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn cancelled_trim_leaves_no_file() {
    let (dir, input) = copy_of("cancel");
    let output = dir.join("out.mp4");
    let result = trim(&input, &output, 0.5, 5.5, TrimMode::Precise, &mut |p| {
        p < 0.5
    });
    assert!(result.is_err());
    assert!(!output.exists());
    assert!(!dir.join("out.part.mp4").exists());
    // The source is untouched.
    assert!((probe(&input).duration - probe(&source()).duration).abs() < 0.01);
}

#[test]
fn bad_ranges_are_refused() {
    let (dir, input) = copy_of("bad");
    let output = dir.join("out.mp4");
    assert!(trim(&input, &output, 3.0, 3.0, TrimMode::Lossless, &mut |_| true).is_err());
    assert!(
        trim(
            &input,
            &dir.join("out.avi"),
            1.0,
            2.0,
            TrimMode::Lossless,
            &mut |_| true
        )
        .is_err()
    );
    assert!(!output.exists());
}
