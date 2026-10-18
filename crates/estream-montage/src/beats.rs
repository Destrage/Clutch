//! Beat detection for montage music: where the beats are, so cuts can land on them.
//!
//! 1. **Onset envelope:** spectral flux (how much the log magnitude spectrum rises from one
//!    frame to the next) over 1024-sample frames with a 512-sample hop, minus its local mean.
//! 2. **Tempo:** autocorrelation of the envelope over 70–180 BPM, weighted toward 120 BPM
//!    (which avoids picking half or double the tempo), refined between frames.
//! 3. **Beats:** dynamic programming (Ellis 2007): each frame's score is its onset strength
//!    plus the best earlier beat about one period back, with a penalty for drifting.
//! 4. **Downbeats:** every fourth beat, starting where the onsets are strongest.

const FRAME: usize = 1024;
const HOP: usize = 512;
const MIN_BPM: f64 = 70.0;
const MAX_BPM: f64 = 180.0;
/// How strongly beats keep to the tempo (higher = stricter).
const TIGHTNESS: f64 = 100.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Beats {
    pub bpm: f64,
    /// Beat times in seconds.
    pub beats: Vec<f64>,
    /// Every fourth beat (the "one").
    pub downbeats: Vec<f64>,
}

impl Beats {
    /// The beat nearest to `t`, if one is within `tolerance` seconds.
    pub fn nearest(&self, t: f64, tolerance: f64) -> Option<f64> {
        nearest(&self.beats, t, tolerance)
    }
}

/// The time in `times` (sorted) nearest to `t`, if within `tolerance`.
pub fn nearest(times: &[f64], t: f64, tolerance: f64) -> Option<f64> {
    let i = times.partition_point(|b| *b < t);
    [i.checked_sub(1), Some(i)]
        .into_iter()
        .flatten()
        .filter_map(|k| times.get(k).copied())
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        .filter(|b| (b - t).abs() <= tolerance)
}

/// Finds the beats in mono audio at `rate` Hz. `None` for silence or very short audio.
pub fn detect(mono: &[f32], rate: u32) -> Option<Beats> {
    let envelope = onset_envelope(mono);
    if envelope.len() < 64 {
        return None;
    }
    let fps = rate as f64 / HOP as f64;
    let period = tempo_period(&envelope, fps)?;
    let frames = track(&envelope, period);
    if frames.len() < 4 {
        return None;
    }
    // A frame's flux describes the change into it; place the beat at the frame's centre.
    let time = |f: usize| (f * HOP + FRAME / 2) as f64 / rate as f64;
    let beats: Vec<f64> = frames.iter().map(|&f| time(f)).collect();
    let phase = (0..4)
        .max_by(|&a, &b| {
            let strength = |p: usize| {
                frames
                    .iter()
                    .skip(p)
                    .step_by(4)
                    .map(|&f| envelope[f])
                    .sum::<f64>()
            };
            strength(a).total_cmp(&strength(b))
        })
        .unwrap_or(0);
    let downbeats = beats.iter().skip(phase).step_by(4).copied().collect();
    Some(Beats {
        bpm: 60.0 * fps / period,
        beats,
        downbeats,
    })
}

/// Spectral-flux onset strength per hop, with the local mean removed and scaled to unit
/// standard deviation.
fn onset_envelope(mono: &[f32]) -> Vec<f64> {
    if mono.len() < FRAME {
        return Vec::new();
    }
    let window: Vec<f64> = (0..FRAME)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FRAME as f64).cos())
        .collect();
    let fft = Fft::new(FRAME);
    let bins = FRAME / 2;
    let mut prev = vec![0.0f64; bins];
    let mut re = vec![0.0f64; FRAME];
    let mut im = vec![0.0f64; FRAME];
    let mut flux = Vec::with_capacity(mono.len() / HOP);
    let mut start = 0;
    while start + FRAME <= mono.len() {
        for i in 0..FRAME {
            re[i] = mono[start + i] as f64 * window[i];
            im[i] = 0.0;
        }
        fft.run(&mut re, &mut im);
        let mut sum = 0.0;
        for k in 1..bins {
            let mag = (1.0 + 1000.0 * (re[k] * re[k] + im[k] * im[k]).sqrt()).ln();
            sum += (mag - prev[k]).max(0.0);
            prev[k] = mag;
        }
        flux.push(sum);
        start += HOP;
    }
    if let Some(first) = flux.first_mut() {
        *first = 0.0; // no previous frame to compare with
    }
    // Remove the local mean (about 0.3 s each side) so steady sound doesn't count.
    let half = 16;
    let mut out = Vec::with_capacity(flux.len());
    for i in 0..flux.len() {
        let (a, b) = (i.saturating_sub(half), (i + half + 1).min(flux.len()));
        let mean = flux[a..b].iter().sum::<f64>() / (b - a) as f64;
        out.push((flux[i] - mean).max(0.0));
    }
    let mean = out.iter().sum::<f64>() / out.len() as f64;
    let sd = (out.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
    if sd > 1e-9 {
        out.iter_mut().for_each(|v| *v /= sd);
    }
    out
}

