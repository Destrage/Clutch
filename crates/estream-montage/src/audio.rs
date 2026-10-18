//! The montage's sound: every segment's audio (at its speed, with transition fades and its
//! gain), the music (with fades, ducked while clips have sound) and the end fade, mixed to
//! 48 kHz stereo for the whole timeline. Montages are short, so the mix is one buffer.

use crate::model::{Montage, Transition};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const RATE: u32 = 48_000;
/// Clip audio louder than this ducks the music.
const DUCK_THRESHOLD_DB: f32 = -35.0;
const DUCK_ATTACK: f32 = 0.15;
const DUCK_RELEASE: f32 = 0.6;
/// 10 ms blocks for the ducking envelope.
const BLOCK: usize = 480;

fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Decoded audio, cached by source range so edits don't decode again.
#[derive(Default)]
pub struct AudioRenderer {
    ranges: HashMap<(PathBuf, u64, u64), Arc<Vec<f32>>>,
    files: HashMap<PathBuf, Arc<Vec<f32>>>,
}

impl AudioRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Interleaved stereo of `path` from `from` to `to` seconds (silence where it has none).
    fn range(&mut self, path: &Path, from: f64, to: f64) -> Arc<Vec<f32>> {
        let key = (
            path.to_path_buf(),
            (from * 1000.0) as u64,
            (to * 1000.0) as u64,
        );
        if let Some(a) = self.ranges.get(&key) {
            return a.clone();
        }
        let frames = ((to - from) * RATE as f64).round().max(0.0) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        let result = estream_media::audio_file::for_each_chunk(path, from, |chunk| {
            let room = frames * 2 - out.len();
            out.extend_from_slice(&chunk[..chunk.len().min(room)]);
            out.len() < frames * 2
        });
        if let Err(e) = result {
            tracing::debug!(error = %e, path = %path.display(), "no audio for a segment");
        }
        out.resize(frames * 2, 0.0);
        let a = Arc::new(out);
        self.ranges.insert(key, a.clone());
        a
    }

    fn file(&mut self, path: &Path) -> Arc<Vec<f32>> {
        if let Some(a) = self.files.get(path) {
            return a.clone();
        }
        let a = Arc::new(estream_media::audio_file::decode_stereo(path).unwrap_or_default());
        self.files.insert(path.to_path_buf(), a.clone());
        a
    }

    /// Drops cached audio the montage no longer uses.
    pub fn forget_unused(&mut self, m: &Montage) {
        self.ranges
            .retain(|(path, ..), _| m.segments.iter().any(|s| &s.source == path));
        self.files
            .retain(|path, _| m.music.as_ref().is_some_and(|mu| &mu.source == path));
    }

    /// The whole montage's mix: interleaved stereo at 48 kHz, as long as the timeline.
    pub fn render(&mut self, m: &Montage) -> Vec<f32> {
        let total = (m.duration() * RATE as f64).round() as usize;
        let mut clips = vec![0.0f32; total * 2];
        let layout = m.layout();
        for p in &layout {
            let seg = &m.segments[p.index];
            let raw = self.range(&seg.source, seg.src_in, seg.src_out);
            let mut audio = retime(&raw, seg.speed);
            let frames = ((p.end - p.start) * RATE as f64).round() as usize;
            audio.resize(frames * 2, 0.0);
            let g = gain(seg.gain_db + m.mix.clip_gain_db);
            // Fades for the transitions on either side.
            let fade_in = p
                .index
                .checked_sub(1)
                .map(|i| (m.transitions[i], m.transition_length(i)));
            let fade_out = (p.index + 1 < m.segments.len())
                .then(|| (m.transitions[p.index], m.transition_length(p.index)));
            let start = (p.start * RATE as f64).round() as usize;
            for f in 0..frames {
                let t = f as f64 / RATE as f64;
                let mut w = g;
                if let Some((kind, len)) = fade_in
                    && len > 0.0
                    && t < len
                {
                    w *= fade_gain(kind, t / len, true);
                }
                if let Some((kind, len)) = fade_out
                    && len > 0.0
                {
                    let from_end = (p.end - p.start) - t;
                    if from_end < len {
                        w *= fade_gain(kind, 1.0 - from_end / len, false);
                    }
                }
                let o = (start + f) * 2;
                if o + 1 < clips.len() {
                    clips[o] += audio[f * 2] * w;
                    clips[o + 1] += audio[f * 2 + 1] * w;
                }
            }
        }

        let mut out = clips.clone();
        if let Some(music) = &m.music
            && !music.missing
        {
            let file = self.file(&music.source);
            let offset = (music.src_in.max(0.0) * RATE as f64).round() as usize;
            let duck = duck_envelope(&clips, m.mix.duck_db);
            let g = gain(music.gain_db);
            let duration = total as f64 / RATE as f64;
            for f in 0..total {
                let t = f as f64 / RATE as f64;
                let i = (offset + f) * 2;
                if i + 1 >= file.len() {
                    break;
                }
                let mut w = g * duck[f / BLOCK];
                if music.fade_in > 0.0 {
                    w *= (t / music.fade_in as f64).min(1.0) as f32;
                }
                if music.fade_out > 0.0 {
                    w *= ((duration - t) / music.fade_out as f64).clamp(0.0, 1.0) as f32;
                }
                out[f * 2] += file[i] * w;
                out[f * 2 + 1] += file[i + 1] * w;
            }
        }
        if m.fade_out > 0.0 {
            let n = (m.fade_out as f64 * RATE as f64) as usize;
            for f in total.saturating_sub(n)..total {
                let w = ((total - f) as f32 / n as f32).clamp(0.0, 1.0);
                out[f * 2] *= w;
                out[f * 2 + 1] *= w;
            }
        }
        out
    }
}

