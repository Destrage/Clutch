#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_clips::markers::{Marker, Markers};
use estream_clips::{EventKind, Sensitivity};
use estream_montage::scan::scan;
mod common;

use common::write_wav;

#[test]
fn scans_a_recording_with_markers_and_a_loud_moment() {
    let dir = std::env::temp_dir().join(format!("estream-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let recording = dir.join("match.wav");

    // 70 s of quiet noise with a loud second at 50 s.
    let mut rng = 0x1234_5678u32;
    let samples: Vec<f32> = (0..48_000 * 70)
        .map(|i| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let t = i as f64 / 48_000.0;
            let gain = if (50.0..51.0).contains(&t) { 0.5 } else { 0.01 };
            (rng as f32 / u32::MAX as f32 - 0.5) * 2.0 * gain
        })
        .collect();
    let stereo: Vec<f32> = samples.iter().flat_map(|s| [*s, *s]).collect();
    write_wav(&recording, &stereo);

    let mut markers = Markers::default();
    for (kind, at) in [(EventKind::Kill, 20.0), (EventKind::Ace, 30.0)] {
        markers.push(Marker {
            kind,
            game: Some("Counter-Strike 2".into()),
            at,
        });
    }
    markers.save(&recording).unwrap();
    let decoded = estream_media::audio_file::decode_stereo(&recording).unwrap();
    assert!(
        (decoded.len() as f64 / 2.0 / 48_000.0 - 70.0).abs() < 0.1,
        "{}",
        decoded.len()
    );

    let mut last = 0.0;
    let highlights = scan(&recording, Sensitivity::Medium, &mut |p| {
        assert!(p >= last);
        last = p;
        true
    })
    .unwrap();
    assert_eq!(last, 1.0);
    eprintln!("{highlights:#?}");
    let reasons: Vec<String> = highlights.iter().flat_map(|h| h.reasons.clone()).collect();
    assert!(reasons.contains(&"Kill".to_owned()), "{reasons:?}");
    assert!(reasons.contains(&"Ace".to_owned()), "{reasons:?}");
    let hype = highlights
        .iter()
        .find(|h| h.reasons == ["Hype"])
        .expect("the loud moment is found from audio alone");
    assert!((hype.anchor - 50.0).abs() < 0.3, "hype at {}", hype.anchor);
    let ace = highlights
        .iter()
        .find(|h| h.reasons.contains(&"Ace".to_owned()))
        .unwrap();
    assert!(
        highlights.iter().all(|h| h.score <= ace.score),
        "the ace scores highest"
    );
    assert!(
        highlights
            .iter()
            .all(|h| h.start >= 0.0 && h.end <= 70.0 + 1e-6)
    );

    // Cancelling stops the scan.
    assert!(scan(&recording, Sensitivity::Medium, &mut |_| false).is_err());
}
