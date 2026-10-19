//! Voice effects: pitch shift, robot, radio, echo and reverb. Buffers are allocated when a filter
//! is created; `update` and `process` never allocate.

use crate::block::Block;
use crate::filters::AudioFilter;
use crate::{BLOCK, SAMPLE_RATE};
use serde_json::Value;
use std::f32::consts::{PI, TAU};

/// A parameter, or its default from the catalog.
fn param(settings: &Value, kind: &str, key: &str) -> f32 {
    crate::filters::param(settings, kind, key)
}

/// Values this small are flushed to zero, so decaying tails end in true silence (and never
/// turn into slow denormals).
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-12 { 0.0 } else { x }
}

/// A circular delay line with fractional reads.
struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    write: usize,
}

impl DelayLine {
    /// Holds at least `samples` of history (rounded up to a power of two).
    fn new(samples: usize) -> Self {
        let size = samples.next_power_of_two().max(2);
        Self {
            buffer: vec![0.0; size],
            mask: size - 1,
            write: 0,
        }
    }

    fn push(&mut self, x: f32) {
        self.buffer[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }

    /// The sample written `delay` samples before the most recent one (linear interpolation).
    fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(0.0, (self.mask - 1) as f32);
        let whole = delay.floor();
        let frac = delay - whole;
        let newest = self.write.wrapping_sub(1) & self.mask;
        let a = self.buffer[newest.wrapping_sub(whole as usize) & self.mask];
        let b = self.buffer[newest.wrapping_sub(whole as usize + 1) & self.mask];
        a + (b - a) * frac
    }
}

// ---------------------------------------------------------------- pitch shift

/// Grain length of the pitch shifter: long enough for low voices, short enough to stay close to
/// the original timing (the average delay is half of it, ~21 ms).
const GRAIN: f32 = 2048.0;

/// Two read taps sweep through a delay line at the pitch ratio and crossfade, so the voice moves
/// up or down while keeping its timing.
pub(crate) struct PitchShift {
    lines: [DelayLine; 2],
    /// Where the first tap is in its sweep, 0..1 (the second is half a sweep ahead).
    phase: f32,
    step: f32,
    mix: f32,
}

impl PitchShift {
    pub(crate) fn new() -> Self {
        Self {
            lines: [
                DelayLine::new(GRAIN as usize + 4),
                DelayLine::new(GRAIN as usize + 4),
            ],
            phase: 0.0,
            step: 0.0,
            mix: 1.0,
        }
    }
}

impl AudioFilter for PitchShift {
    fn update(&mut self, s: &Value) {
        let ratio = 2f32.powf(param(s, "pitch_shift", "semitones") / 12.0);
        // The delay shrinks (higher) or grows (lower) by (ratio - 1) samples per sample.
        self.step = (1.0 - ratio) / GRAIN;
        self.mix = param(s, "pitch_shift", "mix") / 100.0;
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let (p1, p2) = (self.phase, (self.phase + 0.5).fract());
            // sin² windows half a sweep apart sum to one.
            let (w1, w2) = ((PI * p1).sin().powi(2), (PI * p2).sin().powi(2));
            for (line, sample) in self
                .lines
                .iter_mut()
                .zip([&mut block.l[i], &mut block.r[i]])
            {
                line.push(*sample);
                let wet = line.read(p1 * GRAIN) * w1 + line.read(p2 * GRAIN) * w2;
                *sample = *sample * (1.0 - self.mix) + wet * self.mix;
            }
            self.phase = (self.phase + self.step).rem_euclid(1.0);
        }
    }
}

// ---------------------------------------------------------------- robot

/// Ring modulation: the voice times a low sine, for a metallic, robotic sound.
pub(crate) struct Robot {
    phase: f32,
    step: f32,
    mix: f32,
}

impl Robot {
    pub(crate) fn new() -> Self {
        Self {
            phase: 0.0,
            step: 0.0,
            mix: 1.0,
        }
    }
}

