//! Browser source. Pages render in the `estream-browser` helper process (CEF), so a crashing
//! page can't take eStream down; frames and audio come back through shared memory.
//!
//! One helper serves every browser source. A supervisor thread starts it, re-creates every
//! page after a crash (with back-off), and stops it when no pages are left. Sources never
//! block: commands go through a channel and page state is read with `try_lock`.

use crate::media::upload::{CpuTexture, Pixels};
use crossbeam_channel::{Receiver, Sender, unbounded};
use estream_audio::{AudioSource, BLOCK, Block};
use estream_core::{SourceStatus, Timestamp};
use estream_engine::{
    PropKind, Property, PropertySchema, SourceContext, SourceError, SourceFrame, SourceType,
    VideoSource,
};
use estream_gpu::wgpu;
use estream_shm::{AudioReader, FrameReader};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

/// Same default as OBS: transparent page, no margins or scrollbars.
pub const DEFAULT_CSS: &str =
    "body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }";

// ---------------------------------------------------------------- protocol

/// What a page should show (also the `create`/`update` command payload).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageConfig {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub css: String,
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Cmd<'a> {
    Create {
        id: &'a str,
        #[serde(flatten)]
        page: &'a PageConfig,
    },
    Update {
        id: &'a str,
        #[serde(flatten)]
        page: &'a PageConfig,
    },
    Reload {
        id: &'a str,
    },
    Close {
        id: &'a str,
    },
}

#[derive(Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
enum Event {
    Ready,
    FrameBuffer {
        id: String,
        name: String,
        width: u32,
        height: u32,
    },
    AudioBuffer {
        id: String,
        name: String,
    },
    Loaded {
        id: String,
    },
    Error {
        id: String,
        message: String,
    },
}

// ---------------------------------------------------------------- host

/// What the helper has told us about a page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub video: Option<(String, u32, u32)>,
    pub audio: Option<String>,
    pub loaded: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HelperState {
    Starting,
    Running,
    /// Crashed or exited; restarting soon.
    Restarting,
    /// The helper executable can't be started.
    Missing(String),
}

struct State {
    wanted: HashMap<String, PageConfig>,
    pages: HashMap<String, PageState>,
    helper: HelperState,
    /// Bumped whenever the helper (re)starts.
    generation: u64,
    stdin: Option<ChildStdin>,
}

pub struct BrowserHost {
    state: Mutex<State>,
    wake: Condvar,
    commands: Sender<String>,
}

/// The process-wide host (started on first use).
pub fn host() -> Arc<BrowserHost> {
    static HOST: OnceLock<Arc<BrowserHost>> = OnceLock::new();
    HOST.get_or_init(BrowserHost::start).clone()
}

/// `estream-browser.exe` next to this executable (or one level up, for test runners in
/// `target/<profile>/deps`).
fn helper_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "estream-browser.exe"
    } else {
        "estream-browser"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), dir.parent()?.join(name)]
        .into_iter()
        .find(|p| p.exists())
}

impl BrowserHost {
    fn start() -> Arc<Self> {
        let (tx, rx) = unbounded::<String>();
        let host = Arc::new(Self {
            state: Mutex::new(State {
                wanted: HashMap::new(),
                pages: HashMap::new(),
                helper: HelperState::Starting,
                generation: 0,
                stdin: None,
            }),
            wake: Condvar::new(),
            commands: tx,
        });
        {
            let host = host.clone();
            let _ = std::thread::Builder::new()
                .name("estream-browser-host".into())
                .spawn(move || host.supervise());
        }
        {
            let host = host.clone();
            let _ = std::thread::Builder::new()
                .name("estream-browser-cmd".into())
                .spawn(move || host.write_commands(rx));
        }
        host
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, cmd: &Cmd<'_>) {
        if let Ok(line) = serde_json::to_string(cmd) {
            let _ = self.commands.send(line);
        }
    }

    /// Writes queued commands to the current helper's stdin (off the real-time threads).
    fn write_commands(&self, rx: Receiver<String>) {
        while let Ok(line) = rx.recv() {
            let mut state = self.lock();
            if let Some(stdin) = &mut state.stdin
                && writeln!(stdin, "{line}")
                    .and_then(|_| stdin.flush())
                    .is_err()
            {
                state.stdin = None; // the helper is gone; the supervisor restarts it
            }
        }
    }

