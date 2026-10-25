//! Audio filters. Each processes one block in place, without allocating.

use crate::block::Block;
use crate::strong::StrongLink;
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
    /// Named options; when set, the value is the chosen option's index.
    pub choices: &'static [&'static str],
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
        choices: &[],
    }
}

const fn choice(
    key: &'static str,
    label: &'static str,
    choices: &'static [&'static str],
    default: f32,
) -> FilterParam {
    FilterParam {
        key,
        label,
        min: 0.0,
        max: (choices.len() - 1) as f32,
        step: 1.0,
        default,
        unit: "",
        choices,
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
        params: &[
            choice("strength", "Strength", &["Light", "Strong"], NOISE_LIGHT),
            p("amount", "Amount", 0.0, 100.0, 1.0, 100.0, "%"),
        ],
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

// ---------------------------------------------------------------- noise suppression

/// Strength values of the noise suppression `strength` parameter.
pub const NOISE_LIGHT: f32 = 0.0;
pub const NOISE_STRONG: f32 = 1.0;

/// Delay noise suppression adds, in blocks: Light (RNNoise) and Strong (DeepFilterNet3 plus one
/// block of handoff to its thread).
const LIGHT_DELAY: usize = 1;
const STRONG_DELAY: usize = 2;

/// How long Strong delays the voice, for the filter settings.
pub const STRONG_DELAY_MS: u32 = (STRONG_DELAY * 10) as u32;

struct NoiseSuppression {
    states: [Box<nnnoiseless::DenoiseState<'static>>; 2],
    input: [f32; BLOCK],
    output: [f32; BLOCK],
    /// Wet share, 0..=1.
    amount: f32,
    /// Set in Strong; dropping it stops the worker.
    strong: Option<StrongLink>,
    strong_failed: bool,
    /// Light's mono result one block back, standing in for a Strong block that isn't ready.
    light_prev: [f32; BLOCK],
    strong_out: [f32; BLOCK],
    /// The input `0..=STRONG_DELAY` blocks back, for the Amount blend.
    dry: [Block; STRONG_DELAY + 1],
    /// Blocks that used Strong's output (tests).
    strong_blocks: u64,
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
            amount: 1.0,
            strong: None,
            strong_failed: false,
            light_prev: [0.0; BLOCK],
            strong_out: [0.0; BLOCK],
            dry: [Block::SILENT, Block::SILENT, Block::SILENT],
            strong_blocks: 0,
        }
    }

    /// RNNoise on `self.input` (in [-1, 1]) into `self.output`.
    fn light(&mut self, channel: usize) {
        for s in &mut self.input {
            *s *= 32768.0;
        }
        self.states[channel].process_frame(&mut self.output, &self.input);
        for s in &mut self.output {
            *s /= 32768.0;
        }
    }

    fn process_light(&mut self, block: &mut Block) {
        // A mono mic arrives as two equal channels: denoise once and copy.
        let mono = block.l == block.r;
        self.input = block.l;
        self.light(0);
        block.l = self.output;
        if mono {
            block.r = block.l;
        } else {
            self.input = block.r;
            self.light(1);
            block.r = self.output;
        }
    }

    fn process_strong(&mut self, block: &mut Block) {
        for (m, (l, r)) in self.input.iter_mut().zip(block.l.iter().zip(&block.r)) {
            *m = (l + r) * 0.5;
        }
        let mono = self.input;
        // Light keeps running so a missed Strong block has a stand-in, a block later to line up.
        self.light(0);
        let used_strong = self
            .strong
            .as_mut()
            .is_some_and(|link| link.exchange(&mono, &mut self.strong_out));
        let out = if used_strong {
            self.strong_blocks += 1;
            &self.strong_out
        } else {
            &self.light_prev
        };
        block.l = *out;
        block.r = *out;
        self.light_prev = self.output;
    }
}

impl AudioFilter for NoiseSuppression {
    fn update(&mut self, s: &Value) {
        self.amount = param(s, "noise_suppression", "amount").clamp(0.0, 100.0) / 100.0;
        let strong = param(s, "noise_suppression", "strength") >= NOISE_STRONG;
        if !strong {
            self.strong = None;
            self.strong_failed = false;
        } else if self.strong.is_none() && !self.strong_failed {
            // Settings changes run off the steady-state path, so starting a thread here is fine.
            match StrongLink::start() {
                Ok(link) => self.strong = Some(link),
                Err(e) => {
                    tracing::warn!(error = %e, "strong noise suppression thread not started");
                    self.strong_failed = true;
                }
            }
        }
    }

    fn process(&mut self, block: &mut Block) {
        self.dry.rotate_right(1);
        self.dry[0] = block.clone();
        let delay = if self.strong.is_some() {
            self.process_strong(block);
            STRONG_DELAY
        } else {
            self.process_light(block);
            LIGHT_DELAY
        };
        if self.amount < 1.0 {
            let (wet, dry) = (self.amount, 1.0 - self.amount);
            let d = &self.dry[delay];
            for i in 0..BLOCK {
                block.l[i] = block.l[i] * wet + d.l[i] * dry;
                block.r[i] = block.r[i] * wet + d.r[i] * dry;
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

    /// Keyboard-like clicks (a sharp burst into a decaying 3 kHz resonance) every 150 ms over
    /// faint noise, as mono blocks.
    fn typing(blocks: usize) -> Vec<Block> {
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let mut noise = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        let (r, w) = (
            (-1.0f32 / 240.0).exp(),
            std::f32::consts::TAU * 3000.0 / 48_000.0,
        );
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        (0..blocks)
            .map(|n| {
                let mut b = Block::SILENT;
                for i in 0..BLOCK {
                    let t = n * BLOCK + i;
                    let exc = if t % 7200 < 48 { noise() * 0.5 } else { 0.0 };
                    let y = 2.0 * r * w.cos() * y1 - r * r * y2 + exc;
                    (y2, y1) = (y1, y);
                    let s = exc + y * 0.05 + noise() * 0.0005;
                    b.l[i] = s;
                    b.r[i] = s;
                }
                b
            })
            .collect()
    }

    fn strong() -> NoiseSuppression {
        let mut f = NoiseSuppression::new();
        f.update(&json!({ "strength": NOISE_STRONG }));
        f
    }

    /// Runs a block at a pace the worker keeps up with, as the 100 Hz audio thread would.
    fn paced(f: &mut NoiseSuppression, b: &mut Block) {
        f.process(b);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }

    /// Feeds quiet blocks until the Strong model is loaded and answering.
    fn wait_ready(f: &mut NoiseSuppression) {
        for _ in 0..1000 {
            if f.strong_blocks > 0 {
                return;
            }
            paced(f, &mut typing(1)[0]);
        }
        panic!("strong noise suppression never became ready");
    }

    fn energy(blocks: &[Block]) -> f32 {
        blocks.iter().flat_map(|b| b.l.iter()).map(|s| s * s).sum()
    }

    #[test]
    fn strong_removes_more_clicks_than_light() {
        let input = typing(300);
        let mut light = NoiseSuppression::new();
        let mut strong = strong();
        wait_ready(&mut strong);
        let before = strong.strong_blocks;
        let (mut out_light, mut out_strong) = (input.clone(), input.clone());
        for (a, b) in out_light.iter_mut().zip(&mut out_strong) {
            light.process(a);
            paced(&mut strong, b);
        }
        let (e_in, e_light, e_strong) = (
            energy(&input[50..]),
            energy(&out_light[50..]),
            energy(&out_strong[50..]),
        );
        let used = strong.strong_blocks - before;
        assert!(used > 250, "strong used for {used} of 300 blocks");
        assert!(
            e_strong < e_light * 0.1 && e_strong < e_in * 0.1,
            "clicks: in {e_in}, light {e_light}, strong {e_strong}"
        );
    }

    #[test]
    fn strong_outputs_mono() {
        let mut f = strong();
        for n in 0..20 {
            let mut b = sine(0.3, n);
            b.r.iter_mut().for_each(|s| *s *= 0.25);
            paced(&mut f, &mut b);
            assert_eq!(b.l, b.r);
        }
    }

    #[test]
    fn amount_zero_returns_the_input_delayed() {
        let input: Vec<Block> = (0..10).map(|n| sine(0.3, n)).collect();
        for (strength, delay) in [(NOISE_LIGHT, LIGHT_DELAY), (NOISE_STRONG, STRONG_DELAY)] {
            let mut f = NoiseSuppression::new();
            f.update(&json!({ "strength": strength, "amount": 0.0 }));
            for (n, block) in input.iter().enumerate() {
                let mut b = block.clone();
                f.process(&mut b);
                let expected = n
                    .checked_sub(delay)
                    .map_or(Block::SILENT, |k| input[k].clone());
                assert_eq!(b, expected, "strength {strength}, block {n}");
            }
        }
    }

    #[test]
    fn strength_is_a_choice_stored_as_its_index() {
        let strength = kind("noise_suppression")
            .and_then(|k| k.params.iter().find(|p| p.key == "strength"))
            .unwrap();
        assert_eq!(strength.choices, ["Light", "Strong"]);
        assert_eq!((strength.min, strength.max, strength.step), (0.0, 1.0, 1.0));
        let saved: Value = serde_json::from_str(r#"{"strength":1.0,"amount":40.0}"#).unwrap();
        assert_eq!(param(&saved, "noise_suppression", "strength"), NOISE_STRONG);
        // Saved before strengths existed.
        let old: Value = serde_json::from_str("{}").unwrap();
        assert_eq!(param(&old, "noise_suppression", "strength"), NOISE_LIGHT);
        assert_eq!(param(&old, "noise_suppression", "amount"), 100.0);
    }

    #[test]
    fn missing_settings_mean_light_at_full_amount() {
        let mut f = NoiseSuppression::new();
        f.update(&Value::Null);
        assert!(f.strong.is_none());
        assert_eq!(f.amount, 1.0);
    }

    #[test]
    fn switching_to_strong_plays_light_until_the_model_is_ready() {
        let input = typing(400);
        let mut light = NoiseSuppression::new();
        let mut f = NoiseSuppression::new();
        let mut light_prev = Block::SILENT;
        let mut waited = 0;
        for (n, block) in input.iter().enumerate() {
            if n == 20 {
                f.update(&json!({ "strength": NOISE_STRONG }));
            }
            let (mut a, mut b) = (block.clone(), block.clone());
            light.process(&mut a);
            paced(&mut f, &mut b);
            if n > 20 && f.strong_blocks == 0 {
                // Strong mode plays Light a block later, matching its own delay.
                assert_eq!(b, light_prev, "block {n}");
                waited += 1;
            }
            assert!(b.l.iter().all(|s| s.is_finite()));
            light_prev = a;
        }
        assert!(
            waited > 0 && f.strong_blocks > 0,
            "waited {waited} blocks, strong {}",
            f.strong_blocks
        );
    }
}