/// Gain for a transition at `progress` (0..1) through it, for the incoming segment
/// (`incoming`) or the outgoing one.
fn fade_gain(kind: Transition, progress: f64, incoming: bool) -> f32 {
    let p = progress.clamp(0.0, 1.0) as f32;
    match kind {
        Transition::Cut => 1.0,
        // Equal power: the sum keeps a steady loudness.
        Transition::Crossfade { .. } | Transition::Whip { .. } => {
            let x = if incoming { p } else { 1.0 - p };
            (x * std::f32::consts::FRAC_PI_2).sin()
        }
        // Out over the first half, in over the second.
        Transition::DipToBlack { .. } => {
            if incoming {
                ((p - 0.5) * 2.0).clamp(0.0, 1.0)
            } else {
                (1.0 - p * 2.0).clamp(0.0, 1.0)
            }
        }
    }
}

/// Music gain per 10 ms block: down by `duck_db` while clip audio is louder than
/// [`DUCK_THRESHOLD_DB`], with a smooth attack and release.
fn duck_envelope(clips: &[f32], duck_db: f32) -> Vec<f32> {
    let blocks = clips.len() / 2 / BLOCK + 1;
    let low = gain(-duck_db.max(0.0));
    let block_secs = BLOCK as f32 / RATE as f32;
    let attack = 1.0 - (-block_secs / DUCK_ATTACK).exp();
    let release = 1.0 - (-block_secs / DUCK_RELEASE).exp();
    let mut level = 1.0f32;
    let mut out = Vec::with_capacity(blocks);
    for b in 0..blocks {
        let range = b * BLOCK * 2..((b + 1) * BLOCK * 2).min(clips.len());
        let n = range.len().max(1) as f32;
        let rms = (clips[range].iter().map(|s| s * s).sum::<f32>() / n).sqrt();
        let loud = 20.0 * rms.max(1e-9).log10() > DUCK_THRESHOLD_DB;
        let target = if loud { low } else { 1.0 };
        let k = if target < level { attack } else { release };
        level += (target - level) * k;
        out.push(level);
    }
    out
}

/// Plays stereo `audio` at `speed`: unchanged at 1×, time-stretched with the pitch kept
/// (WSOLA) from 0.5× to 2×, and plainly resampled (pitch follows) below 0.5×.
pub fn retime(audio: &[f32], speed: f32) -> Vec<f32> {
    let speed = speed.clamp(0.25, 2.0);
    if (speed - 1.0).abs() < 1e-3 {
        return audio.to_vec();
    }
    if speed < 0.5 {
        return resample(audio, speed);
    }
    wsola(audio, speed)
}

