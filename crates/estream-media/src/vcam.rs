//! Virtual camera output: sends a canvas to "Clutch Virtual Camera".
//!
//! On Windows 11 a feed thread creates the session camera, then watches the shared memory the
//! camera source opens while an app uses it. Only then does it take canvas frames from the
//! engine, scale them to the size the app asked for, and write them; otherwise it costs
//! nothing. On Linux it writes every canvas frame to a `v4l2loopback` device (which can't tell
//! whether an app is watching).

use crossbeam_channel::RecvTimeoutError;
use estream_core::CanvasId;
use estream_engine::{Engine, VideoTap};
use estream_vcam::shared::{self, MAX_HEIGHT, MAX_WIDTH, Shared, fit_nv12};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum VcamState {
    Starting,
    /// The camera exists; no app is using it.
    On,
    /// An app is receiving frames.
    InUse,
    Failed(String),
}

pub struct VirtualCamOutput {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<VcamState>>,
    canvas: Arc<Mutex<CanvasId>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl VirtualCamOutput {
    /// Turns the camera on (asynchronously: watch [`VirtualCamOutput::state`]).
    pub fn start(engine: &Engine, canvas: CanvasId) -> Self {
        Self::start_with(engine.video_tap(), canvas, shared::MAPPING_NAME, true)
    }

    /// `create_camera: false` skips the system camera (tests drive the shared memory directly).
    pub fn start_with(tap: VideoTap, canvas: CanvasId, mapping: &str, create_camera: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(VcamState::Starting));
        let canvas = Arc::new(Mutex::new(canvas));
        let thread = {
            let feed = Feed {
                tap,
                mapping: mapping.to_owned(),
                stop: stop.clone(),
                state: state.clone(),
                canvas: canvas.clone(),
            };
            std::thread::Builder::new()
                .name("estream-vcam-feed".into())
                .spawn(move || feed.run(create_camera))
                .ok()
        };
        Self {
            stop,
            state,
            canvas,
            thread,
        }
    }

    pub fn state(&self) -> VcamState {
        lock(&self.state).clone()
    }

    /// Switches the canvas sent to the camera.
    pub fn set_canvas(&self, canvas: CanvasId) {
        *lock(&self.canvas) = canvas;
    }

    /// Turns the camera off (apps see it disappear).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for VirtualCamOutput {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Feed {
    tap: VideoTap,
    mapping: String,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<VcamState>>,
    canvas: Arc<Mutex<CanvasId>>,
}

impl Feed {
    /// Linux: writes canvas frames (scaled to fit 1080p) to the loopback device.
    #[cfg(target_os = "linux")]
    fn run_loopback(self) {
        use estream_vcam::control::{Loopback, UNSUPPORTED, loopback_device};
        if loopback_device().is_none() {
            *lock(&self.state) = VcamState::Failed(UNSUPPORTED.into());
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(2);
        let mut canvas = *lock(&self.canvas);
        let mut id = self.tap.add(canvas, tx.clone());
        let mut device: Option<Loopback> = None;
        let mut scaled = Vec::new();
        while !self.stop.load(Ordering::Acquire) {
            let wanted = *lock(&self.canvas);
            if wanted != canvas {
                self.tap.remove(id);
                canvas = wanted;
                id = self.tap.add(canvas, tx.clone());
            }
            let frame = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if device.is_none() {
                // The first canvas fixes the size; later ones are scaled into it.
                let (w, h) = fit_within(frame.width, frame.height, MAX_WIDTH, MAX_HEIGHT);
                match Loopback::open(w, h) {
                    Ok(d) => {
                        tracing::info!("virtual camera on");
                        *lock(&self.state) = VcamState::On;
                        device = Some(d);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "virtual camera didn't start");
                        *lock(&self.state) = VcamState::Failed(e);
                        break;
                    }
                }
            }
            let Some(d) = &mut device else { break };
            scaled.resize(shared::nv12_len(d.width, d.height), 0);
            fit_nv12(
                &frame.data,
                frame.width,
                frame.height,
                &mut scaled,
                d.width,
                d.height,
            );
            if let Err(e) = d.write_nv12(&scaled) {
                *lock(&self.state) =
                    VcamState::Failed(format!("writing to the camera failed: {e}"));
                break;
            }
        }
        self.tap.remove(id);
        tracing::info!("virtual camera off");
    }

