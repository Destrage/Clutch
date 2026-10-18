#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{rms, sine, write_wav};
use estream_montage::audio::AudioRenderer;
use estream_montage::model::{Format, Montage, Music, Segment, Transition};
use std::path::PathBuf;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("estream-montage-audio-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

#[test]
fn mixes_clips_with_crossfades_music_ducking_and_the_end_fade() {
    let d = dir();
    let (a, b, song) = (d.join("a.wav"), d.join("b.wav"), d.join("song.wav"));
    write_wav(&a, &sine(440.0, 6.0, 0.4));
    write_wav(&b, &sine(660.0, 6.0, 0.4));
    write_wav(&song, &sine(110.0, 30.0, 0.3));

    let mut m = Montage::new("t", Format::LANDSCAPE);
    for (path, from) in [(&a, 1.0), (&b, 2.0)] {
        let id = m.new_id();
        m.segments.push(Segment::new(id, path, from, from + 4.0));
    }
    m.normalize();
    m.transitions[0] = Transition::Crossfade { secs: 1.0 };
    let mut r = AudioRenderer::new();

    // Clips only: 7 s, steady through the equal-power crossfade.
    let clips = r.render(&m);
    assert_eq!(clips.len(), (m.duration() * 48_000.0).round() as usize * 2);
    let steady = rms(&clips, 0.5, 2.5);
    let mid = rms(&clips, 3.4, 3.6);
    assert!((steady - 0.4 / 2f32.sqrt()).abs() < 0.01, "steady {steady}");
    assert!(
        (mid / steady - 1.0).abs() < 0.2,
        "crossfade dips: {mid} vs {steady}"
    );

    // With music and an end fade: the music ducks under the clips, the end is silent.
    m.segments[1].gain_db = -60.0; // second clip near silent: music comes back up
    m.music = Some(Music {
        source: song.clone(),
        source_rel: None,
        src_in: 0.0,
        gain_db: 0.0,
        fade_in: 0.0,
        fade_out: 0.0,
        beats: Vec::new(),
        missing: false,
    });
    m.fade_out = 0.5;
    let mixed = r.render(&m);
    let music_only = rms(&mixed, 5.5, 6.3);
    assert!(
        (music_only - 0.3 / 2f32.sqrt()).abs() < 0.03,
        "music at full: {music_only}"
    );
    let under_clip = rms(&mixed, 1.5, 2.5);
    let clip_alone = 0.4 / 2f32.sqrt();
    let ducked_music = 0.3 / 2f32.sqrt() * 10f32.powf(-8.0 / 20.0);
    let expected = (clip_alone * clip_alone + ducked_music * ducked_music).sqrt();
    assert!(
        (under_clip - expected).abs() < 0.03,
        "{under_clip} vs {expected}"
    );
    assert!(
        rms(&mixed, m.duration() - 0.02, m.duration()) < 0.01,
        "faded out"
    );
}

#[test]
fn slow_segments_take_longer_and_missing_audio_is_silence() {
    let d = dir();
    let a = d.join("slow.wav");
    write_wav(&a, &sine(440.0, 4.0, 0.4));
    let mut m = Montage::new("t", Format::LANDSCAPE);
    let id = m.new_id();
    let mut s = Segment::new(id, &a, 0.0, 2.0);
    s.speed = 0.5;
    m.segments.push(s);
    let id = m.new_id();
    m.segments
        .push(Segment::new(id, d.join("nothing-here.wav"), 0.0, 1.0));
    m.normalize();
    let mix = AudioRenderer::new().render(&m);
    assert_eq!(mix.len(), 5 * 48_000 * 2, "4 s slowed + 1 s");
    assert!(rms(&mix, 1.0, 3.0) > 0.2, "slowed tone present");
    assert!(rms(&mix, 4.1, 4.9) < 1e-4, "missing source is silent");
}
