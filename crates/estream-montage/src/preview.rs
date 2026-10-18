//! Playing a montage in the editor: a worker thread renders the frame for the current
//! position into preview textures (the same [`Renderer`] as export) while the audio mix plays
//! on the default output device. The audio is the clock; without an output device a timer
//! stands in. When rendering can't keep up, frames are skipped rather than slowing down.

use crate::audio::{AudioRenderer, RATE};
use crate::model::Montage;
use crate::render::Renderer;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use estream_gpu::{GpuContext, wgpu};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

enum Command {
    SetMontage(Arc<Montage>),
    Play,
    Pause,
    Seek(f64),
    Stop,
}

/// A rendered preview frame.
#[derive(Clone)]
pub struct PreviewFrame {
    pub sequence: u64,
    pub texture: wgpu::Texture,
    pub time: f64,
}

#[derive(Default)]
struct Shared {
    latest: Mutex<Option<PreviewFrame>>,
    /// Playback position in 48 kHz frames (written by the audio callback or the timer).
    position: AtomicU64,
    playing: AtomicBool,
    /// The mix the audio callback reads.
    mix: Mutex<Arc<Vec<f32>>>,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn seconds(&self) -> f64 {
        self.position.load(Ordering::Acquire) as f64 / RATE as f64
    }

    fn set_seconds(&self, t: f64) {
        self.position
            .store((t.max(0.0) * RATE as f64) as u64, Ordering::Release);
    }
}

pub struct Preview {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
    duration: f64,
}

