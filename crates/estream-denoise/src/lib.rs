//! Strong noise suppression: the DeepFilterNet3 low-latency model, run on the CPU with `tract`.
//!
//! The DSP and model runner are vendored from DeepFilterNet
//! (<https://github.com/Rikorose/DeepFilterNet>, MIT OR Apache-2.0, see `LICENSE-MIT` and
//! `LICENSE-APACHE`). A run allocates, so a [`Denoiser`] belongs on its own thread, never the
//! audio thread.

mod dsp;
mod model;

use anyhow::{ensure, Result};
use model::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

/// Samples per hop: 10 ms at 48 kHz, one mixer block.
pub const HOP: usize = 480;

static MODEL: &[u8] = include_bytes!("../models/DeepFilterNet3_ll_onnx.tar.gz");

/// One mono DeepFilterNet3 stream at 48 kHz, processed one 10 ms hop at a time.
pub struct Denoiser {
    model: DfTract,
    noisy: Array2<f32>,
    enhanced: Array2<f32>,
}

impl Denoiser {
    /// Builds the embedded model (about half a second).
    pub fn new() -> Result<Self> {
        let model = DfTract::new(DfParams::from_bytes(MODEL)?, &RuntimeParams::default())?;
        ensure!(
            model.sr == 48_000 && model.hop_size == HOP,
            "model runs at {} Hz with {}-sample hops",
            model.sr,
            model.hop_size
        );
        Ok(Self {
            model,
            noisy: Array2::zeros((1, HOP)),
            enhanced: Array2::zeros((1, HOP)),
        })
    }

    /// How far the output lags the input, in samples.
    pub fn delay(&self) -> usize {
        self.model.fft_size - self.model.hop_size + self.model.lookahead * self.model.hop_size
    }

    /// Caps how much the noise is reduced, in dB; 100 or more means no cap.
    pub fn set_attenuation_limit(&mut self, db: f32) {
        self.model.set_atten_lim(db);
    }

    /// Enhances one hop of samples in [-1, 1].
    pub fn process(&mut self, input: &[f32; HOP], output: &mut [f32; HOP]) -> Result<()> {
        for (d, s) in self.noisy.iter_mut().zip(input) {
            *d = *s;
        }
        self.model
            .process(self.noisy.view(), self.enhanced.view_mut())?;
        for (d, s) in output.iter_mut().zip(self.enhanced.iter()) {
            *d = *s;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_latency_model_loads_with_one_hop_of_delay() {
        let d = Denoiser::new().unwrap();
        assert_eq!(d.delay(), HOP);
    }

    #[test]
    fn steady_noise_is_reduced() {
        let mut d = Denoiser::new().unwrap();
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut noise = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            ((rng >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.05
        };
        let (mut e_in, mut e_out) = (0.0f32, 0.0f32);
        let mut out = [0.0; HOP];
        for n in 0..300 {
            let mut hop = [0.0; HOP];
            hop.iter_mut().for_each(|s| *s = noise());
            d.process(&hop, &mut out).unwrap();
            if n >= 100 {
                e_in += hop.iter().map(|s| s * s).sum::<f32>();
                e_out += out.iter().map(|s| s * s).sum::<f32>();
            }
        }
        assert!(e_out < e_in * 0.1, "noise only reduced {e_in} -> {e_out}");
    }
}
