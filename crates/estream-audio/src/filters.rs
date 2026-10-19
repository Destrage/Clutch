//! Audio filters. Each processes one block in place, without allocating.

use crate::block::Block;
use crate::{BLOCK, db};
use serde_json::Value;

pub trait AudioFilter: Send {
    fn update(&mut self, settings: &Value);
    fn process(&mut self, block: &mut Block);
}

/// A numeric filter parameter (for generic UI and validation).
#[derive(Clone, Debug, PartialEq)]
pub struct FilterParam {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default: f32,
    pub unit: &'static str,
}

const fn p(
    key: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    default: f32,
    unit: &'static str,
) -> FilterParam {
    FilterParam {
        key,
        label,
        min,
        max,
        step,
        default,
        unit,
    }
}

pub struct FilterKind {
    pub kind: &'static str,
    pub name: &'static str,
    pub params: &'static [FilterParam],
}

pub const KINDS: &[FilterKind] = &[
    FilterKind {
        kind: "noise_suppression",
        name: "Noise suppression",
        params: &[],
    },
    FilterKind {
        kind: "noise_gate",
        name: "Noise gate",
        params: &[
            p("open_db", "Open threshold", -96.0, 0.0, 1.0, -26.0, "dB"),
            p("close_db", "Close threshold", -96.0, 0.0, 1.0, -32.0, "dB"),
            p("attack_ms", "Attack", 0.0, 500.0, 1.0, 25.0, "ms"),
            p("hold_ms", "Hold", 0.0, 2000.0, 10.0, 200.0, "ms"),
            p("release_ms", "Release", 0.0, 2000.0, 10.0, 150.0, "ms"),
        ],
    },
    FilterKind {
        kind: "compressor",
        name: "Compressor",
        params: &[
            p("threshold_db", "Threshold", -60.0, 0.0, 1.0, -18.0, "dB"),
            p("ratio", "Ratio", 1.0, 32.0, 0.5, 4.0, ":1"),
            p("attack_ms", "Attack", 0.0, 500.0, 1.0, 6.0, "ms"),
            p("release_ms", "Release", 1.0, 1000.0, 1.0, 60.0, "ms"),
            p("makeup_db", "Output gain", -32.0, 32.0, 0.5, 0.0, "dB"),
        ],
    },
    FilterKind {
        kind: "limiter",
        name: "Limiter",
        params: &[
            p("threshold_db", "Threshold", -60.0, 0.0, 0.5, -6.0, "dB"),
            p("release_ms", "Release", 1.0, 1000.0, 1.0, 60.0, "ms"),
        ],
    },
    FilterKind {
        kind: "gain",
        name: "Gain",
        params: &[p("gain_db", "Gain", -30.0, 30.0, 0.5, 0.0, "dB")],
    },
    FilterKind {
        kind: "pitch_shift",
        name: "Pitch shift",
        params: &[
            p("semitones", "Pitch", -12.0, 12.0, 0.5, -4.0, " st"),
            p("mix", "Mix", 0.0, 100.0, 1.0, 100.0, "%"),
        ],
    },
    FilterKind {
        kind: "robot",
        name: "Robot voice",
        params: &[
            p("tone_hz", "Tone", 20.0, 400.0, 1.0, 50.0, "Hz"),
            p("mix", "Mix", 0.0, 100.0, 1.0, 100.0, "%"),
        ],
    },
    FilterKind {
        kind: "radio",
        name: "Radio voice",
        params: &[
            p("low_hz", "Low cut", 100.0, 1500.0, 10.0, 400.0, "Hz"),
            p("high_hz", "High cut", 1000.0, 8000.0, 50.0, 3000.0, "Hz"),
            p("drive", "Drive", 1.0, 10.0, 0.5, 3.0, "x"),
        ],
    },
    FilterKind {
        kind: "echo",
        name: "Echo",
        params: &[
            p("delay_ms", "Delay", 10.0, 1000.0, 10.0, 300.0, "ms"),
            p("feedback", "Repeats", 0.0, 90.0, 1.0, 40.0, "%"),
            p("mix", "Mix", 0.0, 100.0, 1.0, 35.0, "%"),
        ],
    },
    FilterKind {
        kind: "reverb",
        name: "Reverb",
        params: &[
            p("room", "Room size", 0.0, 100.0, 1.0, 70.0, "%"),
            p("damping", "Damping", 0.0, 100.0, 1.0, 50.0, "%"),
            p("mix", "Mix", 0.0, 100.0, 1.0, 30.0, "%"),
        ],
    },
];

pub fn kind(kind: &str) -> Option<&'static FilterKind> {
    KINDS.iter().find(|k| k.kind == kind)
}

