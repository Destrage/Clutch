//! Short sounds for the streamer (stream event sounds): decoded once with FFmpeg, mixed on a
//! small thread and played on an output device (the monitoring device), never into the mix.

use crate::audio_file::decode_stereo;
use estream_audio::devices::MonitorOutput;
use estream_audio::mixer::MonitorSink;
use estream_audio::{BLOCK, Block, SAMPLE_RATE};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BLOCK_TIME: Duration = Duration::from_micros(BLOCK as u64 * 1_000_000 / SAMPLE_RATE as u64);

enum Command {
    Play(PathBuf, f32),
    Stop,
}

/// Plays short sound files on one output device. Dropping it stops playback.
pub struct SoundPlayer {
    device: String,
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl SoundPlayer {
    /// Starts a player on `device` (empty for the default output).
    pub fn start(device: &str) -> Self {
        let (tx, rx) = channel();
        let id = device.to_owned();
        let thread = std::thread::Builder::new()
            .name("estream-sounds".into())
            .spawn(move || run(MonitorOutput::start(&id), &rx))
            .ok();
        Self {
            device: device.to_owned(),
            tx,
            thread,
        }
    }

    /// The device it plays on.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Plays `path` once at `volume` (0..1), over anything already playing.
    pub fn play(&self, path: &Path, volume: f32) {
        let _ = self
            .tx
            .send(Command::Play(path.to_path_buf(), volume.clamp(0.0, 1.0)));
    }
}

impl Drop for SoundPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A sound being played: interleaved stereo samples, the next frame, its volume.
struct Voice {
    samples: Arc<Vec<f32>>,
    frame: usize,
    volume: f32,
}

/// Decoded sounds by file (`None`: couldn't be decoded, reported once).
#[derive(Default)]
struct Cache(HashMap<PathBuf, Option<Arc<Vec<f32>>>>);

impl Cache {
    fn get(&mut self, path: &Path) -> Option<Arc<Vec<f32>>> {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| match decode_stereo(path) {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "sound couldn't be played");
                    None
                }
            })
            .clone()
    }
}

/// Mixes the next block of `voices` into `block`, dropping finished ones.
fn mix(voices: &mut Vec<Voice>, block: &mut Block) {
    block.l.fill(0.0);
    block.r.fill(0.0);
    for v in voices.iter_mut() {
        for i in 0..BLOCK {
            let f = v.frame + i;
            let (Some(l), Some(r)) = (v.samples.get(f * 2), v.samples.get(f * 2 + 1)) else {
                break;
            };
            block.l[i] += l * v.volume;
            block.r[i] += r * v.volume;
        }
        v.frame += BLOCK;
    }
    voices.retain(|v| v.frame * 2 < v.samples.len());
    for s in block.l.iter_mut().chain(block.r.iter_mut()) {
        *s = s.clamp(-1.0, 1.0);
    }
}

fn run(mut out: impl MonitorSink, rx: &Receiver<Command>) {
    let mut cache = Cache::default();
    let mut voices: Vec<Voice> = Vec::new();
    let mut block = Block::default();
    let mut next = Instant::now();
    loop {
        // Idle: wait for a sound; playing: take commands without waiting.
        let command = if voices.is_empty() {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            rx.try_recv().ok()
        };
        match command {
            Some(Command::Stop) => return,
            Some(Command::Play(path, volume)) => {
                if let Some(samples) = cache.get(&path) {
                    if voices.is_empty() {
                        next = Instant::now();
                    }
                    voices.push(Voice {
                        samples,
                        frame: 0,
                        volume,
                    });
                }
                continue;
            }
            None => {}
        }
        if voices.is_empty() {
            continue;
        }
        mix(&mut voices, &mut block);
        out.push(&block);
        next += BLOCK_TIME;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(frames: usize, value: f32, volume: f32) -> Voice {
        Voice {
            samples: Arc::new(vec![value; frames * 2]),
            frame: 0,
            volume,
        }
    }

    #[test]
    fn mixes_overlapping_sounds_and_drops_finished_ones() {
        let mut voices = vec![voice(BLOCK + 10, 0.25, 1.0), voice(5, 0.5, 0.5)];
        let mut block = Block::default();
        mix(&mut voices, &mut block);
        assert!((block.l[0] - 0.5).abs() < 1e-6, "0.25 + 0.5 × 0.5");
        assert!((block.l[5] - 0.25).abs() < 1e-6, "the short sound ended");
        assert_eq!(voices.len(), 1);
        mix(&mut voices, &mut block);
        assert!((block.r[9] - 0.25).abs() < 1e-6);
        assert_eq!(block.r[10], 0.0);
        assert!(voices.is_empty());
    }

    #[test]
    fn loud_mixes_are_clamped() {
        let mut voices = vec![voice(BLOCK, 0.9, 1.0), voice(BLOCK, 0.9, 1.0)];
        let mut block = Block::default();
        mix(&mut voices, &mut block);
        assert_eq!(block.l[0], 1.0);
    }
}