impl AudioFilter for Robot {
    fn update(&mut self, s: &Value) {
        self.step = param(s, "robot", "tone_hz") / SAMPLE_RATE as f32;
        self.mix = param(s, "robot", "mix") / 100.0;
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let carrier = (TAU * self.phase).sin();
            for sample in [&mut block.l[i], &mut block.r[i]] {
                // Ring modulation halves the level on average; make up for it.
                let wet = *sample * carrier * 1.4;
                *sample = *sample * (1.0 - self.mix) + wet * self.mix;
            }
            self.phase = (self.phase + self.step).fract();
        }
    }
}

// ---------------------------------------------------------------- radio

/// A second-order filter (RBJ cookbook), transposed direct form II.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn set(&mut self, high_pass: bool, hz: f32) {
        let w = TAU * hz.clamp(10.0, SAMPLE_RATE as f32 * 0.45) / SAMPLE_RATE as f32;
        let alpha = w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high_pass {
            ((1.0 + cos) / 2.0, -(1.0 + cos))
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos)
        };
        self.b = [b0 / a0, b1 / a0, b0 / a0];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = flush(self.b[1] * x - self.a[0] * y + self.z[1]);
        self.z[1] = flush(self.b[2] * x - self.a[1] * y);
        y
    }
}

/// A narrow band with light distortion: walkie-talkie or telephone.
pub(crate) struct Radio {
    /// High-pass then low-pass, per channel.
    filters: [[Biquad; 2]; 2],
    drive: f32,
    norm: f32,
}

impl Radio {
    pub(crate) fn new() -> Self {
        Self {
            filters: [[Biquad::default(); 2]; 2],
            drive: 1.0,
            norm: 1.0,
        }
    }
}

impl AudioFilter for Radio {
    fn update(&mut self, s: &Value) {
        let (low, high) = (param(s, "radio", "low_hz"), param(s, "radio", "high_hz"));
        for [hp, lp] in &mut self.filters {
            hp.set(true, low);
            lp.set(false, high.max(low + 100.0));
        }
        self.drive = param(s, "radio", "drive").max(1.0);
        self.norm = 1.0 / self.drive.tanh();
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            for (ch, sample) in [&mut block.l[i], &mut block.r[i]].into_iter().enumerate() {
                let [hp, lp] = &mut self.filters[ch];
                let band = lp.run(hp.run(*sample));
                *sample = (band * self.drive).tanh() * self.norm;
            }
        }
    }
}

// ---------------------------------------------------------------- echo

const MAX_ECHO_MS: f32 = 1000.0;

/// Repeats of the voice that fade away.
pub(crate) struct Echo {
    lines: [DelayLine; 2],
    delay: f32,
    feedback: f32,
    mix: f32,
}

impl Echo {
    pub(crate) fn new() -> Self {
        let samples = (MAX_ECHO_MS / 1000.0 * SAMPLE_RATE as f32) as usize + 2;
        Self {
            lines: [DelayLine::new(samples), DelayLine::new(samples)],
            delay: 1.0,
            feedback: 0.0,
            mix: 0.0,
        }
    }
}

impl AudioFilter for Echo {
    fn update(&mut self, s: &Value) {
        let ms = param(s, "echo", "delay_ms").clamp(10.0, MAX_ECHO_MS);
        self.delay = ms / 1000.0 * SAMPLE_RATE as f32;
        self.feedback = (param(s, "echo", "feedback") / 100.0).clamp(0.0, 0.9);
        self.mix = param(s, "echo", "mix") / 100.0;
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            for (line, sample) in self
                .lines
                .iter_mut()
                .zip([&mut block.l[i], &mut block.r[i]])
            {
                // `delay - 1`: the read happens before this sample is written.
                let echo = line.read(self.delay - 1.0);
                line.push(flush(*sample + echo * self.feedback));
                *sample += echo * self.mix;
            }
        }
    }
}

