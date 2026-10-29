#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{engine, probe, skip_without_realtime_gpu, temp_dir};
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
    if skip_without_realtime_gpu() {
        return;
    }
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

#[test]
fn thumbnail_is_a_small_jpeg_of_the_requested_frame() {
    if skip_without_realtime_gpu() {
        return;
    }
    let (dir, input) = copy_of("thumb");
    let jpg = dir.join("clip.jpg");
    let t = estream_media::thumbnail::thumbnail(&input, 3.0, 320, &jpg).unwrap();
    assert!((t - 3.0).abs() < 0.034, "frame at {t}");
    let bytes = std::fs::read(&jpg).unwrap();
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "JPEG signature");
    let p = probe(&jpg);
    let (_, w, h) = p.video.clone().unwrap();
    assert_eq!((w, h), (320, 180));

    // Past the end: the last frame is used.
    let t = estream_media::thumbnail::thumbnail(&input, 60.0, 320, &jpg).unwrap();
    assert!(t > 5.5 && t < 6.2, "last frame at {t}");
}

#[test]
fn audio_decodes_to_48k_stereo_from_any_point() {
    let (_dir, input) = copy_of("audio");
    let all = estream_media::audio_file::decode_stereo(&input).unwrap();
    let secs = all.len() as f64 / 2.0 / 48_000.0;
    assert!(
        (secs - probe(&input).duration).abs() < 0.1,
        "decoded {secs} s"
    );
    // The source has a 1 kHz tone at -12 dBFS: check the level.
    let mono = estream_media::audio_file::to_mono(&all);
    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    let db = 20.0 * rms.log10();
    assert!((db - (-15.0)).abs() < 1.5, "tone at {db} dBFS RMS");
    // From 4 s: about 2 s left; stopping early works too.
    let mut n = 0;
    let rest = estream_media::audio_file::for_each_chunk(&input, 4.0, |c| {
        n += c.len();
        true
    })
    .unwrap();
    assert!((rest - (secs - 4.0)).abs() < 0.15, "rest {rest}");
    let mut chunks = 0;
    estream_media::audio_file::for_each_chunk(&input, 0.0, |_| {
        chunks += 1;
        chunks < 3
    })
    .unwrap();
    assert_eq!(chunks, 3);
}

#[test]
fn frame_reader_gives_the_frame_at_any_time() {
    if skip_without_realtime_gpu() {
        return;
    }
    use estream_media::frames::FrameReader;
    let (_dir, input) = copy_of("frames");
    let mut r = FrameReader::open(&input).unwrap();
    assert_eq!((r.width, r.height), (640, 360));
    assert!((r.fps - 30.0).abs() < 0.5, "fps {}", r.fps);
    let frame = 1.0 / 30.0;
    let time = |r: &mut FrameReader, t: f64| r.frame_at(t).unwrap().map(|f| f.time).unwrap();
    // Exact frames, then stepping forward one frame at a time.
    let t = time(&mut r, 2.4);
    assert!(t <= 2.4 + frame / 2.0 && t > 2.4 - frame, "at 2.4 got {t}");
    let mut last = t;
    for k in 1..=30 {
        let want = 2.4 + k as f64 * frame;
        let got = time(&mut r, want);
        assert!(got > last, "step {k}: {got} after {last}");
        assert!(
            (got - want).abs() <= frame / 2.0 + 1e-3,
            "step {k}: {got} for {want}"
        );
        last = got;
    }
    // Back in time (seek), far ahead (seek), the same time twice.
    let back = time(&mut r, 1.0);
    assert!((back - 1.0).abs() <= frame, "back {back}");
    let ahead = time(&mut r, 5.5);
    assert!((ahead - 5.5).abs() <= frame, "ahead {ahead}");
    assert_eq!(time(&mut r, 5.5), ahead);
    // Before the start: the first frame; past the end: the last one.
    assert!(time(&mut r, -1.0) < frame);
    let end = time(&mut r, 100.0);
    assert!(end > 5.5 && end < 6.2, "end {end}");
    let f = r.frame_at(3.0).unwrap().unwrap();
    assert_eq!(f.data.len(), 640 * 360 * 3 / 2);
}