/// The beat period in frames, from the weighted autocorrelation of the envelope.
fn tempo_period(env: &[f64], fps: f64) -> Option<f64> {
    let min_lag = (60.0 * fps / MAX_BPM).floor() as usize;
    let max_lag = (60.0 * fps / MIN_BPM).ceil() as usize;
    if env.len() < max_lag * 2 {
        return None;
    }
    let center = 60.0 * fps / 120.0;
    // Smooth first: with a period that isn't a whole number of frames (37.5 at 150 BPM),
    // onsets alternate 37 and 38 frames apart, and sharp spikes would only line up at twice
    // the period.
    let mut smooth = env.to_vec();
    for _ in 0..2 {
        let prev = smooth.clone();
        for i in 0..smooth.len() {
            let at = |k: isize| prev[(i as isize + k).clamp(0, prev.len() as isize - 1) as usize];
            smooth[i] = 0.25 * at(-1) + 0.5 * at(0) + 0.25 * at(1);
        }
    }
    let env = &smooth;
    let acf = |lag: usize| -> f64 {
        env.iter().zip(&env[lag..]).map(|(a, b)| a * b).sum::<f64>() / (env.len() - lag) as f64
    };
    // Weight toward 120 BPM: a beat grid also correlates at twice its period (and accents
    // every few beats make that even stronger), so the raw peak could be half the tempo.
    let scored: Vec<(usize, f64)> = (min_lag.max(1)..=max_lag)
        .map(|l| {
            let octaves = (l as f64 / center).log2();
            let weight = (-0.5 * (octaves / 0.9).powi(2)).exp();
            (l, acf(l) * weight)
        })
        .collect();
    let (best_i, &(best, score)) = scored
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1))?;
    if score <= 1e-9 {
        return None;
    }
    // Parabolic interpolation between neighbouring lags.
    let at = |i: usize| scored.get(i).map(|s| s.1);
    let refined = match (best_i.checked_sub(1).and_then(at), at(best_i + 1)) {
        (Some(a), Some(c)) => {
            let denom = a - 2.0 * score + c;
            if denom.abs() > 1e-12 {
                best as f64 + 0.5 * (a - c) / denom
            } else {
                best as f64
            }
        }
        _ => best as f64,
    };
    Some(refined.clamp(min_lag as f64, max_lag as f64))
}

/// Beat frames by dynamic programming over the envelope, for a beat `period` in frames.
fn track(env: &[f64], period: f64) -> Vec<usize> {
    let n = env.len();
    let mut score = vec![0.0f64; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    let (lo, hi) = (
        (period / 2.0).round() as usize,
        (2.0 * period).round() as usize,
    );
    for t in 0..n {
        let mut best = 0.0;
        let mut from = None;
        for gap in lo.max(1)..=hi {
            let Some(p) = t.checked_sub(gap) else { break };
            let penalty = TIGHTNESS * (gap as f64 / period).ln().powi(2);
            let s = score[p] - penalty;
            if from.is_none() || s > best {
                best = s;
                from = Some(p);
            }
        }
        score[t] = env[t] + best.max(0.0);
        back[t] = from.filter(|_| best > 0.0);
    }
    // Start from the best-scoring frame in the last period and walk back.
    let tail = n.saturating_sub(period.ceil() as usize);
    let Some(mut t) = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b])) else {
        return Vec::new();
    };
    let mut beats = vec![t];
    while let Some(p) = back[t] {
        beats.push(p);
        t = p;
    }
    beats.reverse();
    beats
}

/// An in-place iterative radix-2 FFT for one fixed size.
struct Fft {
    n: usize,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    fn new(n: usize) -> Self {
        debug_assert!(n.is_power_of_two());
        let (cos, sin) = (0..n / 2)
            .map(|k| {
                let a = -std::f64::consts::TAU * k as f64 / n as f64;
                (a.cos(), a.sin())
            })
            .unzip();
        Self { n, cos, sin }
    }

