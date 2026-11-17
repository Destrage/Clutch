#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{engine, probe, shared_runner, temp_dir, wait_frames};
use estream_core::Timestamp;
use estream_engine::Command;
use estream_media::{Container, Recording, RecordingSettings, ReplayBuffer, VideoCodec, encode};
use estream_project::SourceDef;
use std::path::Path;
use std::time::Duration;

/// Two mixed tracks, no source tracks.
fn settings(dir: &Path, container: Container) -> RecordingSettings {
    RecordingSettings {
        folder: dir.to_path_buf(),
        container,
        tracks: 0b11,
        source_tracks: false,
        ..Default::default()
    }
}

/// [`settings`] with a track for each source.
fn with_sources(dir: &Path, container: Container) -> RecordingSettings {
    RecordingSettings {
        source_tracks: true,
        ..settings(dir, container)
    }
}

/// Adds a muted tone heard on every canvas: a source that never makes a sound.
fn add_muted_tone(engine: &estream_engine::Engine) {
    let mut quiet = SourceDef::new(
        "Quiet",
        "audio_test_tone",
        serde_json::json!({ "frequency": 440.0, "level_db": -12.0 }),
    );
    quiet.audio.global = true;
    quiet.audio.muted = true;
    engine
        .execute(Command::AddSource {
            source: quiet,
            index: 2,
        })
        .unwrap();
}

fn names(list: &[&str]) -> Vec<Option<String>> {
    list.iter().map(|n| Some((*n).to_owned())).collect()
}

#[test]
fn each_source_gets_its_own_named_track() {
    let engine = engine();
    add_muted_tone(&engine);
    let dir = temp_dir("source-tracks");
    std::thread::sleep(Duration::from_millis(300));

    // The MKV keeps every source track, silent or not.
    let rec = Recording::start(&engine, &with_sources(&dir, Container::Mkv)).unwrap();
    wait_frames(&rec, 30);
    std::thread::sleep(Duration::from_millis(500));
    let mkv = rec.stop(&engine).unwrap();
    let p = probe(&mkv);
    assert_eq!(
        p.audio_names,
        names(&["Track 1", "Track 2", "Tone", "Quiet"])
    );
    assert_eq!(p.audio_default, [true, false, false, false]);
    let v = p.stream_durations[0];
    for a in &p.stream_durations[1..] {
        assert!((v - a).abs() < 0.25, "A/V mismatch: video {v} audio {a}");
    }

    // Remuxed to MP4, the never-heard source is left out; names and defaults stay.
    std::thread::sleep(Duration::from_millis(50));
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            auto_remux: true,
            ..with_sources(&dir, Container::Mkv)
        },
    )
    .unwrap();
    wait_frames(&rec, 30);
    std::thread::sleep(Duration::from_millis(500));
    let mp4 = rec.stop(&engine).unwrap();
    assert_eq!(mp4.extension().unwrap(), "mp4");
    let p = probe(&mp4);
    assert_eq!(p.audio_names, names(&["Track 1", "Track 2", "Tone"]));
    assert_eq!(p.audio_default, [true, false, false]);
}