pub(crate) fn param(settings: &Value, kind: &str, key: &str) -> f32 {
    let default = self::kind(kind)
        .and_then(|k| k.params.iter().find(|p| p.key == key))
        .map_or(0.0, |p| p.default);
    settings
        .get(key)
        .and_then(Value::as_f64)
        .map_or(default, |v| v as f32)
}

pub fn create(kind: &str, settings: &Value) -> Option<Box<dyn AudioFilter>> {
    let mut filter: Box<dyn AudioFilter> = match kind {
        "gain" => Box::new(Gain::default()),
        "noise_gate" => Box::new(NoiseGate::default()),
        "compressor" => Box::new(Compressor::default()),
        "limiter" => Box::new(Limiter::default()),
        "noise_suppression" => Box::new(NoiseSuppression::new()),
        "pitch_shift" => Box::new(crate::voice::PitchShift::new()),
        "robot" => Box::new(crate::voice::Robot::new()),
        "radio" => Box::new(crate::voice::Radio::new()),
        "echo" => Box::new(crate::voice::Echo::new()),
        "reverb" => Box::new(crate::voice::Reverb::new()),
        _ => return None,
    };
    filter.update(settings);
    Some(filter)
}

// ---------------------------------------------------------------- gain

#[derive(Default)]
struct Gain {
    gain: f32,
}

impl AudioFilter for Gain {
    fn update(&mut self, s: &Value) {
        self.gain = db::to_gain(param(s, "gain", "gain_db"));
    }
    fn process(&mut self, block: &mut Block) {
        block.scale(self.gain, self.gain);
    }
}

// ---------------------------------------------------------------- gate

#[derive(Default)]
struct NoiseGate {
    open: f32,
    close: f32,
    attack_step: f32,
    release_step: f32,
    hold_samples: u32,
    hold_left: u32,
    is_open: bool,
    gain: f32,
}

impl AudioFilter for NoiseGate {
    fn update(&mut self, s: &Value) {
        self.open = db::to_gain(param(s, "noise_gate", "open_db"));
        self.close = db::to_gain(param(s, "noise_gate", "close_db"));
        let per_ms = crate::SAMPLE_RATE as f32 / 1000.0;
        self.attack_step = 1.0 / (param(s, "noise_gate", "attack_ms") * per_ms).max(1.0);
        self.release_step = 1.0 / (param(s, "noise_gate", "release_ms") * per_ms).max(1.0);
        self.hold_samples = (param(s, "noise_gate", "hold_ms") * per_ms) as u32;
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let level = block.l[i].abs().max(block.r[i].abs());
            if level >= self.open {
                self.is_open = true;
                self.hold_left = self.hold_samples;
            } else if level < self.close && self.is_open {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.is_open = false;
                }
            }
            self.gain = if self.is_open {
                (self.gain + self.attack_step).min(1.0)
            } else {
                (self.gain - self.release_step).max(0.0)
            };
            block.l[i] *= self.gain;
            block.r[i] *= self.gain;
        }
    }
}

// ---------------------------------------------------------------- compressor

#[derive(Default)]
struct Compressor {
    threshold_db: f32,
    slope: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    envelope: f32,
}

impl AudioFilter for Compressor {
    fn update(&mut self, s: &Value) {
        self.threshold_db = param(s, "compressor", "threshold_db");
        self.slope = 1.0 - 1.0 / param(s, "compressor", "ratio").max(1.0);
        self.attack = db::time_coefficient(param(s, "compressor", "attack_ms"));
        self.release = db::time_coefficient(param(s, "compressor", "release_ms"));
        self.makeup = db::to_gain(param(s, "compressor", "makeup_db"));
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let level = block.l[i].abs().max(block.r[i].abs());
            let coeff = if level > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = level + coeff * (self.envelope - level);
            let over = db::from_gain(self.envelope) - self.threshold_db;
            let gain = if over > 0.0 {
                db::to_gain(-over * self.slope)
            } else {
                1.0
            } * self.makeup;
            block.l[i] *= gain;
            block.r[i] *= gain;
        }
    }
}

// ---------------------------------------------------------------- limiter

#[derive(Default)]
struct Limiter {
    threshold: f32,
    release: f32,
    envelope: f32,
}

impl AudioFilter for Limiter {
    fn update(&mut self, s: &Value) {
        self.threshold = db::to_gain(param(s, "limiter", "threshold_db"));
        self.release = db::time_coefficient(param(s, "limiter", "release_ms"));
    }

    fn process(&mut self, block: &mut Block) {
        for i in 0..BLOCK {
            let level = block.l[i].abs().max(block.r[i].abs());
            // Instant attack: the output never exceeds the threshold.
            self.envelope = level.max(self.envelope * self.release);
            let gain = if self.envelope > self.threshold {
                self.threshold / self.envelope
            } else {
                1.0
            };
            block.l[i] *= gain;
            block.r[i] *= gain;
        }
    }
}

