//! Playing one clip or recording in the studio (the Clips view), with its sound on the
//! monitoring device only: never in the mix, streams or recordings.
//!
//! Decoding is the media sources' [`Player`]; a feeder thread moves its audio to a
//! [`MonitorOutput`]. The player pushes audio [`AUDIO_LEAD`] ahead of its clock, so the feeder
//! keeps that much in reserve: each sample then leaves when its picture is shown.

use crate::player::{AUDIO_LEAD, PlayState, Player, PlayerSettings, PlayerStatus};
use estream_audio::devices::MonitorOutput;
use estream_audio::mixer::MonitorSink;
use estream_audio::{BLOCK, Block, SAMPLE_RATE};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const FEED_EVERY: Duration = Duration::from_millis(5);

/// Playback volume and mute, shared with the feeder thread.
#[derive(Debug)]
struct Gain {
    volume: AtomicU32,
    muted: AtomicBool,
}

impl Gain {
    fn get(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.volume.load(Ordering::Relaxed))
        }
    }
}

pub struct ClipPlayer {
    path: PathBuf,
    player: Arc<Player>,
    gain: Arc<Gain>,
    stop: Arc<AtomicBool>,
    feeder: Option<JoinHandle<()>>,
}

impl ClipPlayer {
    /// Starts playing `path` from the beginning, with sound on `monitor_device` (empty for
    /// the default output).
    pub fn open(path: &Path, monitor_device: &str, volume: f32, muted: bool) -> Self {
        let player = Player::new(PlayerSettings {
            items: vec![path.to_path_buf()],
            looping: false,
            ..Default::default()
        });
        let gain = Arc::new(Gain {
            volume: AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()),
            muted: AtomicBool::new(muted),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let feeder = {
            let (player, gain, stop) = (player.clone(), gain.clone(), stop.clone());
            let device = monitor_device.to_owned();
            std::thread::Builder::new()
                .name("estream-clip-audio".into())
                .spawn(move || feed(&player, MonitorOutput::start(&device), &gain, &stop))
                .ok()
        };
        Self {
            path: path.to_path_buf(),
            player,
            gain,
            stop,
            feeder,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    pub fn status(&self) -> PlayerStatus {
        self.player.status()
    }

    /// Plays or pauses; after the end, plays again from the start.
    pub fn toggle(&self) {
        if self.player.status().state == PlayState::Ended {
            self.player.restart();
        } else {
            self.player.toggle_pause();
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.player.set_paused(paused);
    }

    pub fn seek(&self, position: Duration) {
        if self.player.status().state == PlayState::Ended {
            self.player.restart();
        }
        self.player.seek(position);
    }

    pub fn set_volume(&self, volume: f32) {
        self.gain
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        self.gain.muted.store(muted, Ordering::Relaxed);
    }
}

impl Drop for ClipPlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.feeder.take() {
            let _ = t.join();
        }
        // Release the file now, so the clip can be deleted or replaced straight after.
        self.player.stop_and_join();
    }
}

/// Frames to keep in the player's ring: the audio lead, except near the end (so the last
/// moments play too).
fn reserve(status: &PlayerStatus) -> usize {
    let lead = (AUDIO_LEAD.as_secs_f64() * SAMPLE_RATE as f64) as usize;
    let near_end = status
        .duration
        .is_some_and(|d| status.position + AUDIO_LEAD * 2 >= d);
    if near_end || status.state == PlayState::Ended {
        0
    } else {
        lead
    }
}

fn feed(player: &Player, mut out: impl MonitorSink, gain: &Gain, stop: &AtomicBool) {
    let mut block = Block::default();
    while !stop.load(Ordering::Acquire) {
        let keep = reserve(&player.status());
        while player.buffered_audio() >= keep + BLOCK {
            let n = player.read_audio(&mut block.l, &mut block.r);
            if n == 0 {
                break;
            }
            let g = gain.get();
            for i in 0..BLOCK {
                let live = i < n;
                block.l[i] = if live { block.l[i] * g } else { 0.0 };
                block.r[i] = if live { block.r[i] * g } else { 0.0 };
            }
            out.push(&block);
        }
        std::thread::sleep(FEED_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: PlayState, position: f64, duration: Option<f64>) -> PlayerStatus {
        PlayerStatus {
            state,
            position: Duration::from_secs_f64(position),
            duration: duration.map(Duration::from_secs_f64),
            ..Default::default()
        }
    }

    #[test]
    fn keeps_the_audio_lead_except_at_the_end() {
        let lead = (AUDIO_LEAD.as_secs_f64() * SAMPLE_RATE as f64) as usize;
        assert_eq!(reserve(&status(PlayState::Playing, 1.0, Some(30.0))), lead);
        assert_eq!(reserve(&status(PlayState::Playing, 29.9, Some(30.0))), 0);
        assert_eq!(reserve(&status(PlayState::Ended, 30.0, Some(30.0))), 0);
        assert_eq!(reserve(&status(PlayState::Playing, 1.0, None)), lead);
    }

    /// Plays a real file for a second, muted (`CLUTCH_TEST_CLIP=<path>`, run with `--ignored`).
    #[test]
    #[ignore]
    fn plays_a_real_clip() {
        let Ok(path) = std::env::var("CLUTCH_TEST_CLIP") else {
            return;
        };
        let clip = ClipPlayer::open(Path::new(&path), "", 1.0, true);
        std::thread::sleep(Duration::from_millis(1500));
        let status = clip.status();
        println!("{status:?} sequence={}", clip.player().sequence());
        assert_eq!(status.state, PlayState::Playing, "{:?}", status.error);
        assert!(status.position > Duration::from_millis(500));
        assert!(clip.player().sequence() > 30, "frames are being decoded");
        // The feeder keeps no more than the lead (plus a block) in the player's ring.
        let lead = (AUDIO_LEAD.as_secs_f64() * SAMPLE_RATE as f64) as usize;
        assert!(clip.player().buffered_audio() <= lead + BLOCK * 4);
        let started = std::time::Instant::now();
        drop(clip);
        assert!(started.elapsed() < Duration::from_secs(1), "closing doesn't hang");
    }

    #[test]
    fn mute_silences_without_losing_the_volume() {
        let gain = Gain {
            volume: AtomicU32::new(0.5f32.to_bits()),
            muted: AtomicBool::new(true),
        };
        assert_eq!(gain.get(), 0.0);
        gain.muted.store(false, Ordering::Relaxed);
        assert_eq!(gain.get(), 0.5);
    }
}
