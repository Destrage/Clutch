//! Finding highlights in a recording: from its marker file (game events saved while it was
//! recorded) and from hype detection over its audio. Overlapping candidates merge; each
//! highlight has a score and the reasons it was found.

use estream_clips::markers::Markers;
use estream_clips::{EventKind, HypeConfig, HypeDetector, Peak, Sensitivity};
use std::path::Path;

/// A stretch of a recording worth using.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    /// Seconds in the recording.
    pub start: f64,
    pub end: f64,
    /// The main moment (the strongest event, or the hype peak).
    pub anchor: f64,
    /// 0..=1.
    pub score: f32,
    /// e.g. ["Ace", "Hype"].
    pub reasons: Vec<String>,
    /// Every event time inside.
    pub events: Vec<f64>,
}

impl Highlight {
    pub fn length(&self) -> f64 {
        self.end - self.start
    }
}

/// One signal before merging.
#[derive(Clone, Debug, PartialEq)]
struct Candidate {
    at: f64,
    start: f64,
    end: f64,
    score: f32,
    reason: String,
}

fn from_markers(markers: &Markers) -> Vec<Candidate> {
    markers
        .markers
        .iter()
        .filter(|m| m.kind != EventKind::Death)
        .map(|m| {
            let (pre, post) = m.kind.window();
            Candidate {
                at: m.at,
                start: m.at - pre,
                end: m.at + post,
                score: m.kind.weight(),
                reason: m.kind.label(),
            }
        })
        .collect()
}

/// Hype score from how far above the baseline the peak was: 10 dB → 0.3, 30 dB → 0.9.
fn hype_score(over_db: f32) -> f32 {
    (0.3 + (over_db - 10.0) / 20.0 * 0.6).clamp(0.3, 0.9)
}

fn from_peaks(peaks: &[Peak]) -> Vec<Candidate> {
    let (pre, post) = EventKind::Hype.window();
    peaks
        .iter()
        .map(|p| Candidate {
            at: p.at,
            start: p.at - pre,
            end: p.at + post,
            score: hype_score(p.over_db),
            reason: EventKind::Hype.label(),
        })
        .collect()
}

/// Merges overlapping candidates into highlights within `0..duration`, sorted by time.
/// A merged highlight keeps the best score, plus 0.1 for every other signal in it.
fn merge(mut candidates: Vec<Candidate>, duration: f64) -> Vec<Highlight> {
    candidates.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut groups: Vec<Vec<Candidate>> = Vec::new();
    for c in candidates {
        match groups.last_mut() {
            Some(g) if g.iter().any(|x| c.start <= x.end) => g.push(c),
            _ => groups.push(vec![c]),
        }
    }
    groups
        .into_iter()
        .map(|g| {
            let best = g
                .iter()
                .max_by(|a, b| a.score.total_cmp(&b.score))
                .cloned()
                .unwrap_or_else(|| g[0].clone());
            let mut reasons: Vec<String> = Vec::new();
            for c in &g {
                if !reasons.contains(&c.reason) {
                    reasons.push(c.reason.clone());
                }
            }
            let mut events: Vec<f64> = g.iter().map(|c| c.at).collect();
            events.sort_by(f64::total_cmp);
            Highlight {
                start: g
                    .iter()
                    .map(|c| c.start)
                    .fold(f64::INFINITY, f64::min)
                    .max(0.0),
                end: g
                    .iter()
                    .map(|c| c.end)
                    .fold(f64::NEG_INFINITY, f64::max)
                    .min(duration.max(0.0)),
                anchor: best.at,
                score: (best.score + 0.1 * (g.len() - 1) as f32).min(1.0),
                reasons,
                events,
            }
        })
        .filter(|h| h.end > h.start)
        .collect()
}

/// Finds hype peaks in a stream of interleaved stereo 48 kHz chunks.
pub struct AudioScanner {
    detector: HypeDetector,
    /// Samples (per channel) seen.
    frames: usize,
    block: Vec<f32>,
    peaks: Vec<Peak>,
}

const BLOCK: usize = 480; // 10 ms at 48 kHz

impl AudioScanner {
    pub fn new(sensitivity: Sensitivity) -> Self {
        Self {
            detector: HypeDetector::new(HypeConfig::game(sensitivity)),
            frames: 0,
            block: Vec::with_capacity(BLOCK),
            peaks: Vec::new(),
        }
    }

    pub fn push(&mut self, stereo: &[f32]) {
        for lr in stereo.as_chunks::<2>().0 {
            self.block.push((lr[0] + lr[1]) * 0.5);
            if self.block.len() == BLOCK {
                let rms = (self.block.iter().map(|s| s * s).sum::<f32>() / BLOCK as f32).sqrt();
                let t = self.frames as f64 / 48_000.0;
                if let Some(p) = self.detector.push(t, rms) {
                    self.peaks.push(p);
                }
                self.frames += BLOCK;
                self.block.clear();
            }
        }
    }

