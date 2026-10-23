//! Display, window and game capture sources (Windows.Graphics.Capture).

use super::capture_audio::CaptureAudio;
use super::d3d::D3D11Interop;
use super::enumerate::{self, WindowInfo};
use super::wgc::{RingReader, Target, WgcCapture};
use estream_audio::AudioSource as _;
use estream_core::Timestamp;
use estream_engine::source::parse;
use estream_engine::{
    Property, PropertySchema, SelectOption, SourceContext, SourceError, SourceFrame, SourceStatus,
    SourceType, VideoSource,
};
use estream_gpu::{GpuContext, wgpu};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

type InteropCache = Mutex<Vec<(wgpu::Device, Result<D3D11Interop, String>)>>;
static INTEROP: OnceLock<InteropCache> = OnceLock::new();

/// The D3D11 interop device for gpu (created once per wgpu device).
pub fn interop(gpu: &GpuContext) -> Result<D3D11Interop, String> {
    let cache = INTEROP.get_or_init(Default::default);
    let mut cache = cache
        .lock()
        .map_err(|_| "interop cache poisoned".to_owned())?;
    let entry = match cache.iter().position(|(d, _)| *d == gpu.device) {
        Some(i) => &cache[i].1,
        None => {
            tracing::debug!("capture: creating D3D11 interop");
            cache.push((
                gpu.device.clone(),
                D3D11Interop::new(gpu).map_err(|e| e.to_string()),
            ));
            tracing::debug!("capture: interop created");
            &cache[cache.len() - 1].1
        }
    };
    entry
        .clone()
        .map_err(|e| format!("Screen capture needs the DirectX 12 renderer ({e})"))
}

const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    /// Display capture: monitor device name (empty = primary).
    pub monitor: String,
    /// Window/game capture: window match key (see `enumerate::WindowInfo::key`).
    pub window: String,
    /// Game capture: `"auto"` (any fullscreen app) or `"window"`.
    pub mode: String,
    pub cursor: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            monitor: String::new(),
            window: String::new(),
            mode: "auto".into(),
            cursor: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Display,
    Window,
    Game,
}

impl CaptureKind {
    fn id(self) -> &'static str {
        match self {
            CaptureKind::Display => "display_capture",
            CaptureKind::Window => "window_capture",
            CaptureKind::Game => "game_capture",
        }
    }
}

type CaptureResult = Result<(WgcCapture, Option<WindowInfo>), SourceStatus>;
type PendingCapture = std::sync::mpsc::Receiver<CaptureResult>;

pub struct CaptureSource {
    kind: CaptureKind,
    settings: CaptureSettings,
    capture: Option<WgcCapture>,
    latched_game: Option<WindowInfo>,
    reader: RingReader,
    status: SourceStatus,
    next_attempt: Instant,
    pending: Option<PendingCapture>,
}

impl CaptureSource {
    fn new(kind: CaptureKind) -> Self {
        Self {
            kind,
            settings: CaptureSettings::default(),
            capture: None,
            latched_game: None,
            reader: RingReader::default(),
            status: SourceStatus::Unavailable("Starting capture…".into()),
            next_attempt: Instant::now(),
            pending: None,
        }
    }

    fn check_latched_window(&mut self) {
        if let Some(target) = &self.latched_game
            && !enumerate::is_window_valid(target.hwnd, target.pid)
        {
            self.latched_game = None;
        }
    }

    fn stop(&mut self) {
        self.capture = None;
        self.pending = None;
        self.reader.clear();
    }