    fn run(&self, re: &mut [f64], im: &mut [f64]) {
        let n = self.n;
        // Bit-reversal permutation.
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A click track: a short 2 kHz pip on every beat (every fourth louder) over quiet noise.
    fn clicks(bpm: f64, secs: f64, offset: f64) -> (Vec<f32>, Vec<f64>) {
        let n = (secs * RATE as f64) as usize;
        let mut out = vec![0.0f32; n];
        let mut rng = 0x2545_f491u32;
        for s in out.iter_mut() {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            *s = (rng as f32 / u32::MAX as f32 - 0.5) * 0.01;
        }
        let period = 60.0 / bpm;
        let mut truth = Vec::new();
        let mut k = 0;
        loop {
            let t = offset + k as f64 * period;
            if t >= secs - 0.05 {
                break;
            }
            truth.push(t);
            let amp = if k % 4 == 0 { 0.9 } else { 0.4 };
            let start = (t * RATE as f64) as usize;
            for i in 0..(0.03 * RATE as f64) as usize {
                if let Some(s) = out.get_mut(start + i) {
                    let x = i as f32 / RATE as f32;
                    *s += amp * (std::f32::consts::TAU * 2000.0 * x).sin() * (-x * 150.0).exp();
                }
            }
            k += 1;
        }
        (out, truth)
    }

    /// `half_ok`: very fast music may be read at half time, which still puts every beat on a
    /// real one (what montage cuts need).
    fn check(bpm: f64, half_ok: bool) {
        let (audio, truth) = clicks(bpm, 30.0, 0.37);
        let found = detect(&audio, RATE).unwrap_or_else(|| panic!("no beats at {bpm}"));
        let close = |target: f64| (found.bpm - target).abs() / target < 0.02;
        assert!(
            close(bpm) || (half_ok && close(bpm / 2.0)),
            "tempo {} for {bpm}",
            found.bpm
        );
        let expected = if close(bpm) {
            truth.len()
        } else {
            truth.len() / 2
        };
        let errors: Vec<f64> = found
            .beats
            .iter()
            .filter_map(|b| nearest(&truth, *b, 0.1).map(|t| (b - t).abs()))
            .collect();
        let matched = errors.len() as f64 / found.beats.len() as f64;
        let mean = errors.iter().sum::<f64>() / errors.len().max(1) as f64;
        assert!(
            matched > 0.9,
            "{bpm}: only {matched:.2} of beats on a click"
        );
        assert!(mean < 0.03, "{bpm}: mean error {mean:.3} s");
        assert!(
            found.beats.len() as f64 > expected as f64 * 0.85,
            "{bpm}: too few beats"
        );
    }

    #[test]
    fn finds_tempo_and_beats_of_click_tracks() {
        for bpm in [92.0, 120.0, 128.0, 150.0] {
            check(bpm, false);
        }
        check(174.0, true);
    }

    #[test]
    fn downbeats_land_on_the_accents() {
        let (audio, truth) = clicks(120.0, 30.0, 0.37);
        let found = detect(&audio, RATE).unwrap_or_else(|| panic!("no beats"));
        let accents: Vec<f64> = truth.iter().step_by(4).copied().collect();
        let on = found
            .downbeats
            .iter()
            .filter(|d| nearest(&accents, **d, 0.08).is_some())
            .count();
        assert!(
            on as f64 >= found.downbeats.len() as f64 * 0.9,
            "{on} of {} downbeats on accents",
            found.downbeats.len()
        );
    }

    #[test]
    fn silence_and_short_audio_have_no_beats() {
        assert!(detect(&vec![0.0; RATE as usize * 10], RATE).is_none());
        assert!(detect(&[0.1; 2000], RATE).is_none());
    }

    #[test]
    fn nearest_respects_the_tolerance() {
        let times = [1.0, 2.0, 3.0];
        assert_eq!(nearest(&times, 2.2, 0.25), Some(2.0));
        assert_eq!(nearest(&times, 2.85, 0.25), Some(3.0));
        assert_eq!(nearest(&times, 2.5, 0.2), None);
        assert_eq!(nearest(&[], 1.0, 1.0), None);
    }

    #[test]
    fn fft_matches_a_direct_transform() {
        let n = 16;
        let fft = Fft::new(n);
        let input: Vec<f64> = (0..n).map(|i| ((i * 7) % 5) as f64 - 2.0).collect();
        let (mut re, mut im) = (input.clone(), vec![0.0; n]);
        fft.run(&mut re, &mut im);
        for k in 0..n {
            let (mut r, mut i) = (0.0, 0.0);
            for (t, x) in input.iter().enumerate() {
                let a = -std::f64::consts::TAU * (k * t) as f64 / n as f64;
                r += x * a.cos();
                i += x * a.sin();
            }
            assert!(
                (re[k] - r).abs() < 1e-9 && (im[k] - i).abs() < 1e-9,
                "bin {k}"
            );
        }
    }
}
