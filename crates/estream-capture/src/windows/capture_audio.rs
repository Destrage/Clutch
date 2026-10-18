//! Audio for Game and Window capture, and Desktop audio that leaves the captured game out.
//!
//! A capture's audio follows the process whose window the capture shows (in "any fullscreen
//! application" mode it latches onto the fullscreen app and lets go when it exits, like the
//! video does). Sources record what they capture in [`claims`], so:
//! - a capture stays silent when an Application audio source already captures the same app;
//! - Desktop audio on the default device switches to "everything except that process".

use super::app_audio::capture;
use super::app_audio::claims::{self, By};
use super::capture::{CaptureKind, CaptureSettings, waiting_message};
use super::enumerate::{self, WindowInfo};
use estream_audio::devices::DeviceSource;
use estream_audio::ring::StereoConsumer;
use estream_audio::{AudioSource, Block, SAMPLE_RATE};
use estream_core::{SourceStatus, Timestamp};
use estream_engine::{PropertySchema, SourceType};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// Keep at most this much buffered, and this much after trimming (as for device capture).
const MAX_BUFFERED: usize = SAMPLE_RATE as usize * 60 / 1000;
const KEEP_BUFFERED: usize = SAMPLE_RATE as usize * 20 / 1000;

/// Whether the settings turn the capture's audio on (missing = on).
pub fn audio_enabled(settings: &Value) -> bool {
    settings
        .get("audio")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// Reads a block from a capture ring, trimming backlog to bound latency.
fn read_trimmed(consumer: &mut StereoConsumer, block: &mut Block) -> usize {
    let available = consumer.available();
    if available > MAX_BUFFERED {
        consumer.skip(available - KEEP_BUFFERED);
    }
    consumer.read_block(block)
}

/// The window whose process a capture's audio should follow.
fn target(kind: CaptureKind, settings: &CaptureSettings) -> Option<WindowInfo> {
    match (kind, settings.mode.as_str()) {
        (CaptureKind::Display, _) => None,
        (CaptureKind::Game, "auto") => enumerate::foreground_fullscreen(),
        _ if settings.window.is_empty() => None,
        _ => enumerate::find_window(&settings.window, &enumerate::windows()),
    }
}

/// The audio part of a Game or Window capture.
pub struct CaptureAudio {
    kind: CaptureKind,
    /// Settings the worker runs with (`None` = not started).
    current: Option<(bool, CaptureSettings)>,
    stop: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
    rx: Option<Receiver<StereoConsumer>>,
    consumer: Option<StereoConsumer>,
    status: Arc<Mutex<SourceStatus>>,
}

impl CaptureAudio {
    pub fn new(kind: CaptureKind) -> Self {
        Self {
            kind,
            current: None,
            stop: Arc::new(AtomicBool::new(true)),
            pid: Arc::new(AtomicU32::new(0)),
            rx: None,
            consumer: None,
            status: Arc::new(Mutex::new(SourceStatus::Active)),
        }
    }

    fn restart(&mut self, enabled: bool, settings: CaptureSettings) {
        self.stop.store(true, Ordering::Release);
        self.consumer = None;
        self.rx = None;
        self.stop = Arc::new(AtomicBool::new(false));
        self.pid = Arc::new(AtomicU32::new(0));
        self.status = Arc::new(Mutex::new(SourceStatus::Active));
        self.current = Some((enabled, settings.clone()));
        if !enabled {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let (kind, stop, pid, status) = (
            self.kind,
            self.stop.clone(),
            self.pid.clone(),
            self.status.clone(),
        );
        let set = move |s: SourceStatus| {
            if let Ok(mut st) = status.lock() {
                *st = s;
            }
        };
        let _ = std::thread::Builder::new()
            .name("estream-capture-audio".into())
            .spawn(move || {
                // SAFETY: per-thread COM init for WASAPI activation.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                while !stop.load(Ordering::Acquire) {
                    match target(kind, &settings) {
                        Some(w) if claims::by_app(w.pid) => set(SourceStatus::Unavailable(
                            format!("{}'s sound comes from your Application audio source", w.exe),
                        )),
                        Some(w) => {
                            set(SourceStatus::Active);
                            pid.store(w.pid, Ordering::Release);
                            // Hand over to an Application audio source added later.
                            let wanted = || !claims::by_app(w.pid);
                            let result = capture(w.pid, false, &stop, &wanted, &tx);
                            pid.store(0, Ordering::Release);
                            if let Err(e) = result {
                                tracing::info!(exe = %w.exe, "capture audio ended: {e}");
                            }
                        }
                        None => set(SourceStatus::Unavailable(waiting_message(kind, &settings))),
                    }
                    for _ in 0..10 {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
    }
}

impl Drop for CaptureAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl AudioSource for CaptureAudio {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        let enabled = audio_enabled(settings);
        let parsed: CaptureSettings = serde_json::from_value(settings.clone()).unwrap_or_default();
        let next = (enabled, parsed);
        if self.current.as_ref() != Some(&next) {
            // Only targeting matters: the cursor setting alone doesn't restart the audio.
            let same_target = self.current.as_ref().is_some_and(|(e, s)| {
                *e == next.0 && s.window == next.1.window && s.mode == next.1.mode
            });
            if same_target {
                self.current = Some(next);
            } else {
                self.restart(next.0, next.1);
            }
        }
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        if let Some(rx) = &self.rx
            && let Ok(c) = rx.try_recv()
        {
            self.consumer = Some(c);
        }
        let pid = self.pid.load(Ordering::Acquire);
        if pid == 0 {
            return 0;
        }
        let Some(c) = &mut self.consumer else {
            return 0;
        };
        claims::claim(pid, By::Capture);
        read_trimmed(c, block)
    }

    fn status(&self) -> SourceStatus {
        if !self.current.as_ref().is_some_and(|(enabled, _)| *enabled) {
            return SourceStatus::Active;
        }
        self.status
            .try_lock()
            .map(|s| s.clone())
            .unwrap_or(SourceStatus::Active)
    }
}

/// A running "everything except `pid`" capture.
struct Exclusion {
    pid: u32,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    rx: Receiver<StereoConsumer>,
    consumer: Option<StereoConsumer>,
}

impl Exclusion {
    fn start(pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel();
        let (stop_flag, alive_flag) = (stop.clone(), alive.clone());
        let spawned = std::thread::Builder::new()
            .name("estream-desktop-exclude".into())
            .spawn(move || {
                // SAFETY: per-thread COM init for WASAPI activation.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                if let Err(e) = capture(pid, true, &stop_flag, &|| true, &tx) {
                    tracing::info!(pid, "desktop audio exclusion ended: {e}");
                }
                alive_flag.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            alive.store(false, Ordering::Release);
        }
        Self {
            pid,
            stop,
            alive,
            rx,
            consumer: None,
        }
    }
}

impl Drop for Exclusion {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Desktop audio. On the default device it leaves out the process a capture (or an
/// Application audio source) already captures, so that sound isn't heard twice.
pub struct DesktopAudio {
    inner: DeviceSource,
    default_device: bool,
    exclusion: Option<Exclusion>,
    next_check: Instant,
}

impl Default for DesktopAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopAudio {
    pub fn new() -> Self {
        Self {
            inner: DeviceSource::new(estream_audio::devices::Direction::Output),
            default_device: true,
            exclusion: None,
            next_check: Instant::now(),
        }
    }

    /// Starts, switches or stops the exclusion capture (checked four times a second).
    fn follow_claims(&mut self) {
        let now = Instant::now();
        if now < self.next_check {
            return;
        }
        self.next_check = now + Duration::from_millis(250);
        let current = self.exclusion.as_ref().map(|e| e.pid);
        let wanted = if self.default_device {
            match claims::desktop_exclusion() {
                Some(pid) => pid,
                None => current, // busy: try again next time
            }
        } else {
            None
        };
        let dead = self
            .exclusion
            .as_ref()
            .is_some_and(|e| !e.alive.load(Ordering::Acquire));
        if wanted != current || dead {
            self.exclusion = wanted.filter(|_| !dead).map(Exclusion::start);
        }
    }
}

impl AudioSource for DesktopAudio {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        self.default_device = settings
            .get("device")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty();
        self.next_check = Instant::now();
        self.inner.update(settings)
    }

    fn read(&mut self, block: &mut Block, now: Timestamp) -> usize {
        self.follow_claims();
        if let Some(e) = &mut self.exclusion {
            if let Ok(c) = e.rx.try_recv() {
                e.consumer = Some(c);
            }
            if let Some(c) = &mut e.consumer {
                // Keep the plain loopback drained, so switching back has no stale audio.
                let mut scratch = Block::SILENT;
                self.inner.read(&mut scratch, now);
                return read_trimmed(c, block);
            }
        }
        self.inner.read(block, now)
    }

    fn status(&self) -> SourceStatus {
        self.inner.status()
    }
}

/// Desktop audio with the same settings and UI as `plain`, the built-in type.
pub fn desktop_type(plain: SourceType) -> SourceType {
    let (schema, validation) = (plain.clone(), plain.clone());
    SourceType::audio(
        &plain.kind,
        &plain.name,
        &plain.description,
        move |settings| schema.schema(settings),
        |settings| {
            let mut s = DesktopAudio::new();
            s.update(settings)?;
            Ok(Box::new(s))
        },
    )
    .with_validation(move |settings| -> PropertySchema { validation.validation_schema(settings) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_is_on_unless_turned_off() {
        assert!(audio_enabled(&serde_json::json!({})));
        assert!(audio_enabled(&serde_json::json!({ "audio": true })));
        assert!(!audio_enabled(&serde_json::json!({ "audio": false })));
    }

    #[test]
    fn claims_pick_what_desktop_audio_leaves_out() {
        claims::clear();
        assert_eq!(claims::desktop_exclusion(), Some(None));
        claims::claim(42, By::App);
        assert!(claims::by_app(42));
        assert_eq!(claims::desktop_exclusion(), Some(Some(42)));
        // A captured game wins over an app.
        claims::claim(7, By::Capture);
        assert_eq!(claims::desktop_exclusion(), Some(Some(7)));
        assert!(!claims::by_app(7));
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(claims::desktop_exclusion(), Some(None));
    }
}