    /// Shows `config` in page `id` (creating it if needed).
    pub fn set_page(&self, id: &str, config: PageConfig) {
        let mut state = self.lock();
        let existing = state.wanted.insert(id.to_owned(), config.clone());
        match existing {
            Some(old) if old == config => {}
            Some(_) => self.send(&Cmd::Update { id, page: &config }),
            None => {
                self.send(&Cmd::Create { id, page: &config });
                self.wake.notify_all();
            }
        }
    }

    pub fn remove_page(&self, id: &str) {
        let mut state = self.lock();
        if state.wanted.remove(id).is_some() {
            state.pages.remove(id);
            self.send(&Cmd::Close { id });
            self.wake.notify_all();
        }
    }

    pub fn reload(&self, id: &str) {
        self.send(&Cmd::Reload { id });
    }

    /// Page state without waiting (`None` if busy or unknown).
    pub fn page(&self, id: &str) -> Option<(PageState, HelperState, u64)> {
        let state = self.state.try_lock().ok()?;
        Some((
            state.pages.get(id).cloned().unwrap_or_default(),
            state.helper.clone(),
            state.generation,
        ))
    }

    pub fn helper_state(&self) -> HelperState {
        self.lock().helper.clone()
    }

    /// Kills the helper process (tests use this to simulate a crash).
    pub fn kill_helper_for_test(&self) -> bool {
        HELPER_PID.lock().ok().and_then(|p| *p).is_some_and(|pid| {
            Command::new("taskkill")
                .args(["/F", "/PID", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    /// Keeps a helper running while pages exist.
    fn supervise(&self) {
        let mut failures = 0u32;
        loop {
            // Wait until some page is wanted.
            {
                let mut state = self.lock();
                while state.wanted.is_empty() {
                    state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            }
            let Some(path) = helper_path() else {
                self.lock().helper = HelperState::Missing(
                    "The browser helper (estream-browser.exe) is missing next to eStream".into(),
                );
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let child = Command::new(&path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut child: Child = match child {
                Ok(c) => c,
                Err(e) => {
                    self.lock().helper =
                        HelperState::Missing(format!("Couldn't start the browser helper: {e}"));
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            };
            if let Ok(mut p) = HELPER_PID.lock() {
                *p = Some(child.id());
            }
            let stdout = child.stdout.take();
            {
                let mut state = self.lock();
                state.stdin = child.stdin.take();
                state.helper = HelperState::Starting;
                state.generation += 1;
                state.pages.clear();
                // (Re)create every page on the new helper.
                for (id, page) in &state.wanted {
                    self.send(&Cmd::Create { id, page });
                }
            }
            let started = std::time::Instant::now();
            if let Some(stdout) = stdout {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(event) = serde_json::from_str::<Event>(&line) else {
                        continue;
                    };
                    self.apply(event);
                }
            }
            // The helper exited (crash, or we closed it when no pages were left).
            let _ = child.wait();
            let wanted = {
                let mut state = self.lock();
                state.stdin = None;
                state.pages.clear();
                let wanted = !state.wanted.is_empty();
                state.helper = if wanted {
                    HelperState::Restarting
                } else {
                    HelperState::Starting
                };
                wanted
            };
            if wanted {
                failures = if started.elapsed() > Duration::from_secs(30) {
                    0
                } else {
                    failures + 1
                };
                let delay = Duration::from_millis((1000u64 << failures.min(4)).min(15_000));
                tracing::warn!(?delay, "browser helper exited; restarting");
                std::thread::sleep(delay);
            }
        }
    }

    fn apply(&self, event: Event) {
        let mut state = self.lock();
        match event {
            Event::Ready => state.helper = HelperState::Running,
            Event::FrameBuffer {
                id,
                name,
                width,
                height,
            } => {
                state.pages.entry(id).or_default().video = Some((name, width, height));
            }
            Event::AudioBuffer { id, name } => {
                state.pages.entry(id).or_default().audio = Some(name);
            }
            Event::Loaded { id } => {
                let page = state.pages.entry(id).or_default();
                page.loaded = true;
                page.error = None;
            }
            Event::Error { id, message } => {
                state.pages.entry(id).or_default().error = Some(message);
            }
        }
    }
}

static HELPER_PID: Mutex<Option<u32>> = Mutex::new(None);

// ---------------------------------------------------------------- settings

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
struct BrowserSettings {
    url: String,
    file: String,
    width: u32,
    height: u32,
    fps: u32,
    css: String,
    #[serde(rename = "_source_id")]
    source_id: String,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            url: String::new(),
            file: String::new(),
            width: 800,
            height: 600,
            fps: 30,
            css: DEFAULT_CSS.into(),
            source_id: String::new(),
        }
    }
}

impl BrowserSettings {
    fn parse(settings: &Value) -> Self {
        serde_json::from_value(settings.clone()).unwrap_or_default()
    }

    fn page(&self) -> Option<PageConfig> {
        let url = if !self.file.trim().is_empty() {
            let path = PathBuf::from(self.file.trim());
            format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
        } else {
            self.url.trim().to_owned()
        };
        (!url.is_empty()).then(|| PageConfig {
            url,
            width: self.width.clamp(16, 7680),
            height: self.height.clamp(16, 4320),
            fps: self.fps.clamp(1, 60),
            css: self.css.clone(),
        })
    }

    /// The page id: the engine-provided source id (shared by the video and audio sides).
    fn id(&self) -> Option<String> {
        (!self.source_id.is_empty()).then(|| self.source_id.clone())
    }
}

pub fn schema() -> PropertySchema {
    PropertySchema::new(vec![
        Property::text("url", "URL", false, "").hint("Alerts, chat or any web page"),
        Property::new(
            "file",
            "Local file",
            PropKind::Path {
                filters: vec!["html".into(), "htm".into()],
            },
            "",
        )
        .hint("Used instead of the URL when set"),
        Property::int("width", "Width", 16, 7680, 800),
        Property::int("height", "Height", 16, 4320, 600),
        Property::int("fps", "Frame rate", 1, 60, 30),
        Property::text("css", "Custom CSS", true, DEFAULT_CSS),
    ])
}

pub fn source_type() -> SourceType {
    SourceType::new(
        "browser",
        "Browser",
        "Web pages: alerts, chat, widgets (runs in its own process)",
        |_| schema(),
        |_, settings| Ok(Box::new(BrowserVideo::new(settings))),
    )
    .with_audio(|settings| Ok(Box::new(BrowserAudio::new(settings))))
}

/// Reloads the page of browser source `id` (the refresh action).
pub fn reload(id: &str) {
    host().reload(id);
}

// ---------------------------------------------------------------- video side

pub struct BrowserVideo {
    id: String,
    host: Arc<BrowserHost>,
    reader: Option<(String, FrameReader)>,
    generation: u64,
    buffer: Vec<u8>,
    texture: CpuTexture,
    status: SourceStatus,
    has_page: bool,
}

impl BrowserVideo {
    fn new(settings: &Value) -> Self {
        let mut video = Self {
            id: String::new(),
            host: host(),
            reader: None,
            generation: 0,
            buffer: Vec::new(),
            texture: CpuTexture::default(),
            status: SourceStatus::Unavailable("Starting the browser…".into()),
            has_page: false,
        };
        video.apply(settings);
        video
    }

    fn apply(&mut self, settings: &Value) {
        let s = BrowserSettings::parse(settings);
        // Without an engine id (standalone use), keep a stable private one.
        if let Some(id) = s.id() {
            self.id = id;
        } else if self.id.is_empty() {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            self.id = format!(
                "page-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
        }
        match s.page() {
            Some(page) => {
                self.host.set_page(&self.id, page);
                self.has_page = true;
            }
            None => {
                self.host.remove_page(&self.id);
                self.has_page = false;
                self.texture.clear();
                self.status = SourceStatus::Unavailable("Enter a URL or choose a file".into());
            }
        }
    }
}

impl Drop for BrowserVideo {
    fn drop(&mut self) {
        self.host.remove_page(&self.id);
    }
}

impl VideoSource for BrowserVideo {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        self.apply(settings);
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, _: Timestamp) {
        if !self.has_page {
            return;
        }
        let Some((page, helper, generation)) = self.host.page(&self.id) else {
            return;
        };
        self.status = match (&helper, &page.error) {
            (HelperState::Missing(m), _) => SourceStatus::Error(m.clone()),
            (HelperState::Restarting, _) => SourceStatus::Unavailable("Browser restarting…".into()),
            (_, Some(e)) => SourceStatus::Error(e.clone()),
            _ => SourceStatus::Active,
        };
        if generation != self.generation {
            self.generation = generation;
            self.reader = None;
        }
        let Some((name, w, h)) = page.video else {
            return;
        };
        if self.reader.as_ref().is_none_or(|(n, _)| *n != name) {
            self.reader = FrameReader::open(&name, w, h)
                .ok()
                .map(|r| (name.clone(), r));
        }
        let Some((_, reader)) = &mut self.reader else {
            return;
        };
        if reader.read(&mut self.buffer) {
            // Already straight-alpha RGBA (the helper converts).
            let pixels = Pixels {
                width: w,
                height: h,
                rgba: std::mem::take(&mut self.buffer),
            };
            self.texture.upload(ctx, &pixels);
            self.buffer = pixels.rgba;
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.texture.frame()
    }

    fn status(&self) -> SourceStatus {
        self.status.clone()
    }
}

// ---------------------------------------------------------------- audio side

pub struct BrowserAudio {
    id: String,
    host: Arc<BrowserHost>,
    reader: Option<(String, AudioReader)>,
    priming: bool,
}

impl BrowserAudio {
    fn new(settings: &Value) -> Self {
        Self {
            id: BrowserSettings::parse(settings).id().unwrap_or_default(),
            host: host(),
            reader: None,
            priming: true,
        }
    }
}

impl AudioSource for BrowserAudio {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        self.id = BrowserSettings::parse(settings).id().unwrap_or_default();
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        let Some((page, ..)) = self.host.page(&self.id) else {
            return 0;
        };
        let Some(name) = page.audio else {
            self.reader = None;
            return 0;
        };
        if self.reader.as_ref().is_none_or(|(n, _)| *n != name) {
            self.reader = AudioReader::open(&name).ok().map(|r| (name.clone(), r));
            self.priming = true;
        }
        let Some((_, reader)) = &mut self.reader else {
            return 0;
        };
        // Keep ~50 ms buffered so packet jitter from the page doesn't click.
        if self.priming {
            if reader.available() < 2400 {
                return 0;
            }
            self.priming = false;
        }
        let n = reader.read(&mut block.l, &mut block.r);
        if n < BLOCK {
            self.priming = true;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_make_pages() {
        let s = BrowserSettings::parse(
            &serde_json::json!({ "url": " https://example.com ", "width": 99999 }),
        );
        let page = s.page().unwrap_or_else(|| panic!("page"));
        assert_eq!(page.url, "https://example.com");
        assert_eq!(page.width, 7680);
        assert_eq!(page.css, DEFAULT_CSS);
        let f = BrowserSettings::parse(&serde_json::json!({ "file": "C:\\overlays\\alert.html" }));
        assert_eq!(
            f.page().map(|p| p.url),
            Some("file:///C:/overlays/alert.html".into())
        );
        assert!(
            BrowserSettings::parse(&serde_json::json!({}))
                .page()
                .is_none()
        );
    }

    #[test]
    fn commands_serialize_as_flat_json() {
        let page = PageConfig {
            url: "https://x".into(),
            width: 10,
            height: 20,
            fps: 30,
            css: String::new(),
        };
        let json = serde_json::to_string(&Cmd::Create {
            id: "a",
            page: &page,
        })
        .unwrap_or_default();
        assert!(json.contains("\"op\":\"create\""), "{json}");
        assert!(json.contains("\"width\":10"), "{json}");
        let ev: Event = serde_json::from_str(
            r#"{"ev":"frame_buffer","id":"a","name":"n","width":1,"height":2}"#,
        )
        .unwrap_or(Event::Ready);
        assert!(matches!(ev, Event::FrameBuffer { width: 1, .. }));
    }
}
