//! Converts device audio (any channel count, any rate) to 48 kHz stereo.

use crate::SAMPLE_RATE;
use crate::ring::StereoProducer;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// Lives in a device callback. Allocates only when created.
pub struct InputConverter {
    channels: usize,
    resampler: Option<Async<f32>>,
    /// Stereo frames waiting for a full resampler chunk (interleaved).
    pending: Vec<f32>,
    out: Vec<f32>,
    chunk: usize,
}

impl InputConverter {
    pub fn new(channels: usize, rate: u32) -> Self {
        let chunk = (rate as usize / 100).max(64);
        let resampler = (rate != SAMPLE_RATE).then(|| {
            let params = SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: Some(0.95),
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };
            Async::<f32>::new_sinc(
                SAMPLE_RATE as f64 / rate as f64,
                1.1,
                &params,
                chunk,
                2,
                FixedAsync::Input,
            )
        });
        let resampler = match resampler {
            Some(Ok(r)) => Some(r),
            Some(Err(e)) => {
                tracing::error!(error = %e, rate, "resampler setup failed; audio will be pitched");
                None
            }
            None => None,
        };
        let out_max = resampler.as_ref().map_or(0, |r| r.output_frames_max());
        Self {
            channels: channels.max(1),
            resampler,
            pending: Vec::with_capacity(chunk * 4),
            out: vec![0.0; out_max * 2],
            chunk,
        }
    }

    /// Converts interleaved device samples and pushes them into `producer`.
    pub fn push(&mut self, data: &[f32], producer: &mut StereoProducer) {
        let ch = self.channels;
        let frames = data.len() / ch;
        // Downmix/upmix to stereo: mono is duplicated, >2 channels keep front L/R.
        let stereo = (0..frames).flat_map(|f| {
            let base = f * ch;
            if ch == 1 {
                [data[base], data[base]]
            } else {
                [data[base], data[base + 1]]
            }
        });
        let Some(resampler) = &mut self.resampler else {
            self.pending.clear();
            self.pending.extend(stereo);
            producer.push(&self.pending);
            return;
        };
        self.pending.extend(stereo);
        while self.pending.len() >= self.chunk * 2 {
            let input = InterleavedSlice::new(&self.pending[..self.chunk * 2], 2, self.chunk);
            let out_frames = self.out.len() / 2;
            let output = InterleavedSlice::new_mut(&mut self.out, 2, out_frames);
            match (input, output) {
                (Ok(input), Ok(mut output)) => {
                    if let Ok((_, written)) =
                        resampler.process_into_buffer(&input, &mut output, None)
                    {
                        producer.push(&self.out[..written * 2]);
                    }
                }
                _ => break,
            }
            self.pending.drain(..self.chunk * 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::stereo_ring;

    #[test]
    fn mono_is_duplicated_at_48k() {
        let (mut p, mut c) = stereo_ring(4800);
        let mut conv = InputConverter::new(1, 48_000);
        conv.push(&[0.5; 480], &mut p);
        let mut block = crate::Block::SILENT;
        assert_eq!(c.read_block(&mut block), 480);
        assert_eq!((block.l[10], block.r[10]), (0.5, 0.5));
    }

    #[test]
    fn resamples_44k1_to_48k() {
        let (mut p, c) = stereo_ring(48_000);
        let mut conv = InputConverter::new(2, 44_100);
        // One second of 44.1 kHz stereo.
        let input: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let s = (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5;
                [s, s]
            })
            .collect();
        for chunk in input.chunks(882) {
            conv.push(chunk, &mut p);
        }
        // ~48 000 frames out (minus the resampler's startup delay).
        let got = c.available();
        assert!((46_500..=48_100).contains(&got), "{got}");
    }
}