    pub fn finish(self) -> Vec<Peak> {
        self.peaks
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{0}")]
    Media(#[from] estream_media::MediaError),
    #[error("scan cancelled")]
    Cancelled,
}

/// Scans a recording: its markers (if there is a marker file) and its audio. `progress`
/// receives 0..=1 and returns false to cancel.
pub fn scan(
    recording: &Path,
    sensitivity: Sensitivity,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Vec<Highlight>, ScanError> {
    let markers = Markers::load(recording).unwrap_or_default();
    let duration = estream_media::probe_duration(recording).unwrap_or(f64::INFINITY);
    let mut scanner = AudioScanner::new(sensitivity);
    let mut seen = 0usize;
    let mut cancelled = false;
    let decoded = estream_media::audio_file::for_each_chunk(recording, 0.0, |chunk| {
        scanner.push(chunk);
        seen += chunk.len() / 2;
        let p = (seen as f64 / 48_000.0 / duration).clamp(0.0, 1.0) as f32;
        if !progress(p) {
            cancelled = true;
        }
        !cancelled
    });
    if cancelled {
        return Err(ScanError::Cancelled);
    }
    // A recording without audio still has its markers.
    let audio_secs = match decoded {
        Ok(secs) => secs,
        Err(e) if markers.markers.is_empty() => return Err(e.into()),
        Err(_) => 0.0,
    };
    let duration = if duration.is_finite() {
        duration
    } else {
        audio_secs
    };
    let mut candidates = from_markers(&markers);
    candidates.extend(from_peaks(&scanner.finish()));
    progress(1.0);
    Ok(merge(candidates, duration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_clips::markers::Marker;

    fn marker(kind: EventKind, at: f64) -> Marker {
        Marker {
            kind,
            game: Some("Counter-Strike 2".into()),
            at,
        }
    }

    #[test]
    fn markers_become_scored_windows_and_the_ace_wins() {
        let mut m = Markers::default();
        m.push(marker(EventKind::Kill, 100.0));
        m.push(marker(EventKind::Ace, 400.0));
        m.push(marker(EventKind::Death, 500.0));
        let h = merge(from_markers(&m), 1200.0);
        assert_eq!(h.len(), 2, "deaths aren't highlights: {h:?}");
        assert_eq!((h[0].start, h[0].end), (90.0, 103.0));
        let best = h
            .iter()
            .max_by(|a, b| a.score.total_cmp(&b.score))
            .unwrap_or_else(|| panic!("none"));
        assert_eq!(best.reasons, ["Ace"]);
        assert_eq!(best.anchor, 400.0);
    }

    #[test]
    fn overlapping_signals_merge_and_score_higher() {
        let mut m = Markers::default();
        m.push(marker(EventKind::Kill, 100.0));
        m.push(marker(EventKind::Kill, 104.0));
        let peaks = [Peak {
            at: 103.0,
            over_db: 20.0,
        }];
        let mut c = from_markers(&m);
        c.extend(from_peaks(&peaks));
        let h = merge(c, 1000.0);
        assert_eq!(h.len(), 1, "{h:?}");
        assert_eq!(h[0].reasons, ["Kill", "Hype"]);
        assert_eq!(h[0].events, [100.0, 103.0, 104.0]);
        assert!(
            (h[0].score - 0.8).abs() < 1e-6,
            "0.6 + 2 × 0.1: {}",
            h[0].score
        );
        assert_eq!((h[0].start, h[0].end), (90.0, 107.0));
    }

    #[test]
    fn windows_stay_inside_the_recording() {
        let mut m = Markers::default();
        m.push(marker(EventKind::Kill, 3.0));
        m.push(marker(EventKind::MatchWin, 598.0));
        let h = merge(from_markers(&m), 600.0);
        assert_eq!(h[0].start, 0.0);
        assert_eq!(h[1].end, 600.0);
    }

    #[test]
    fn audio_scanner_finds_a_loud_moment() {
        let mut s = AudioScanner::new(Sensitivity::Medium);
        let rate = 48_000usize;
        let mut rng = 0x9e37_79b9u32;
        let mut chunk = Vec::with_capacity(rate * 2);
        for second in 0..60 {
            chunk.clear();
            let gain = if second == 40 { 0.4 } else { 0.01 };
            for _ in 0..rate {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let v = (rng as f32 / u32::MAX as f32 - 0.5) * 2.0 * gain;
                chunk.extend([v, v]);
            }
            s.push(&chunk);
        }
        let peaks = s.finish();
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].at - 40.0).abs() < 0.3);
    }
}
