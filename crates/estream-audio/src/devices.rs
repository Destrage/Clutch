//! Audio devices via cpal: enumeration, input/loopback capture sources and the
//! monitoring output. cpal streams live on worker threads (they are not `Send`),
//! which also re-open devices that disappear and come back.

use crate::block::Block;
use crate::convert::InputConverter;
use crate::mixer::MonitorSink;
use crate::ring::{StereoConsumer, StereoProducer, stereo_ring};
use crate::source::AudioSource;
use crate::{BLOCK, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use estream_core::{SourceStatus, Timestamp};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Microphones, line inputs.
    Input,
    /// Speakers/headphones (captured via loopback, or used for monitoring).
    Output,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn host() -> cpal::Host {
    cpal::default_host()
}

fn device_name(d: &cpal::Device) -> String {
    d.description()
        .map(|desc| desc.name().to_owned())
        .unwrap_or_else(|_| "Unknown device".into())
}

fn device_id(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|id| id.to_string())
}

/// On PipeWire, cpal also lists virtual "default" entries and apps' playback streams, and
/// counts sinks as inputs (capturing one records what it plays). Microphones are the
/// sources; speakers and headphones (for desktop audio and monitoring) are the sinks.
#[cfg(target_os = "linux")]
fn pipewire_device(d: &cpal::Device, direction: Direction) -> bool {
    use cpal::DeviceDirection;
    let virtual_default = device_id(d).is_some_and(|id| id.ends_with("_default"));
    let Ok(desc) = d.description() else {
        return false;
    };
    !virtual_default
        && match direction {
            Direction::Input => desc.direction() == DeviceDirection::Input,
            Direction::Output => desc.direction() == DeviceDirection::Duplex,
        }
}

#[cfg(target_os = "linux")]
fn on_pipewire(host: &cpal::Host) -> bool {
    host.id() == cpal::HostId::PipeWire
}

pub fn devices(direction: Direction) -> Vec<DeviceInfo> {
    let host = host();
    #[cfg(target_os = "linux")]
    if on_pipewire(&host) {
        let graph = crate::pipewire::snapshot().unwrap_or_default();
        let default = match direction {
            Direction::Input => graph.default_source,
            Direction::Output => graph.default_sink,
        }
        .map(|name| format!("pipewire:{name}"));
        let list = match direction {
            Direction::Input => host.input_devices().map(|d| d.collect::<Vec<_>>()),
            Direction::Output => host.output_devices().map(|d| d.collect::<Vec<_>>()),
        };
        return list
            .unwrap_or_default()
            .iter()
            .filter(|d| pipewire_device(d, direction))
            .filter_map(|d| {
                let id = device_id(d)?;
                Some(DeviceInfo {
                    is_default: Some(&id) == default.as_ref(),
                    name: device_name(d),
                    id,
                })
            })
            .collect();
    }
    let default = match direction {
        Direction::Input => host.default_input_device(),
        Direction::Output => host.default_output_device(),
    }
    .and_then(|d| device_id(&d));
    let list = match direction {
        Direction::Input => host.input_devices().map(|d| d.collect::<Vec<_>>()),
        Direction::Output => host.output_devices().map(|d| d.collect::<Vec<_>>()),
    };
    list.unwrap_or_default()
        .iter()
        .filter_map(|d| {
            let id = device_id(d)?;
            Some(DeviceInfo {
                is_default: Some(&id) == default.as_ref(),
                name: device_name(d),
                id,
            })
        })
        .collect()
}

/// Finds a device by id; an empty id means the system default.
fn find(direction: Direction, id: &str) -> Option<cpal::Device> {
    let host = host();
    #[cfg(target_os = "linux")]
    if id.is_empty() && direction == Direction::Output && on_pipewire(&host) {
        // The default sink itself (it follows default changes): unlike cpal's default
        // output, it can be captured as well as played to.
        return host
            .output_devices()
            .ok()?
            .find(|d| device_id(d).as_deref() == Some("pipewire:sink_default"));
    }
    if id.is_empty() {
        return match direction {
            Direction::Input => host.default_input_device(),
            Direction::Output => host.default_output_device(),
        };
    }
    let list = match direction {
        Direction::Input => host.input_devices().ok()?.collect::<Vec<_>>(),
        Direction::Output => host.output_devices().ok()?.collect::<Vec<_>>(),
    };
    list.into_iter()
        .find(|d| device_id(d).as_deref() == Some(id))
}

// ------------------------------------------------------------------ capture

/// Glitches (xruns) and automatic reroutes are reported through the error
/// callback but the stream keeps running; only a lost device ends it.
fn is_fatal(e: &cpal::Error) -> bool {
    !matches!(
        e.kind(),
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged
    )
}