// ---------------------------------------------------------------- reverb

/// Freeverb's tunings at 44.1 kHz (scaled to 48 kHz); the right channel is spread a little.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    store: f32,
}

impl Comb {
    fn run(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buffer[self.index];
        self.store = flush(y * (1.0 - damp) + self.store * damp);
        self.buffer[self.index] = flush(x + self.store * feedback);
        self.index = (self.index + 1) % self.buffer.len();
        y
    }
}

struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}

impl Allpass {
    fn run(&mut self, x: f32) -> f32 {
        let delayed = self.buffer[self.index];
        let y = delayed - x;
        self.buffer[self.index] = flush(x + delayed * 0.5);
        self.index = (self.index + 1) % self.buffer.len();
        y
    }
}

/// A room around the voice: small room to cave.
pub(crate) struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    feedback: f32,
    damp: f32,
    mix: f32,
}

impl Reverb {
    pub(crate) fn new() -> Self {
        let scale = |n: usize| n * SAMPLE_RATE as usize / 44_100;
        let channel = |spread: usize| {
            let combs = COMBS
                .iter()
                .map(|n| Comb {
                    buffer: vec![0.0; scale(n + spread)],
                    index: 0,
                    store: 0.0,
                })
                .collect();
            let allpasses = ALLPASSES
                .iter()
                .map(|n| Allpass {
                    buffer: vec![0.0; scale(n + spread)],
                    index: 0,
                })
                .collect();
            (combs, allpasses)
        };
        let (l_combs, l_all) = channel(0);
        let (r_combs, r_all) = channel(SPREAD);
        Self {
            combs: [l_combs, r_combs],
            allpasses: [l_all, r_all],
            feedback: 0.84,
            damp: 0.2,
            mix: 0.3,
        }
    }
}

impl AudioFilter for Reverb {
    fn update(&mut self, s: &Value) {
        let room = param(s, "reverb", "room") / 100.0;
        self.feedback = 0.7 + 0.28 * room.clamp(0.0, 1.0);
        self.damp = (param(s, "reverb", "damping") / 100.0).clamp(0.0, 1.0) * 0.4;
        self.mix = param(s, "reverb", "mix") / 100.0;
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            // Freeverb feeds the mono sum to both channels' networks.
            let input = (block.l[i] + block.r[i]) * 0.015;
            for (ch, sample) in [&mut block.l[i], &mut block.r[i]].into_iter().enumerate() {
                let mut wet = 0.0;
                for comb in &mut self.combs[ch] {
                    wet += comb.run(input, self.feedback, self.damp);
                }
                for allpass in &mut self.allpasses[ch] {
                    wet = allpass.run(wet);
                }
                *sample = *sample * (1.0 - self.mix * 0.5) + wet * self.mix * 3.0;
            }
        }
    }
}

// ---------------------------------------------------------------- presets

/// One-click voices: `(id, name)`, in menu order (after "Off").
pub const PRESETS: &[(&str, &str)] = &[
    ("deeper", "Deeper"),
    ("higher", "Higher"),
    ("robot", "Robot"),
    ("radio", "Radio"),
    ("cave", "Cave"),
    ("echo", "Echo"),
];

