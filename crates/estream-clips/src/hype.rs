//! Audio hype detection: a sudden jump in loudness (a shout, an explosion of noise) far above
//! what the signal has been doing lately.
//!
//! - **Short-term loudness:** mean power over the last 400 ms, in dB.
//! - **Baseline:** the median short-term loudness over the last 60 s, so a long loud fight
//!   doesn't keep raising it the way an average would.
//! - **Fires** when short-term loudness stays above the baseline by the sensitivity threshold
//!   for 300 ms, above an absolute floor (so keyboard noise on a quiet mic never counts), then
//!   waits out a cooldown.
//!
//! [`HypeDetector::push`] takes levels at any rate (the app's ~60 Hz meter reads);
//! [`scan`] runs the same detector over decoded audio, for finding highlights in recordings.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    #[default]
    Medium,
    High,
}

impl Sensitivity {
    /// How far above the baseline (dB) counts as hype.
    pub fn threshold_db(self) -> f32 {
        match self {
            Sensitivity::Low => 18.0,
            Sensitivity::Medium => 14.0,
            Sensitivity::High => 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HypeConfig {
    pub sensitivity: Sensitivity,
    /// Short-term loudness must also be above this (dBFS).
    pub floor_db: f32,
    /// The baseline counts as at least this (dBFS), so after a quiet stretch an ordinary sound
    /// isn't a big jump.
    pub baseline_floor_db: f32,
    pub cooldown: f64,
    pub sustain: f64,
    pub short_window: f64,
    pub baseline_window: f64,
    /// No detection until this much history exists.
    pub warmup: f64,
}

impl HypeConfig {
    /// For game or desktop audio. Games go quiet often (menus, looting, waiting), and a jump
    /// from silence is not a highlight, so the baseline has a floor and the cooldown is long.
    /// Low keeps only the loudest moments: in shooters every gunfight clears a jump from a quiet
    /// stretch (fights peak around −14 to −21 dBFS), so it also needs −16 dBFS outright.
    pub fn game(sensitivity: Sensitivity) -> Self {
        let (floor_db, baseline_floor_db) = match sensitivity {
            Sensitivity::Low => (-16.0, -34.0),
            Sensitivity::Medium | Sensitivity::High => (-35.0, -40.0),
        };
        Self {
            sensitivity,
            floor_db,
            baseline_floor_db,
            cooldown: 45.0,
            sustain: 0.3,
            short_window: 0.4,
            baseline_window: 60.0,
            warmup: 10.0,
        }
    }

    /// For a microphone: ignores anything quieter than −30 dBFS (typing, breathing); a shout
    /// after a quiet minute counts.
    pub fn mic(sensitivity: Sensitivity) -> Self {
        Self {
            floor_db: -30.0,
            baseline_floor_db: SILENCE_DB,
            cooldown: 20.0,
            ..Self::game(sensitivity)
        }
    }
}

/// A detected hype moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    /// When the loudness first crossed the threshold (seconds, on the caller's clock).
    pub at: f64,
    /// How far above the baseline it was when confirmed (dB).
    pub over_db: f32,
}

/// Silence is treated as this level, so dB values stay finite.
const SILENCE_DB: f32 = -90.0;
/// How often a short-term value goes into the baseline history.
const HISTORY_STEP: f64 = 0.1;

fn to_db(power: f64) -> f32 {
    if power <= 1e-9 {
        SILENCE_DB
    } else {
        (10.0 * power.log10()).max(SILENCE_DB as f64) as f32
    }
}

#[derive(Debug)]
pub struct HypeDetector {
    cfg: HypeConfig,
    /// (time, power) within the short window.
    short: VecDeque<(f64, f64)>,
    /// (time, level dB) within the sustain time: its median must be hot too, so a click that
    /// lingers in the short window doesn't count.
    recent: VecDeque<(f64, f32)>,
    recent_sorted: Vec<f32>,
    /// (time, short-term dB) every [`HISTORY_STEP`], within the baseline window.
    history: VecDeque<(f64, f32)>,
    sorted: Vec<f32>,
    baseline: Option<f32>,
    first: Option<f64>,
    above_since: Option<f64>,
    last_fire: Option<f64>,
}

impl HypeDetector {
    pub fn new(cfg: HypeConfig) -> Self {
        Self {
            cfg,
            short: VecDeque::new(),
            recent: VecDeque::new(),
            recent_sorted: Vec::new(),
            history: VecDeque::new(),
            sorted: Vec::new(),
            baseline: None,
            first: None,
            above_since: None,
            last_fire: None,
        }
    }

    pub fn config(&self) -> &HypeConfig {
        &self.cfg
    }

    /// Switches to another configuration (e.g. a new sensitivity), keeping the history.
    pub fn set_config(&mut self, cfg: HypeConfig) {
        self.cfg = cfg;
    }

    /// The current baseline (dB), once there is history.
    pub fn baseline(&self) -> Option<f32> {
        self.baseline
    }

    /// Whether the median level over the sustain time is hot as well.
    fn recent_is_hot(&mut self, baseline: f32) -> bool {
        self.recent_sorted.clear();
        self.recent_sorted
            .extend(self.recent.iter().map(|(_, d)| *d));
        self.recent_sorted.sort_by(f32::total_cmp);
        self.recent_sorted
            .get(self.recent_sorted.len() / 2)
            .is_some_and(|m| m - baseline >= self.cfg.sensitivity.threshold_db())
    }

    /// Adds an RMS level (linear, 0..=1) measured at `t` seconds. Times must not go backwards.
    pub fn push(&mut self, t: f64, rms: f32) -> Option<Peak> {
        let first = *self.first.get_or_insert(t);
        let power = (rms as f64) * (rms as f64);
        self.short.push_back((t, power));
        while self
            .short
            .front()
            .is_some_and(|(s, _)| t - s > self.cfg.short_window)
        {
            self.short.pop_front();
        }
        let short_db =
            to_db(self.short.iter().map(|(_, p)| p).sum::<f64>() / self.short.len() as f64);
        self.recent.push_back((t, to_db(power)));
        while self
            .recent
            .front()
            .is_some_and(|(s, _)| t - s > self.cfg.sustain)
        {
            self.recent.pop_front();
        }

        if self
            .history
            .back()
            .is_none_or(|(h, _)| t - h >= HISTORY_STEP)
        {
            // Hype itself doesn't count toward the baseline, so a long shout can fire once
            // and the baseline stays where the quiet part was.
            if self.above_since.is_none() {
                self.history.push_back((t, short_db));
            }
            while self
                .history
                .front()
                .is_some_and(|(h, _)| t - h > self.cfg.baseline_window)
            {
                self.history.pop_front();
            }
            self.sorted.clear();
            self.sorted.extend(self.history.iter().map(|(_, d)| *d));
            self.sorted.sort_by(f32::total_cmp);
            self.baseline = self.sorted.get(self.sorted.len() / 2).copied();
        }

        let baseline = self.baseline?.max(self.cfg.baseline_floor_db);
        let over = short_db - baseline;
        let hot = t - first >= self.cfg.warmup
            && short_db >= self.cfg.floor_db
            && over >= self.cfg.sensitivity.threshold_db();
        if !hot {
            self.above_since = None;
            return None;
        }
        let since = *self.above_since.get_or_insert(t);
        let cooled = self
            .last_fire
            .is_none_or(|f| since - f >= self.cfg.cooldown);
        if t - since >= self.cfg.sustain && cooled && self.recent_is_hot(baseline) {
            self.last_fire = Some(since);
            return Some(Peak {
                at: since,
                over_db: over,
            });
        }
        None
    }
}

/// Finds hype moments in mono audio `samples` at `rate` Hz (offline, e.g. a recording's mix).
pub fn scan(samples: &[f32], rate: u32, cfg: HypeConfig) -> Vec<Peak> {
    let mut detector = HypeDetector::new(cfg);
    let block = (rate as usize / 100).max(1); // 10 ms
    let mut peaks = Vec::new();
    for (i, chunk) in samples.chunks(block).enumerate() {
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        let t = (i * block) as f64 / rate as f64;
        if let Some(p) = detector.push(t, rms) {
            peaks.push(p);
        }
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// Feeds `level(t)` (dBFS) at 60 Hz from `from` to `to`, returning the peaks.
    fn run(d: &mut HypeDetector, from: f64, to: f64, level: impl Fn(f64) -> f32) -> Vec<Peak> {
        let mut out = Vec::new();
        let mut t = from;
        while t < to {
            if let Some(p) = d.push(t, gain(level(t))) {
                out.push(p);
            }
            t += 1.0 / 60.0;
        }
        out
    }

    #[test]
    fn a_steady_level_never_fires() {
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::High));
        // Loud, with a little flutter.
        let peaks = run(&mut d, 0.0, 300.0, |t| -12.0 + (t * 7.0).sin() as f32 * 2.0);
        assert!(peaks.is_empty(), "{peaks:?}");
    }

    #[test]
    fn low_game_sensitivity_skips_ordinary_fights() {
        // A gunfight at −18 dBFS after a quiet minute: medium fires, low doesn't.
        let fight = |t: f64| if t < 60.0 { -45.0 } else { -18.0 };
        let mut medium = HypeDetector::new(HypeConfig::game(Sensitivity::Medium));
        assert_eq!(run(&mut medium, 0.0, 70.0, fight).len(), 1);
        let mut low = HypeDetector::new(HypeConfig::game(Sensitivity::Low));
        assert!(run(&mut low, 0.0, 70.0, fight).is_empty());
        // A really loud moment still counts on low.
        let mut low = HypeDetector::new(HypeConfig::game(Sensitivity::Low));
        assert_eq!(
            run(&mut low, 0.0, 70.0, |t| if t < 60.0 {
                -45.0
            } else {
                -13.0
            })
            .len(),
            1
        );
    }

    #[test]
    fn a_big_jump_fires_once() {
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::Medium));
        let peaks = run(&mut d, 0.0, 70.0, |t| if t < 60.0 { -40.0 } else { -20.0 });
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].at - 60.0).abs() < 0.3, "at {}", peaks[0].at);
        assert!(peaks[0].over_db > 14.0);
    }

    #[test]
    fn repeated_jumps_inside_the_cooldown_fire_once() {
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::Medium));
        let burst = |t: f64| {
            // 1 s bursts at 60, 65, 70, 90 and 110 s.
            if [60.0, 65.0, 70.0, 90.0, 110.0]
                .iter()
                .any(|s| t >= *s && t < s + 1.0)
            {
                -18.0
            } else {
                -40.0
            }
        };
        let peaks = run(&mut d, 0.0, 120.0, burst);
        let at: Vec<i64> = peaks.iter().map(|p| p.at.round() as i64).collect();
        assert_eq!(at, [60, 110], "{peaks:?}");
    }

    #[test]
    fn ordinary_game_sound_after_a_quiet_stretch_is_not_hype() {
        // A minute of near silence (a menu, looting), then normal gunfire at −30 dBFS: far
        // above the quiet median, but not above what a game normally sounds like.
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::Medium));
        let shots = |t: f64| if t < 60.0 { -70.0 } else { -30.0 };
        assert!(run(&mut d, 0.0, 70.0, shots).is_empty());
        // A real spike out of the same silence still counts.
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::Medium));
        let spike = |t: f64| if t < 60.0 { -70.0 } else { -18.0 };
        assert_eq!(run(&mut d, 0.0, 70.0, spike).len(), 1);
    }

    #[test]
    fn sensitivity_sets_the_threshold() {
        let step = |t: f64| if t < 60.0 { -40.0 } else { -28.0 }; // +12 dB
        let mut low = HypeDetector::new(HypeConfig::game(Sensitivity::Low));
        let mut high = HypeDetector::new(HypeConfig::game(Sensitivity::High));
        assert!(run(&mut low, 0.0, 65.0, step).is_empty());
        assert_eq!(run(&mut high, 0.0, 65.0, step).len(), 1);
    }

    #[test]
    fn short_blips_and_quiet_mics_are_ignored() {
        // A 0.1 s click is shorter than the sustain time.
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::High));
        let click = |t: f64| {
            if (60.0..60.1).contains(&t) {
                -10.0
            } else {
                -50.0
            }
        };
        assert!(run(&mut d, 0.0, 65.0, click).is_empty());
        // Typing on a silent mic: a big jump, but below the mic floor.
        let mut mic = HypeDetector::new(HypeConfig::mic(Sensitivity::High));
        let typing = |t: f64| if t < 60.0 { -80.0 } else { -45.0 };
        assert!(run(&mut mic, 0.0, 65.0, typing).is_empty());
        // A shout after a quiet minute does count.
        let mut mic = HypeDetector::new(HypeConfig::mic(Sensitivity::Medium));
        let shout = |t: f64| {
            if (60.0..62.0).contains(&t) {
                -12.0
            } else {
                -60.0
            }
        };
        assert_eq!(run(&mut mic, 0.0, 65.0, shout).len(), 1);
    }

    #[test]
    fn nothing_fires_during_warmup() {
        let mut d = HypeDetector::new(HypeConfig::game(Sensitivity::High));
        let early = |t: f64| if t < 3.0 { -50.0 } else { -10.0 };
        assert!(run(&mut d, 0.0, 9.0, early).is_empty());
    }

    #[test]
    fn scan_finds_a_burst_in_decoded_audio() {
        let rate = 48_000;
        let mut rng = 0x1234_5678u32;
        let mut noise = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        let samples: Vec<f32> = (0..rate * 45)
            .map(|i| {
                let t = i as f64 / rate as f64;
                let level = if (30.0..31.5).contains(&t) {
                    -12.0
                } else {
                    -40.0
                };
                noise() * gain(level) * 1.7 // uniform noise: RMS ≈ 0.58 × peak
            })
            .collect();
        let peaks = scan(&samples, rate as u32, HypeConfig::game(Sensitivity::Medium));
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].at - 30.0).abs() < 0.3, "at {}", peaks[0].at);
    }
}