#[test]
fn a_saved_clip_leaves_out_silent_source_tracks() {
    let engine = engine();
    add_muted_tone(&engine);
    let dir = temp_dir("source-clip");
    std::thread::sleep(Duration::from_millis(300));
    let replay = ReplayBuffer::start(
        &engine,
        &RecordingSettings {
            replay_seconds: 10,
            ..with_sources(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(5));
    let now = engine.clock().now();
    let secs = |s: f64| Timestamp(now.0.saturating_sub((s * 1e9) as u64));
    let path = dir.join("clip.mp4");
    replay
        .save_range(secs(3.0), secs(1.0), path.clone(), Container::Mp4)
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap();
    replay.stop(&engine).unwrap();
    let p = probe(&path);
    assert_eq!(p.audio_names, names(&["Track 1", "Track 2", "Tone"]));
    assert_eq!(p.audio_default, [true, false, false]);
}

#[test]
fn streams_never_carry_source_tracks() {
    let stream = estream_media::StreamSettings::default();
    assert!(!stream.encoder_settings().source_tracks);
    assert!(RecordingSettings::default().source_tracks, "on by default");
}

#[test]
fn hardware_encoder_is_picked_automatically() {
    let list = encode::available(VideoCodec::H264);
    eprintln!(
        "H.264 encoders: {:?}",
        list.iter().map(|e| e.name).collect::<Vec<_>>()
    );
    assert!(!list.is_empty(), "at least the software encoder opens");
    let chosen = encode::choose(&RecordingSettings::default()).unwrap();
    if list.iter().any(|e| e.hardware) {
        assert!(
            chosen.hardware,
            "auto picks hardware first: {}",
            chosen.name
        );
    }
}

#[test]
fn records_video_and_two_audio_tracks_in_sync() {
    let engine = engine();
    let dir = temp_dir("mkv");
    std::thread::sleep(Duration::from_millis(300));
    let rec = Recording::start(&engine, &settings(&dir, Container::Mkv)).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    let frames = rec
        .stats()
        .frames
        .load(std::sync::atomic::Ordering::Relaxed);
    let path = rec.stop(&engine).unwrap();
    let p = probe(&path);
    eprintln!(
        "{} frames, duration {:.3}s, streams {:?}",
        frames, p.duration, p.stream_durations
    );
    let (codec, w, h) = p.video.clone().unwrap();
    assert_eq!((w, h), (640, 360));
    assert!(codec.contains("H264"), "{codec}");
    assert_eq!(p.audio_streams(), 2);
    assert!((p.duration - 3.0).abs() < 0.3, "duration {}", p.duration);
    // Both streams cover the recording. A slow first video frame (CPU rendering on CI) shortens
    // the video stream by its delay; timestamps stay right, so this isn't drift.
    let tolerance = if shared_runner() { 0.25 } else { 0.1 };
    let v = p.stream_durations[0];
    for a in &p.stream_durations[1..] {
        assert!(
            (v - a).abs() < tolerance,
            "A/V duration mismatch: video {v} audio {a}"
        );
    }
}

#[test]
fn every_container_is_playable_and_mkv_remuxes() {
    let engine = engine();
    let dir = temp_dir("containers");
    std::thread::sleep(Duration::from_millis(300));
    for container in Container::ALL {
        let rec = Recording::start(&engine, &settings(&dir, container)).unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let path = rec.stop(&engine).unwrap();
        let p = probe(&path);
        assert!(p.duration > 0.8, "{container:?}: {}", p.duration);
        assert_eq!(p.audio_streams(), 2, "{container:?}");
        // Distinct names (same second) must not overwrite each other.
        std::thread::sleep(Duration::from_millis(50));
    }
    let rec = Recording::start(
        &engine,
        &RecordingSettings {
            auto_remux: true,
            ..settings(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let mp4 = rec.stop(&engine).unwrap();
    assert_eq!(mp4.extension().unwrap(), "mp4");
    let p = probe(&mp4);
    assert!(p.video.is_some() && p.audio_streams() == 2 && p.duration > 0.8);
}

#[test]
fn replay_buffer_saves_the_last_seconds() {
    let engine = engine();
    let dir = temp_dir("replay");
    std::thread::sleep(Duration::from_millis(300));
    let replay = ReplayBuffer::start(
        &engine,
        &RecordingSettings {
            replay_seconds: 5,
            ..settings(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(9));
    let path = replay.save().unwrap();
    // The buffer keeps running after a save.
    std::thread::sleep(Duration::from_millis(500));
    assert!(replay.save().is_ok());
    replay.stop(&engine).unwrap();
    let p = probe(&path);
    eprintln!("replay duration {:.2}s", p.duration);
    assert!(
        p.duration >= 4.5 && p.duration <= 7.5,
        "replay {}",
        p.duration
    );
    assert!(p.video.is_some() && p.audio_streams() == 2);
}

#[test]
fn replay_buffer_saves_a_time_window() {
    let engine = engine();
    let dir = temp_dir("replay-range");
    std::thread::sleep(Duration::from_millis(300));
    let replay = ReplayBuffer::start(
        &engine,
        &RecordingSettings {
            replay_seconds: 10,
            ..settings(&dir, Container::Mkv)
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(8));
    let now = engine.clock().now();
    let secs = |s: f64| Timestamp(now.0.saturating_sub((s * 1e9) as u64));
    // 4 s ago to 1 s ago: starts at the keyframe at or before `from` (keyframes every 2 s).
    let (from, to) = (secs(4.0), secs(1.0));
    let path = dir.join("window.mp4");
    let clip = replay
        .save_range(from, to, path.clone(), Container::Mp4)
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap();
    assert_eq!(clip.path, path);
    assert!(clip.start <= from, "starts at or before the request");
    assert!(
        from.0 - clip.start.0 <= 2_100_000_000,
        "at most one GOP early"
    );
    assert!(
        clip.end <= to && to.0 - clip.end.0 < 100_000_000,
        "ends at the request"
    );

    // A window that is no longer buffered is refused.
    let old = replay
        .save_range(secs(60.0), secs(50.0), dir.join("old.mp4"), Container::Mp4)
        .recv_timeout(Duration::from_secs(20))
        .unwrap();
    assert!(old.is_err());
    replay.stop(&engine).unwrap();

    let p = probe(&path);
    let span = (clip.end.0 - clip.start.0) as f64 / 1e9;
    eprintln!("window clip {:.2}s (span {span:.2}s)", p.duration);
    assert!(
        p.duration >= 2.9 && p.duration <= 5.2,
        "clip {}",
        p.duration
    );
    assert!(
        (p.duration - span).abs() < 0.2,
        "file {} vs span {span}",
        p.duration
    );
    let v = p.stream_durations[0];
    for a in &p.stream_durations[1..] {
        assert!((v - a).abs() < 0.15, "A/V mismatch: video {v} audio {a}");
    }
}