fn resample(audio: &[f32], speed: f32) -> Vec<f32> {
    let frames = audio.len() / 2;
    let out_frames = (frames as f64 / speed as f64).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for f in 0..out_frames {
        let pos = f as f64 * speed as f64;
        let i = pos.floor() as usize;
        let frac = (pos - i as f64) as f32;
        for c in 0..2 {
            let a = audio.get(i * 2 + c).copied().unwrap_or(0.0);
            let b = audio.get((i + 1) * 2 + c).copied().unwrap_or(a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// Waveform-similarity overlap-add: Hann windows of 1024 frames laid down every 512 output
/// frames, each taken from near its nominal input position where it best continues the
/// previous one.
fn wsola(audio: &[f32], speed: f32) -> Vec<f32> {
    const N: usize = 1024;
    const HS: usize = N / 2;
    const DELTA: isize = 256;
    let frames = audio.len() / 2;
    let out_frames = (frames as f64 / speed as f64).round() as usize;
    if frames < N * 2 {
        return resample(audio, speed);
    }
    let window: Vec<f32> = (0..N)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos())
        .collect();
    let mono = |i: usize| (audio[i * 2] + audio[i * 2 + 1]) * 0.5;
    let mut out = vec![0.0f32; (out_frames + N) * 2];
    let mut prev: usize = 0;
    let mut k = 0usize;
    loop {
        let out_pos = k * HS;
        if out_pos >= out_frames {
            break;
        }
        let nominal = (out_pos as f64 * speed as f64) as isize;
        let pos = if k == 0 {
            0
        } else {
            // Find where the input best matches the natural continuation of the last piece.
            let natural = prev + HS;
            let lo = (nominal - DELTA).max(0);
            let hi = (nominal + DELTA).min(frames as isize - N as isize);
            if hi < lo || natural + N > frames {
                break;
            }
            let mut best = (f32::NEG_INFINITY, lo as usize);
            let mut p = lo;
            while p <= hi {
                let p_us = p as usize;
                let mut score = 0.0;
                let mut i = 0;
                while i < N {
                    score += mono(p_us + i) * mono(natural + i);
                    i += 4; // every 4th frame is plenty for alignment
                }
                if score > best.0 {
                    best = (score, p_us);
                }
                p += 2;
            }
            best.1
        };
        if pos + N > frames {
            break;
        }
        for i in 0..N {
            let w = window[i];
            out[(out_pos + i) * 2] += audio[(pos + i) * 2] * w;
            out[(out_pos + i) * 2 + 1] += audio[(pos + i) * 2 + 1] * w;
        }
        prev = pos;
        k += 1;
    }
    // Hann windows at 50 % overlap sum to one, except over the first half window.
    for i in 0..HS.min(out_frames) {
        let w = window[i].max(1e-3);
        out[i * 2] /= w;
        out[i * 2 + 1] /= w;
    }
    out.truncate(out_frames * 2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * RATE as f32) as usize)
            .flat_map(|i| {
                let v = amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin();
                [v, v]
            })
            .collect()
    }

    /// Zero crossings per second of the left channel, from `skip` frames in.
    fn crossing_rate(audio: &[f32], skip: usize) -> f32 {
        let left: Vec<f32> = audio.iter().step_by(2).skip(skip).copied().collect();
        let n = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        n as f32 / (left.len() as f32 / RATE as f32)
    }

    #[test]
    fn slow_motion_keeps_the_pitch() {
        let tone = sine(440.0, 2.0, 0.5);
        let slow = retime(&tone, 0.5);
        assert!(
            ((slow.len() / 2) as i64 - (tone.len() as i64)).abs() < 10,
            "twice as long"
        );
        let rate = crossing_rate(&slow, 2048);
        assert!((rate - 440.0).abs() < 15.0, "pitch kept: {rate} Hz");
        let fast = retime(&tone, 2.0);
        assert!(
            ((fast.len() * 2) as i64 - tone.len() as i64).abs() < 10,
            "half as long"
        );
        assert!((crossing_rate(&fast, 2048) - 440.0).abs() < 15.0);
        // Below half speed the pitch drops with it.
        let very = retime(&tone, 0.25);
        assert!((crossing_rate(&very, 0) - 110.0).abs() < 5.0);
        // Level stays about the same.
        let rms = |a: &[f32]| (a.iter().map(|s| s * s).sum::<f32>() / a.len() as f32).sqrt();
        assert!((rms(&slow[8192..]) / rms(&tone) - 1.0).abs() < 0.15);
    }

    #[test]
    fn transition_fades_keep_or_drop_the_level() {
        // Equal-power crossfade: the two gains' squares add to one.
        for p in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let a = fade_gain(Transition::Crossfade { secs: 1.0 }, p, false);
            let b = fade_gain(Transition::Crossfade { secs: 1.0 }, p, true);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
        let dip = Transition::DipToBlack { secs: 1.0 };
        assert_eq!(fade_gain(dip, 0.5, false), 0.0);
        assert_eq!(fade_gain(dip, 0.5, true), 0.0);
        assert_eq!(fade_gain(Transition::Cut, 0.3, true), 1.0);
    }

    #[test]
    fn music_ducks_under_loud_clips() {
        // One second quiet, one second loud, one second quiet.
        let mut clips = vec![0.0f32; RATE as usize * 2 * 3];
        for (i, s) in clips.iter_mut().enumerate() {
            let t = i / 2;
            if (RATE as usize..2 * RATE as usize).contains(&t) {
                *s = 0.3 * ((i as f32) * 0.05).sin();
            }
        }
        let env = duck_envelope(&clips, 8.0);
        let at = |secs: f32| env[(secs * 100.0) as usize];
        assert!((at(0.9) - 1.0).abs() < 1e-3, "full before");
        assert!(
            (at(1.9) - gain(-8.0)).abs() < 0.02,
            "ducked during: {}",
            at(1.9)
        );
        assert!(at(2.05) < 0.6 && at(2.05) > gain(-8.0), "releasing");
        // A 0.6 s release time constant: ~0.88 a second later, and still rising.
        assert!(
            at(2.95) > 0.85 && at(2.95) > at(2.5),
            "back up after: {}",
            at(2.95)
        );
    }
}