    fn run(self, create_camera: bool) {
        #[cfg(target_os = "linux")]
        if create_camera {
            return self.run_loopback();
        }
        #[cfg(windows)]
        let _com = estream_vcam::control::ComApartment::multithreaded();
        #[cfg(windows)]
        let _camera = if create_camera {
            match estream_vcam::control::VirtualCamera::start() {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(error = %e, "virtual camera didn't start");
                    *lock(&self.state) = VcamState::Failed(e.message());
                    return;
                }
            }
        } else {
            None
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        if create_camera {
            *lock(&self.state) = VcamState::Failed(estream_vcam::control::UNSUPPORTED.into());
            return;
        }
        *lock(&self.state) = VcamState::On;
        tracing::info!("virtual camera on");

        let (tx, rx) = crossbeam_channel::bounded(2);
        let mut consumer: Option<(u64, CanvasId)> = None;
        let mut shared: Option<Shared> = None;
        let mut checked_at: Option<Instant> = None;
        let mut scaled = Vec::new();
        while !self.stop.load(Ordering::Acquire) {
            if shared.is_none()
                && checked_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
            {
                checked_at = Some(Instant::now());
                shared = Shared::open(&self.mapping).ok();
            }
            let watching = shared.as_ref().is_some_and(|s| s.consumer_alive(1500));
            let gone = shared.as_ref().is_none_or(|s| !s.consumer_alive(3000));
            let canvas = *lock(&self.canvas);
            match consumer {
                None if watching => {
                    tracing::info!("virtual camera in use");
                    consumer = Some((self.tap.add(canvas, tx.clone()), canvas));
                }
                Some((id, c)) if watching && c != canvas => {
                    self.tap.remove(id);
                    consumer = Some((self.tap.add(canvas, tx.clone()), canvas));
                }
                Some((id, _)) if gone => {
                    tracing::info!("virtual camera no longer in use");
                    self.tap.remove(id);
                    consumer = None;
                    while rx.try_recv().is_ok() {}
                    shared = None; // the source may come back with a fresh mapping
                }
                _ => {}
            }
            *lock(&self.state) = if consumer.is_some() {
                VcamState::InUse
            } else {
                VcamState::On
            };
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => {
                    if let Some(s) = &shared {
                        let (w, h) = s.wanted();
                        if (2..=MAX_WIDTH).contains(&w) && (2..=MAX_HEIGHT).contains(&h) {
                            // Scale outside the seqlock so the reader only waits for a copy.
                            scaled.resize(shared::nv12_len(w, h), 0);
                            fit_nv12(&frame.data, frame.width, frame.height, &mut scaled, w, h);
                            s.write_frame(w, h, |dst| dst.copy_from_slice(&scaled));
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(s) = &shared {
                        s.heartbeat();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some((id, _)) = consumer {
            self.tap.remove(id);
        }
        tracing::info!("virtual camera off");
    }
}

/// The largest even size with `w`×`h`'s aspect that fits in `max_w`×`max_h`.
#[cfg(target_os = "linux")]
fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / w.max(1) as f64)
        .min(max_h as f64 / h.max(1) as f64)
        .min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

#[cfg(all(test, target_os = "linux"))]
mod loopback_tests {
    use super::fit_within;

    #[test]
    fn canvases_fit_1080p() {
        assert_eq!(fit_within(1920, 1080, 1920, 1080), (1920, 1080));
        assert_eq!(fit_within(3840, 2160, 1920, 1080), (1920, 1080));
        assert_eq!(fit_within(1080, 1920, 1920, 1080), (608, 1080));
        assert_eq!(fit_within(1281, 721, 1920, 1080), (1280, 720));
    }
}