/// The filters a preset adds: `(kind, settings)`. Each settings object also names the preset.
pub fn preset_filters(id: &str) -> Vec<(&'static str, Value)> {
    use serde_json::json;
    let filters = match id {
        "deeper" => vec![("pitch_shift", json!({ "semitones": -4.0 }))],
        "higher" => vec![("pitch_shift", json!({ "semitones": 5.0 }))],
        "robot" => vec![("robot", json!({ "tone_hz": 50.0 }))],
        "radio" => vec![("radio", json!({}))],
        "cave" => vec![
            (
                "reverb",
                json!({ "room": 95.0, "damping": 20.0, "mix": 45.0 }),
            ),
            (
                "echo",
                json!({ "delay_ms": 180.0, "feedback": 30.0, "mix": 20.0 }),
            ),
        ],
        "echo" => vec![(
            "echo",
            json!({ "delay_ms": 350.0, "feedback": 40.0, "mix": 35.0 }),
        )],
        _ => Vec::new(),
    };
    filters
        .into_iter()
        .map(|(kind, mut settings)| {
            settings["preset"] = json!(id);
            (kind, settings)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use crate::block::Block;
    use crate::filters::create;
    use crate::{BLOCK, SAMPLE_RATE};
    use serde_json::json;

    fn sine(hz: f32, amp: f32, block_index: usize) -> Block {
        let mut b = Block::SILENT;
        for i in 0..BLOCK {
            let t = (block_index * BLOCK + i) as f32 / SAMPLE_RATE as f32;
            let s = (t * hz * std::f32::consts::TAU).sin() * amp;
            b.l[i] = s;
            b.r[i] = s;
        }
        b
    }

    /// The dominant frequency of a signal, from its rising zero crossings.
    fn frequency(samples: &[f32]) -> f32 {
        let crossings: Vec<f32> = samples
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
            .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
            .collect();
        let periods = (crossings.len() - 1) as f32;
        periods * SAMPLE_RATE as f32 / (crossings[crossings.len() - 1] - crossings[0])
    }

    #[test]
    fn pitch_shift_moves_the_frequency() {
        for semitones in [-5.0f32, 4.0] {
            let mut f = create("pitch_shift", &json!({ "semitones": semitones })).unwrap();
            let mut out = Vec::new();
            for n in 0..100 {
                let mut b = sine(440.0, 0.5, n);
                f.process(&mut b);
                if n >= 20 {
                    out.extend_from_slice(&b.l);
                }
            }
            let expected = 440.0 * 2f32.powf(semitones / 12.0);
            let got = frequency(&out);
            assert!(
                (got - expected).abs() / expected < 0.02,
                "{semitones} semitones: {got} Hz, expected {expected} Hz"
            );
        }
    }

    #[test]
    fn effects_stay_bounded_and_end_in_silence() {
        for kind in ["pitch_shift", "robot", "radio", "echo", "reverb"] {
            let mut f = create(kind, &json!({})).unwrap();
            for n in 0..50 {
                let mut b = sine(300.0, 0.9, n);
                f.process(&mut b);
                let peak = b.peak()[0].max(b.peak()[1]);
                assert!(peak.is_finite() && peak < 4.0, "{kind}: peak {peak}");
            }
            // Ten seconds of silence: every tail has died away.
            let mut last = Block::SILENT;
            for _ in 0..1000 {
                last = Block::SILENT;
                f.process(&mut last);
            }
            assert!(last.is_silent(), "{kind}: {:?}", last.peak());
        }
    }

    #[test]
    fn presets_make_known_filters() {
        for (id, _) in super::PRESETS {
            let filters = super::preset_filters(id);
            assert!(!filters.is_empty(), "{id}");
            for (kind, settings) in filters {
                assert!(create(kind, &settings).is_some(), "{id}: {kind}");
                assert_eq!(settings["preset"], *id);
            }
        }
    }

    #[test]
    fn echo_repeats_after_the_delay() {
        let mut f = create(
            "echo",
            &json!({ "delay_ms": 100.0, "feedback": 0.0, "mix": 100.0 }),
        )
        .unwrap();
        // One click, then silence: it comes back 100 ms (4800 samples, 10 blocks) later.
        let mut first = Block::SILENT;
        first.l[0] = 1.0;
        f.process(&mut first);
        let mut found = None;
        for n in 1..20 {
            let mut b = Block::SILENT;
            f.process(&mut b);
            if let Some(i) = b.l.iter().position(|s| *s > 0.5) {
                found = Some(n * BLOCK + i);
                break;
            }
        }
        assert_eq!(found, Some(4800));
    }
}