impl Preview {
    pub fn start(gpu: &GpuContext, montage: Arc<Montage>) -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel();
        let duration = montage.duration();
        let thread = {
            let (gpu, shared) = (gpu.clone(), shared.clone());
            std::thread::Builder::new()
                .name("estream-montage-preview".into())
                .spawn(move || run(gpu, montage, shared, rx))
                .ok()
        };
        Self {
            shared,
            commands: tx,
            thread,
            duration,
        }
    }

    /// Shows an edited montage (keeps the position).
    pub fn set_montage(&mut self, montage: Arc<Montage>) {
        self.duration = montage.duration();
        let _ = self.commands.send(Command::SetMontage(montage));
    }

    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn seek(&self, t: f64) {
        self.shared.set_seconds(t.clamp(0.0, self.duration));
        let _ = self
            .commands
            .send(Command::Seek(t.clamp(0.0, self.duration)));
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    /// The playback position in seconds.
    pub fn position(&self) -> f64 {
        self.shared.seconds()
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// The newest rendered frame.
    pub fn latest(&self) -> Option<PreviewFrame> {
        self.shared.latest.lock().ok().and_then(|l| l.clone())
    }

    /// The last rendering problem (a missing file…), if any.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Plays the shared mix from the shared position while `playing`. Returns `None` if there's
/// no usable output device (the worker then keeps time itself).
fn open_output(shared: Arc<Shared>) -> Option<cpal::Stream> {
    let device = cpal::default_host().default_output_device()?;
    let supported = device.default_output_config().ok()?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return None;
    }
    let config = supported.config();
    let channels = config.channels as usize;
    let ratio = RATE as f64 / config.sample_rate as f64;
    // Fractional position kept between callbacks (in 48 kHz frames).
    let mut frac = 0.0f64;
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], _| {
                out.fill(0.0);
                if !shared.playing.load(Ordering::Acquire) {
                    return;
                }
                let Ok(mix) = shared.mix.try_lock().map(|m| m.clone()) else {
                    return;
                };
                let start = shared.position.load(Ordering::Acquire) as f64 + frac;
                let frames = out.len() / channels;
                let total = mix.len() / 2;
                for f in 0..frames {
                    let pos = start + f as f64 * ratio;
                    let i = pos.floor() as usize;
                    let t = (pos - i as f64) as f32;
                    for ch in 0..channels.min(2) {
                        let a = if i < total { mix[i * 2 + ch] } else { 0.0 };
                        let b = if i + 1 < total {
                            mix[(i + 1) * 2 + ch]
                        } else {
                            a
                        };
                        out[f * channels + ch] = a + (b - a) * t;
                    }
                }
                let end = start + frames as f64 * ratio;
                frac = end.fract();
                shared.position.store(end.floor() as u64, Ordering::Release);
            },
            |e| tracing::warn!(error = %e, "montage preview audio failed"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

fn run(gpu: GpuContext, mut montage: Arc<Montage>, shared: Arc<Shared>, rx: Receiver<Command>) {
    let format = montage.format;
    let mut renderer = Renderer::new(&gpu, format.width, format.height);
    let mut targets: Vec<wgpu::Texture> = Vec::new();
    let mut next_target = 0usize;
    let mut audio = AudioRenderer::new();
    let mut mix_for: Option<Arc<Montage>> = None;
    let output = open_output(shared.clone());
    let mut timer: Option<(Instant, f64)> = None;
    let mut sequence = 0u64;
    let mut last_frame: Option<f64> = None;
    let mut dirty = true;

    loop {
        // Commands: take everything waiting; for seeks only the last one matters.
        let wait = if shared.playing.load(Ordering::Acquire) {
            Duration::from_millis(4)
        } else {
            Duration::from_millis(50)
        };
        let mut commands: Vec<Command> = rx.recv_timeout(wait).into_iter().collect();
        commands.extend(rx.try_iter());
        for c in commands {
            match c {
                Command::Stop => return,
                Command::SetMontage(m) => {
                    if m.format != montage.format {
                        renderer = Renderer::new(&gpu, m.format.width, m.format.height);
                        targets.clear();
                    }
                    montage = m;
                    renderer.forget_unused(&montage);
                    audio.forget_unused(&montage);
                    dirty = true;
                }
                Command::Play => {
                    if shared.seconds() >= montage.duration() - 0.01 {
                        shared.set_seconds(0.0);
                    }
                    // A fresh mix if the montage changed since the last one.
                    if mix_for.as_ref().is_none_or(|m| !Arc::ptr_eq(m, &montage)) {
                        let mix = Arc::new(audio.render(&montage));
                        if let Ok(mut slot) = shared.mix.lock() {
                            *slot = mix;
                        }
                        mix_for = Some(montage.clone());
                    }
                    timer = Some((Instant::now(), shared.seconds()));
                    shared.playing.store(true, Ordering::Release);
                }
                Command::Pause => {
                    shared.playing.store(false, Ordering::Release);
                    timer = None;
                }
                Command::Seek(t) => {
                    shared.set_seconds(t);
                    if let Some(timer) = &mut timer {
                        *timer = (Instant::now(), t);
                    }
                    dirty = true;
                }
            }
        }

        let playing = shared.playing.load(Ordering::Acquire);
        // Without an audio device the worker keeps time.
        if playing
            && output.is_none()
            && let Some((since, from)) = timer
        {
            shared.set_seconds(from + since.elapsed().as_secs_f64());
        }
        let duration = montage.duration();
        let mut t = shared.seconds();
        if playing && t >= duration {
            shared.playing.store(false, Ordering::Release);
            t = duration;
            shared.set_seconds(t);
        }
        // Draw when the frame to show changed (frames are skipped, never queued).
        let frame_index = (t * montage.format.fps.max(1) as f64).floor();
        if !dirty && last_frame == Some(frame_index) {
            continue;
        }
        dirty = false;
        last_frame = Some(frame_index);
        let draw_at = t.min((duration - 1e-3).max(0.0));
        match renderer.render(&montage, draw_at) {
            Ok(()) => {
                if let Ok(mut e) = shared.error.lock() {
                    *e = None;
                }
            }
            Err(e) => {
                if let Ok(mut slot) = shared.error.lock() {
                    *slot = Some(e.to_string());
                }
            }
        }
        if targets.is_empty() {
            let (w, h) = renderer.size();
            targets = (0..3)
                .map(|_| estream_gpu::create_preview_texture(&gpu, w, h))
                .collect();
        }
        let target = targets[next_target % targets.len()].clone();
        next_target += 1;
        renderer.to_preview(&estream_gpu::preview_target_view(&target));
        sequence += 1;
        if let Ok(mut latest) = shared.latest.lock() {
            *latest = Some(PreviewFrame {
                sequence,
                texture: target,
                time: t,
            });
        }
    }
}