// ---------------------------------------------------------------- RNNoise

struct NoiseSuppression {
    states: [Box<nnnoiseless::DenoiseState<'static>>; 2],
    input: [f32; BLOCK],
    output: [f32; BLOCK],
}

impl NoiseSuppression {
    fn new() -> Self {
        Self {
            states: [
                nnnoiseless::DenoiseState::new(),
                nnnoiseless::DenoiseState::new(),
            ],
            input: [0.0; BLOCK],
            output: [0.0; BLOCK],
        }
    }
}

impl AudioFilter for NoiseSuppression {
    fn update(&mut self, _: &Value) {}

    fn process(&mut self, block: &mut Block) {
        // RNNoise works on 480-sample frames at 48 kHz in 16-bit range — exactly one block.
        for (channel, state) in [&mut block.l, &mut block.r]
            .into_iter()
            .zip(self.states.iter_mut())
        {
            for (dst, src) in self.input.iter_mut().zip(channel.iter()) {
                *dst = src * 32768.0;
            }
            state.process_frame(&mut self.output, &self.input);
            for (dst, src) in channel.iter_mut().zip(self.output.iter()) {
                *dst = src / 32768.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sine(amp: f32, block_index: usize) -> Block {
        let mut b = Block::SILENT;
        for i in 0..BLOCK {
            let t = (block_index * BLOCK + i) as f32;
            let s = (t * 1000.0 * std::f32::consts::TAU / 48_000.0).sin() * amp;
            b.l[i] = s;
            b.r[i] = s;
        }
        b
    }

    #[test]
    fn limiter_holds_ceiling() {
        let mut f = create("limiter", &json!({ "threshold_db": -6.0 })).unwrap();
        for n in 0..20 {
            let mut b = sine(1.0, n);
            f.process(&mut b);
            let peak_db = db::from_gain(b.peak()[0]);
            assert!(peak_db <= -6.0 + 0.5, "block {n}: {peak_db} dB");
        }
    }

    #[test]
    fn gate_closes_on_quiet_input_and_opens_on_loud() {
        let mut f = create(
            "noise_gate",
            &json!({ "hold_ms": 50.0, "release_ms": 50.0 }),
        )
        .unwrap();
        // Loud: open.
        for n in 0..5 {
            let mut b = sine(0.5, n);
            f.process(&mut b);
        }
        // Quiet (-60 dB) for 300 ms: closes after hold + release.
        let mut last = Block::SILENT;
        for n in 0..30 {
            last = sine(0.001, n);
            f.process(&mut last);
        }
        assert!(
            last.is_silent() || last.peak()[0] < 1e-6,
            "gate output {:?}",
            last.peak()
        );
        // Loud again: re-opens within the attack time (25 ms).
        let mut b = Block::SILENT;
        for n in 0..4 {
            b = sine(0.5, n);
            f.process(&mut b);
        }
        assert!(b.peak()[0] > 0.45, "gate re-opened: {:?}", b.peak());
    }

    #[test]
    fn compressor_reduces_by_ratio() {
        let mut f = create(
            "compressor",
            &json!({ "threshold_db": -20.0, "ratio": 4.0, "attack_ms": 0.0, "release_ms": 1000.0 }),
        )
        .unwrap();
        let mut b = Block::SILENT;
        for n in 0..50 {
            b = sine(1.0, n); // 0 dB peak: 20 dB over → 15 dB reduction at 4:1
            f.process(&mut b);
        }
        let out_db = db::from_gain(b.peak()[0]);
        assert!((out_db + 15.0).abs() < 1.0, "{out_db} dB");
    }

    #[test]
    fn gain_and_noise_suppression_run() {
        let mut g = create("gain", &json!({ "gain_db": -6.0 })).unwrap();
        let mut b = sine(1.0, 0);
        g.process(&mut b);
        assert!((db::from_gain(b.peak()[0]) + 6.0).abs() < 0.1);

        let mut ns = create("noise_suppression", &Value::Null).unwrap();
        // White-ish noise should be attenuated after a few frames.
        let mut rng = 12345u32;
        let mut energy_in = 0.0;
        let mut energy_out = 0.0;
        for n in 0..50 {
            let mut b = Block::SILENT;
            for i in 0..BLOCK {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let s = ((rng >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.1;
                b.l[i] = s;
                b.r[i] = s;
            }
            if n >= 25 {
                energy_in += b.rms()[0];
            }
            ns.process(&mut b);
            if n >= 25 {
                energy_out += b.rms()[0];
            }
        }
        // At least 3 dB less noise (RNNoise typically removes 5-6 dB of white noise).
        assert!(
            energy_out < energy_in * 0.707,
            "noise reduced: {energy_in} -> {energy_out}"
        );
        assert!(create("nope", &Value::Null).is_none());
    }
}