    /// Resolves the target and starts the session on a background thread
    /// (window enumeration and WinRT setup take milliseconds — too slow for
    /// the video thread). The result is picked up in `tick`.
    fn start_async(&mut self, gpu: &GpuContext) {
        let (kind, settings, latched, gpu) = (
            self.kind,
            self.settings.clone(),
            self.latched_game.clone(),
            gpu.clone(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("estream-capture-start".into())
            .spawn(move || {
                let _ = tx.send(start_capture(kind, &settings, latched, &gpu));
            });
        if spawned.is_ok() {
            self.pending = Some(rx);
        }
    }
    fn waiting_message(&self) -> String {
        if self.kind == CaptureKind::Game
            && self.settings.mode == "auto"
            && let Some(target) = &self.latched_game
        {
            return format!("Waiting for [{}] {}", target.exe, target.title);
        }
        waiting_message(self.kind, &self.settings)
    }
}

pub(crate) fn waiting_message(kind: CaptureKind, settings: &CaptureSettings) -> String {
    {
        match (kind, settings.mode.as_str()) {
            (CaptureKind::Game, "auto") => "Waiting for a fullscreen game or app".into(),
            _ if settings.window.is_empty() => "Choose a window to capture".into(),
            _ => {
                let (exe, _, title) = enumerate::parse_key(&settings.window);
                format!("Waiting for [{exe}] {title}")
            }
        }
    }
}

impl VideoSource for CaptureSource {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let mut new: CaptureSettings = parse(settings)?;
        if self.kind == CaptureKind::Game && settings.get("cursor").is_none() {
            new.cursor = false;
        }
        let retarget = new.monitor != self.settings.monitor
            || new.window != self.settings.window
            || new.mode != self.settings.mode;
        if retarget {
            self.latched_game = None;
            self.stop();
            self.next_attempt = Instant::now();
        } else if new.cursor != self.settings.cursor
            && let Some(c) = &self.capture
        {
            c.set_cursor(new.cursor);
        }
        self.settings = new;
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, _: Timestamp) {
        if self.capture.as_ref().is_some_and(|c| c.ring.is_closed()) {
            tracing::info!(kind = self.kind.id(), "capture target closed");
            self.stop();
            self.check_latched_window();
            self.status = SourceStatus::Unavailable(self.waiting_message());
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok((capture, latched))) => {
                    tracing::info!(kind = self.kind.id(), "capture started");
                    self.capture = Some(capture);
                    if self.kind == CaptureKind::Game && self.settings.mode == "auto" {
                        self.latched_game = latched;
                    }
                    self.status = SourceStatus::Active;
                    self.pending = None;
                }
                Ok(Err(status)) => {
                    self.check_latched_window();
                    self.status = status;
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        if self.capture.is_none() && self.pending.is_none() && Instant::now() >= self.next_attempt {
            self.next_attempt = Instant::now() + RETRY;
            let needs_window = match self.kind {
                CaptureKind::Display => false,
                CaptureKind::Window => true,
                CaptureKind::Game => self.settings.mode == "window",
            };
            if needs_window && self.settings.window.is_empty() {
                // Nothing chosen yet: no need to enumerate anything.
                self.status = SourceStatus::Unavailable(self.waiting_message());
            } else {
                self.check_latched_window();
                self.start_async(ctx.gpu);
            }
        }
        if let Some(capture) = &self.capture {
            self.reader.tick(&capture.ring, ctx);
            if let Some(e) = capture.ring.error() {
                self.status = SourceStatus::Error(e);
            }
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.reader.frame()
    }

    fn status(&self) -> SourceStatus {
        self.status.clone()
    }
}

/// Finds the capture target and starts a session (runs on a helper thread).
fn start_capture(
    kind: CaptureKind,
    settings: &CaptureSettings,
    latched: Option<WindowInfo>,
    gpu: &GpuContext,
) -> Result<(WgcCapture, Option<WindowInfo>), SourceStatus> {
    let interop = interop(gpu).map_err(SourceStatus::Error)?;
    let waiting = || SourceStatus::Unavailable(waiting_message(kind, settings));
    let find = || enumerate::find_window(&settings.window, &enumerate::windows());
    let (target, target_info) = match kind {
        CaptureKind::Display => {
            let monitors = enumerate::monitors();
            let found = if settings.monitor.is_empty() {
                monitors.into_iter().find(|m| m.primary)
            } else {
                monitors.into_iter().find(|m| m.id == settings.monitor)
            };
            let m = found.ok_or_else(|| {
                SourceStatus::Unavailable(format!("Display {} is not connected", settings.monitor))
            })?;
            (Target::Monitor(m.handle), None)
        }
        CaptureKind::Window => {
            let win = find().ok_or_else(waiting)?;
            (Target::Window(win.hwnd), Some(win))
        }
        CaptureKind::Game => {
            let win = if settings.mode == "window" {
                find().ok_or_else(waiting)?
            } else if let Some(latched) = latched
                && enumerate::is_window_valid(latched.hwnd, latched.pid)
            {
                latched
            } else {
                enumerate::foreground_fullscreen().ok_or_else(waiting)?
            };
            (Target::Window(win.hwnd), Some(win))
        }
    };
    let capture = WgcCapture::start(&interop, target, settings.cursor).map_err(|e| {
        tracing::warn!(kind = kind.id(), error = %e, "capture failed to start");
        SourceStatus::Error(e.to_string())
    })?;
    Ok((capture, target_info))
}

fn window_options() -> Vec<SelectOption> {
    let mut list: Vec<WindowInfo> = enumerate::windows();
    list.sort_by_key(|w| (w.exe.to_lowercase(), w.title.to_lowercase()));
    list.into_iter()
        .map(|w| SelectOption::new(w.key(), w.label()))
        .collect()
}

/// Keeps the currently selected window in the list even if it isn't running.
fn with_current(mut options: Vec<SelectOption>, settings: &Value) -> Vec<SelectOption> {
    if let Some(key) = settings.get("window").and_then(Value::as_str)
        && !key.is_empty()
        && !options.iter().any(|o| o.value == key)
    {
        let (exe, _, title) = enumerate::parse_key(key);
        options.insert(
            0,
            SelectOption::new(key, format!("[{exe}] {title} (not running)")),
        );
    }
    options
}

pub fn source_types() -> Vec<SourceType> {
    let display = SourceType::new(
        "display_capture",
        "Display capture",
        "Capture an entire monitor",
        |_| {
            let options = enumerate::monitors()
                .iter()
                .enumerate()
                .map(|(i, m)| SelectOption::new(m.id.clone(), m.label(i)))
                .collect::<Vec<_>>();
            let default = options
                .first()
                .map(|o| o.value.clone())
                .unwrap_or(Value::from(""));
            PropertySchema::new(vec![
                Property::select("monitor", "Display", options, default),
                Property::bool("cursor", "Capture cursor", true),
            ])
        },
        |ctx, settings| {
            let mut s = CaptureSource::new(CaptureKind::Display);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    );
    let window = SourceType::new(
        "window_capture",
        "Window capture",
        "Capture a single application window",
        |settings| {
            PropertySchema::new(vec![
                Property::select(
                    "window",
                    "Window",
                    with_current(window_options(), settings),
                    "",
                )
                .hint("Re-attaches automatically when the app restarts"),
                Property::bool("cursor", "Capture cursor", true),
                audio_property("The app's sound, on its own fader", true),
            ])
        },
        |ctx, settings| {
            let mut s = CaptureSource::new(CaptureKind::Window);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    )
    .with_audio(|settings| {
        let mut a = CaptureAudio::new(CaptureKind::Window);
        a.update(settings)?;
        Ok(Box::new(a))
    });
    let game = SourceType::new(
        "game_capture",
        "Game capture",
        "Capture fullscreen and borderless games",
        |settings| {
            PropertySchema::new(vec![
                Property::select(
                    "mode",
                    "Mode",
                    vec![
                        SelectOption::new("auto", "Any fullscreen application"),
                        SelectOption::new("window", "Specific window"),
                    ],
                    "auto",
                ),
                Property::select(
                    "window",
                    "Window",
                    with_current(window_options(), settings),
                    "",
                )
                .hint("Used in “Specific window” mode"),
                Property::bool("cursor", "Capture cursor", false),
                audio_property("The game's sound, on its own fader", false),
            ])
        },
        |ctx, settings| {
            let mut s = CaptureSource::new(CaptureKind::Game);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    )
    .with_audio(|settings| {
        let mut a = CaptureAudio::new(CaptureKind::Game);
        a.update(settings)?;
        Ok(Box::new(a))
    });
    let display = display.with_validation(|_| {
        PropertySchema::new(vec![
            Property::select("monitor", "Display", vec![], ""),
            Property::bool("cursor", "Capture cursor", true),
        ])
    });
    let window = window.with_validation(|_| {
        PropertySchema::new(vec![
            Property::select("window", "Window", vec![], ""),
            Property::bool("cursor", "Capture cursor", true),
            Property::bool("audio", "Capture audio", true),
        ])
    });
    let game = game.with_validation(|_| {
        PropertySchema::new(vec![
            Property::select("mode", "Mode", vec![], "auto"),
            Property::select("window", "Window", vec![], ""),
            Property::bool("cursor", "Capture cursor", false),
            Property::bool("audio", "Capture audio", false),
        ])
    });
    vec![display, window, game]
}

/// The "Capture audio" switch of Game and Window capture. New Game captures start with it off (the
/// app asks when the game is already heard elsewhere).
fn audio_property(what: &str, default: bool) -> Property {
    Property::bool("audio", "Capture audio", default).hint(&format!(
        "{what}. Desktop audio leaves it out, so you never hear it twice."
    ))
}
