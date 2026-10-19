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
    let ratio = SAMPLE_RATE as f64 / config.sample_rate as f64;
    // Linear-interpolation resampler for monitoring (cheap; fine for headphones).
    let mut src = vec![0.0f32; BLOCK * 8];
    let mut pos = 0.0f64;
    let mut prev = [0.0f32; 2];
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], _| {
                let frames = out.len() / channels;
                let needed = ((frames as f64 * ratio).ceil() as usize + 2).min(src.len() / 2);
                let got = consumer.read_interleaved(&mut src[..needed * 2]);
                for f in 0..frames {
                    let t = pos + f as f64 * ratio;
                    let i = t.floor() as usize;
                    let frac = (t - i as f64) as f32;
                    let sample = |k: usize, ch: usize| -> f32 {
                        if k == 0 {
                            prev[ch]
                        } else if k - 1 < got {
                            src[(k - 1) * 2 + ch]
                        } else {
                            0.0
                        }
                    };
                    for ch in 0..channels {
                        out[f * channels + ch] = if ch < 2 {
                            let a = sample(i, ch);
                            a + (sample(i + 1, ch) - a) * frac
                        } else {
                            0.0
                        };
                    }
                }
                if got > 0 {
                    prev = [src[(got - 1) * 2], src[(got - 1) * 2 + 1]];
                }
                pos = (pos + frames as f64 * ratio - got as f64).clamp(0.0, 1.0);
            },
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
#[cfg(test)]
mod tests {
    use super::*;

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