fn build_capture<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut producer: StereoProducer,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut converter = InputConverter::new(config.channels as usize, config.sample_rate);
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                scratch.clear();
                scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
                converter.push(&scratch, &mut producer);
            },
            move |e| {
                if is_fatal(&e) {
                    tracing::warn!(error = %e, "audio capture stream failed");
                    failed.store(true, Ordering::Release);
                } else {
                    tracing::debug!(error = %e, "audio capture glitch");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

fn open_capture(
    direction: Direction,
    id: &str,
    failed: Arc<AtomicBool>,
) -> Result<(cpal::Stream, StereoConsumer), String> {
    let device = find(direction, id).ok_or_else(|| "Device is not connected".to_owned())?;
    // Capturing an output device's default config opens it in loopback mode.
    let supported = match direction {
        Direction::Input => device.default_input_config(),
        Direction::Output => device.default_output_config(),
    }
    .map_err(|e| e.to_string())?;
    let config = supported.config();
    // 500 ms of headroom; the mixer keeps actual latency far lower.
    let (producer, consumer) = stereo_ring(SAMPLE_RATE as usize / 2);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_capture::<f32>(&device, config, producer, failed),
        SampleFormat::I16 => build_capture::<i16>(&device, config, producer, failed),
        SampleFormat::I32 => build_capture::<i32>(&device, config, producer, failed),
        SampleFormat::U16 => build_capture::<u16>(&device, config, producer, failed),
        other => Err(format!("Unsupported sample format {other:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    tracing::info!(device = %device_name(&device), ?direction, "audio capture started");
    Ok((stream, consumer))
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited < total && !stop.load(Ordering::Acquire) {
        std::thread::sleep(step);
        waited += step;
    }
}

fn spawn_capture(
    direction: Direction,
    id: String,
    tx: Sender<StereoConsumer>,
    status: Arc<Mutex<SourceStatus>>,
) -> Worker {
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("estream-audio-device".into())
            .spawn(move || {
                let set = |s: SourceStatus| {
                    if let Ok(mut st) = status.lock() {
                        *st = s;
                    }
                };
                while !stop.load(Ordering::Acquire) {
                    let failed = Arc::new(AtomicBool::new(false));
                    match open_capture(direction, &id, failed.clone()) {
                        Ok((stream, consumer)) => {
                            if tx.send(consumer).is_err() {
                                return;
                            }
                            set(SourceStatus::Active);
                            while !stop.load(Ordering::Acquire) && !failed.load(Ordering::Acquire) {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            drop(stream);
                            if !stop.load(Ordering::Acquire) {
                                set(SourceStatus::Unavailable("Device disconnected".into()));
                            }
                        }
                        Err(e) => set(SourceStatus::Unavailable(e)),
                    }
                    sleep_unless_stopped(&stop, Duration::from_secs(2));
                }
            })
            .ok()
    };
    Worker { stop, thread }
}

/// Captures an input device, or an output device via loopback ("desktop audio").
pub struct DeviceSource {
    direction: Direction,
    device: Option<String>,
    worker: Option<Worker>,
    rx: Option<Receiver<StereoConsumer>>,
    consumer: Option<StereoConsumer>,
    status: Arc<Mutex<SourceStatus>>,
    last_status: SourceStatus,
}

/// Keep at most this much buffered (bounded latency, absorbs clock drift).
const MAX_BUFFERED: usize = SAMPLE_RATE as usize * 60 / 1000;
/// After trimming, keep this much as jitter headroom.
const KEEP_BUFFERED: usize = SAMPLE_RATE as usize * 20 / 1000;

impl DeviceSource {
    pub fn new(direction: Direction) -> Self {
        Self {
            direction,
            device: None,
            worker: None,
            rx: None,
            consumer: None,
            status: Arc::new(Mutex::new(SourceStatus::Unavailable(
                "Opening device…".into(),
            ))),
            last_status: SourceStatus::Unavailable("Opening device…".into()),
        }
    }
}

impl AudioSource for DeviceSource {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        let id = settings
            .get("device")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if self.device.as_deref() != Some(id.as_str()) {
            self.worker = None;
            self.consumer = None;
            let (tx, rx) = channel();
            self.worker = Some(spawn_capture(
                self.direction,
                id.clone(),
                tx,
                self.status.clone(),
            ));
            self.rx = Some(rx);
            self.device = Some(id);
        }
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        if let Some(rx) = &self.rx
            && let Ok(consumer) = rx.try_recv()
        {
            self.consumer = Some(consumer);
        }
        let Some(consumer) = &mut self.consumer else {
            return 0;
        };
        let available = consumer.available();
        if available > MAX_BUFFERED {
            consumer.skip(available - KEEP_BUFFERED);
        }
        consumer.read_block(block)
    }

    fn status(&self) -> SourceStatus {
        // Never block the audio thread on the status lock.
        match self.status.try_lock() {
            Ok(s) => s.clone(),
            Err(_) => self.last_status.clone(),
        }
    }
}

// ------------------------------------------------------------------ monitoring

/// Plays the monitor mix on an output device. The audio thread only pushes
/// into a ring; the device runs on its own worker thread.
pub struct MonitorOutput {
    producer: StereoProducer,
    _worker: Worker,
}

impl MonitorOutput {
    pub fn start(device_id: &str) -> Self {
        // ~200 ms ring between the mixer and the device.
        let (producer, consumer) = stereo_ring(SAMPLE_RATE as usize / 5);
        let stop = Arc::new(AtomicBool::new(false));
        let id = device_id.to_owned();
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("estream-monitor".into())
                .spawn(move || {
                    let mut consumer = Some(consumer);
                    // Wait for the device to exist, then run until stopped or failed.
                    let device = loop {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(d) = find(Direction::Output, &id) {
                            break d;
                        }
                        sleep_unless_stopped(&stop, Duration::from_secs(2));
                    };
                    let failed = Arc::new(AtomicBool::new(false));
                    let Some(c) = consumer.take() else { return };
                    match open_monitor(&device, c, failed.clone()) {
                        Ok(stream) => {
                            while !stop.load(Ordering::Acquire) && !failed.load(Ordering::Acquire) {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            drop(stream);
                            if failed.load(Ordering::Acquire) {
                                tracing::warn!("monitoring device lost; reselect it to resume");
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "monitoring unavailable"),
                    }
                })
                .ok()
        };
        Self {
            producer,
            _worker: Worker { stop, thread },
        }
    }
}

impl MonitorSink for MonitorOutput {
    fn push(&mut self, block: &Block) {
        self.producer.push_block(block);
    }
}

fn open_monitor(
    device: &cpal::Device,
    mut consumer: StereoConsumer,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    if supported.sample_format() != SampleFormat::F32 {
        return Err(format!(
            "Unsupported output format {:?}",
            supported.sample_format()
        ));
    }
    let config = supported.config();
    let channels = config.channels as usize;
    let mut resampler = MonitorResampler::new(config.sample_rate);
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], _| resampler.fill(&mut consumer, out, channels),
            move |e| {
                if is_fatal(&e) {
                    tracing::warn!(error = %e, "monitoring stream failed");
                    failed.store(true, Ordering::Release);
                }
            },
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    tracing::info!(device = %device_name(device), "monitoring started");
    Ok(stream)
}

/// Ring frames the monitor resampler looks at in one pass.
const MONITOR_WINDOW: usize = BLOCK * 8;
/// Output gain change per frame when fading in after the ring ran dry (~1.3 ms at 48 kHz).
const MONITOR_FADE_STEP: f32 = 1.0 / 64.0;
/// Per-frame decay of the last sound while the ring is dry, so it fades out instead of
/// dropping to zero (which clicks).
const MONITOR_DECAY: f32 = 0.9;

/// Linear-interpolation resampler from the 48 kHz monitor ring to the device rate (cheap;
/// fine for headphones). It takes from the ring exactly the frames it has played past, so
/// the ring drains at the real rate and there are no seams between device callbacks.
struct MonitorResampler {
    /// Ring frames per device frame.
    ratio: f64,
    /// The last frame taken from the ring: index 0 of the next window.
    prev: [f32; 2],
    /// Read position past `prev`, in ring frames (0..=1).
    pos: f64,
    /// Peeked ring frames, interleaved: window index `k` is `src[k - 1]`.
    src: Vec<f32>,
    /// Output gain: drops to 0 when the ring runs dry and ramps back to 1.
    gain: f32,
    /// The last value written per channel, for fading out while the ring is dry.
    last: [f32; 2],
}

impl MonitorResampler {
    fn new(device_rate: u32) -> Self {
        Self {
            ratio: SAMPLE_RATE as f64 / device_rate.max(1) as f64,
            prev: [0.0; 2],
            pos: 0.0,
            src: vec![0.0; MONITOR_WINDOW * 2],
            gain: 1.0,
            last: [0.0; 2],
        }
    }

    /// Fills `out` (interleaved, `channels` per frame; channels past the first two stay
    /// silent) from the ring.
    fn fill(&mut self, consumer: &mut StereoConsumer, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        // A pass needs at most `pos + frames * ratio` (rounded up) ring frames.
        let per_pass = (((MONITOR_WINDOW - 2) as f64 / self.ratio) as usize).max(1);
        for part in out.chunks_mut(per_pass * channels) {
            self.pass(consumer, part, channels);
        }
    }

    fn pass(&mut self, consumer: &mut StereoConsumer, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let end = self.pos + frames as f64 * self.ratio;
        let need = (end.ceil() as usize).min(MONITOR_WINDOW);
        let got = consumer.peek_interleaved(&mut self.src[..need * 2]);
        let (src, prev) = (&self.src, self.prev);
        let sample = |k: usize, ch: usize| {
            if k == 0 {
                prev[ch]
            } else {
                src[(k - 1) * 2 + ch]
            }
        };
        let mut played = frames;
        for f in 0..frames {
            let t = self.pos + f as f64 * self.ratio;
            let i = t as usize;
            if i + 1 > got {
                played = f; // the ring ran dry
                break;
            }
            let frac = (t - i as f64) as f32;
            self.gain = (self.gain + MONITOR_FADE_STEP).min(1.0);
            for ch in 0..channels {
                out[f * channels + ch] = if ch < 2 {
                    let a = sample(i, ch);
                    let v = (a + (sample(i + 1, ch) - a) * frac) * self.gain;
                    self.last[ch] = v;
                    v
                } else {
                    0.0
                };
            }
        }
        for f in played..frames {
            self.gain = 0.0;
            for ch in 0..channels {
                out[f * channels + ch] = if ch < 2 {
                    self.last[ch] *= MONITOR_DECAY;
                    self.last[ch]
                } else {
                    0.0
                };
            }
        }
        // Take what was played past; the frame under the read position becomes `prev`.
        let t_end = self.pos + played as f64 * self.ratio;
        let taken = (t_end as usize).min(got);
        if taken > 0 {
            self.prev = [src[(taken - 1) * 2], src[(taken - 1) * 2 + 1]];
        }
        consumer.skip(taken);
        self.pos = (t_end - taken as f64).clamp(0.0, 1.0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `sizes` device callbacks (stereo) and returns the left channel.
    fn play(r: &mut MonitorResampler, c: &mut StereoConsumer, sizes: &[usize]) -> Vec<f32> {
        let mut left = Vec::new();
        for &n in sizes {
            let mut out = vec![9.0; n * 2];
            r.fill(c, &mut out, 2);
            left.extend(out.iter().step_by(2));
        }
        left
    }

    #[test]
    fn monitor_at_48k_plays_every_frame_once() {
        let (mut p, mut c) = stereo_ring(20_000);
        let input: Vec<f32> = (1..=10_000).flat_map(|i| [i as f32, -(i as f32)]).collect();
        p.push(&input);
        let mut r = MonitorResampler::new(SAMPLE_RATE);
        // Uneven callback sizes, as devices use.
        let sizes = [480, 441, 512, 7, 1000, 480, 333];
        let left = play(&mut r, &mut c, &sizes);
        let total: usize = sizes.iter().sum();
        // The first frame is the silence before the ring; then every input frame in order.
        let expected: Vec<f32> = (0..total).map(|i| i as f32).collect();
        assert_eq!(left, expected);
        // Exactly what was played was taken, plus the next frame (held as `prev`).
        assert_eq!(c.available(), 10_000 - total);
    }

    #[test]
    fn monitor_resamples_without_seams_or_drift() {
        let (mut p, mut c) = stereo_ring(60_000);
        let tone = |i: usize| (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin();
        let input: Vec<f32> = (0..50_000).flat_map(|i| [tone(i), tone(i)]).collect();
        p.push(&input);
        let mut r = MonitorResampler::new(44_100);
        let left = play(&mut r, &mut c, &[441; 100]); // one second at 44.1 kHz
        // A 1 kHz tone at 44.1 kHz moves at most 2π·1000/44100 ≈ 0.143 per frame; a dropped
        // or repeated sample at a callback edge would show as a larger step.
        let worst = left[1..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.15, "seam of {worst}");
        // One second of device time takes one second of ring audio.
        let taken = 50_000 - c.available();
        assert!((47_999..=48_001).contains(&taken), "took {taken} frames");
    }

    #[test]
    fn monitor_fades_around_a_dry_ring() {
        let (mut p, mut c) = stereo_ring(5_000);
        let mut r = MonitorResampler::new(SAMPLE_RATE);
        let mut left = play(&mut r, &mut c, &[480]); // device starts before any sound
        p.push(&[0.5; 1_000]); // 500 frames
        left.extend(play(&mut r, &mut c, &[480, 480])); // runs dry part-way
        p.push(&[0.5; 1_000]);
        left.extend(play(&mut r, &mut c, &[480]));
        let worst = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.06, "step of {worst}");
        assert!(
            left.iter().any(|&v| (v - 0.5).abs() < 1e-6),
            "reaches full level"
        );
    }

    #[test]
    fn enumeration_does_not_crash() {
        for d in devices(Direction::Input)
            .into_iter()
            .chain(devices(Direction::Output))
        {
            assert!(!d.id.is_empty());
        }
    }
}
