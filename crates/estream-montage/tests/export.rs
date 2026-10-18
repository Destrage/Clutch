#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{probe, sine, write_wav, write_y4m};
use estream_gpu::{GpuContext, GpuOptions};
use estream_montage::export::{ExportOptions, ExportPreset, export};
use estream_montage::model::{Format, Montage, Music, Segment, Transition};
use std::path::PathBuf;

fn setup(name: &str) -> (PathBuf, Montage) {
    let dir = std::env::temp_dir().join(format!("estream-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (red, blue, song) = (
        dir.join("red.y4m"),
        dir.join("blue.y4m"),
        dir.join("song.wav"),
    );
    write_y4m(&red, 2, |_| [1.0, 0.0, 0.0]);
    write_y4m(&blue, 2, |_| [0.0, 0.0, 1.0]);
    write_wav(&song, &sine(220.0, 10.0, 0.3));
    let mut m = Montage::new("Export test", Format::LANDSCAPE);
    for path in [&red, &blue] {
        let id = m.new_id();
        m.segments.push(Segment::new(id, path, 0.0, 2.0));
    }
    m.normalize();
    m.transitions[0] = Transition::Crossfade { secs: 0.5 };
    m.music = Some(Music {
        source: song,
        source_rel: None,
        src_in: 0.0,
        gain_db: 0.0,
        fade_in: 0.0,
        fade_out: 0.5,
        beats: Vec::new(),
        missing: false,
    });
    m.fade_out = 0.3;
    (dir, m)
}

#[test]
fn export_matches_the_timeline_with_audio_in_sync() {
    let gpu = GpuContext::new_headless(&GpuOptions::default()).unwrap();
    let (dir, m) = setup("timeline");
    let out = dir.join("montage.mp4");
    let options = ExportOptions {
        preset: ExportPreset::Custom(Format {
            width: 320,
            height: 180,
            fps: 30,
        }),
        bitrate_kbps: 2_000,
        ..ExportOptions::new(&out)
    };
    let mut last = 0.0;
    let path = export(&gpu, &m, &options, &mut |p| {
        assert!(p >= last);
        last = p;
        true
    })
    .unwrap();
    assert_eq!(path, out);
    assert_eq!(last, 1.0);
    assert!(!dir.join("montage.part.mp4").exists());

    let p = probe(&out);
    eprintln!("{p:?}");
    assert_eq!(p.size, Some((320, 180)));
    let video = p
        .streams
        .iter()
        .find(|(c, _)| c.contains("H264"))
        .expect("H.264 video");
    let audio = p
        .streams
        .iter()
        .find(|(c, _)| c.contains("AAC"))
        .expect("AAC audio");
    // 2 + 2 s with a 0.5 s crossfade.
    assert!((m.duration() - 3.5).abs() < 1e-9);
    assert!((video.1 - 3.5).abs() < 0.1, "video {}", video.1);
    assert!(
        (audio.1 - video.1).abs() < 0.1,
        "A/V: audio {} video {}",
        audio.1,
        video.1
    );
}

#[test]
fn cancelled_export_leaves_nothing() {
    let gpu = GpuContext::new_headless(&GpuOptions::default()).unwrap();
    let (dir, m) = setup("cancel");
    let out = dir.join("cancelled.mp4");
    let options = ExportOptions {
        preset: ExportPreset::Custom(Format {
            width: 320,
            height: 180,
            fps: 30,
        }),
        ..ExportOptions::new(&out)
    };
    let result = export(&gpu, &m, &options, &mut |p| p < 0.5);
    assert!(result.is_err());
    assert!(!out.exists() && !dir.join("cancelled.part.mp4").exists());
    // Nothing to export is an error too.
    let empty = Montage::default();
    assert!(export(&gpu, &empty, &options, &mut |_| true).is_err());
}
