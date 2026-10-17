//! eStream browser helper: renders web pages offscreen with CEF in its own process.
//!
//! Commands arrive as JSON lines on stdin (`create`, `update`, `reload`, `close`); events go
//! out as JSON lines on stdout. Frames are written into named shared memory (seqlocked,
//! straight alpha) and page audio into a shared ring, both read by eStream. Closing stdin
//! (eStream exiting) closes every page and ends the process.

// No console windows for the helper or the CEF subprocesses it relaunches (its pipes to
// eStream are redirected handles, which work for GUI-subsystem processes too).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cef::*;
use estream_shm::{AudioWriter, FrameWriter};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, OnceLock};

const AUDIO_RATE: u32 = 48_000;

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct PageConfig {
    url: String,
    width: u32,
    height: u32,
    fps: u32,
    css: String,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    Create {
        id: String,
        #[serde(flatten)]
        page: PageConfig,
    },
    Update {
        id: String,
        #[serde(flatten)]
        page: PageConfig,
    },
    Reload {
        id: String,
    },
    Close {
        id: String,
    },
}

/// A page and the shared memory it writes into.
struct Page {
    config: PageConfig,
    browser: Option<Browser>,
    frame: Option<FrameWriter>,
    /// Bumped for every new frame mapping (resizes).
    frame_n: u32,
    audio: Option<AudioWriter>,
    audio_channels: usize,
}

type Pages = Mutex<HashMap<String, Page>>;

fn pages() -> &'static Pages {
    static PAGES: OnceLock<Pages> = OnceLock::new();
    PAGES.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Page>> {
    pages().lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes one event line to stdout.
fn emit(event: serde_json::Value) {
    static OUT: Mutex<()> = Mutex::new(());
    let _guard = OUT.lock();
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}

fn mapping_name(id: &str, kind: &str, n: u32) -> String {
    format!(
        "Local\\estream-browser-{}-{id}-{kind}-{n}",
        std::process::id()
    )
}

// ---------------------------------------------------------------- handlers

wrap_render_handler! {
    struct PageRenderHandler {
        id: Arc<String>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let Some(rect) = rect else { return };
            let (w, h) = lock()
                .get(self.id.as_str())
                .map_or((800, 600), |p| (p.config.width, p.config.height));
            rect.x = 0;
            rect.y = 0;
            rect.width = w as i32;
            rect.height = h as i32;
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ != PaintElementType::VIEW || buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let (w, h) = (width as u32, height as u32);
            // SAFETY: CEF hands us a BGRA buffer of width*height*4 bytes for this call.
            let src = unsafe { std::slice::from_raw_parts(buffer, (w * h * 4) as usize) };
            let mut pages = lock();
            let Some(page) = pages.get_mut(self.id.as_str()) else { return };
            if page.frame.as_ref().is_none_or(|f| f.width != w || f.height != h) {
                page.frame_n += 1;
                let name = mapping_name(&self.id, "video", page.frame_n);
                match FrameWriter::create(&name, w, h) {
                    Ok(writer) => {
                        page.frame = Some(writer);
                        emit(serde_json::json!({
                            "ev": "frame_buffer", "id": *self.id, "name": name,
                            "width": w, "height": h,
                        }));
                    }
                    Err(e) => {
                        emit(serde_json::json!({ "ev": "error", "id": *self.id, "message": e.to_string() }));
                        return;
                    }
                }
            }
            if let Some(frame) = &mut page.frame {
                // CEF paints premultiplied BGRA; eStream sources are straight-alpha RGBA.
                frame.write(|dst| unpremultiply(src, dst));
            }
        }
    }
}

/// Converts premultiplied BGRA to straight-alpha RGBA.
fn unpremultiply(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0)
    {
        let a = s[3];
        *d = match a {
            255 => [s[2], s[1], s[0], 255],
            0 => [0, 0, 0, 0],
            _ => {
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                [un(s[2]), un(s[1]), un(s[0]), a]
            }
        };
    }
}

wrap_audio_handler! {
    struct PageAudioHandler {
        id: Arc<String>,
    }

    impl AudioHandler {
        fn audio_parameters(
            &self,
            _browser: Option<&mut Browser>,
            params: Option<&mut AudioParameters>,
        ) -> ::std::os::raw::c_int {
            if let Some(p) = params {
                p.sample_rate = AUDIO_RATE as i32;
                p.channel_layout = ChannelLayout::LAYOUT_STEREO;
            }
            1
        }

        fn on_audio_stream_started(
            &self,
            _browser: Option<&mut Browser>,
            _params: Option<&AudioParameters>,
            channels: ::std::os::raw::c_int,
        ) {
            let mut pages = lock();
            let Some(page) = pages.get_mut(self.id.as_str()) else { return };
            page.audio_channels = channels.max(1) as usize;
            if page.audio.is_none() {
                let name = mapping_name(&self.id, "audio", 1);
                if let Ok(writer) = AudioWriter::create(&name, AUDIO_RATE, AUDIO_RATE) {
                    page.audio = Some(writer);
                    emit(serde_json::json!({ "ev": "audio_buffer", "id": *self.id, "name": name }));
                }
            }
        }

        fn on_audio_stream_packet(
            &self,
            _browser: Option<&mut Browser>,
            data: *mut *const f32,
            frames: ::std::os::raw::c_int,
            _pts: i64,
        ) {
            if data.is_null() || frames <= 0 {
                return;
            }
            let mut pages = lock();
            let Some(page) = pages.get_mut(self.id.as_str()) else { return };
            let channels = page.audio_channels;
            let Some(writer) = &mut page.audio else { return };
            let n = frames as usize;
            // SAFETY: CEF passes `channels` planar float buffers of `frames` samples.
            let planes: Vec<&[f32]> = (0..channels)
                .map(|c| unsafe { std::slice::from_raw_parts(*data.add(c), n) })
                .collect();
            let left = planes[0];
            let right = planes.get(1).copied().unwrap_or(left);
            writer.push(left, right);
        }
    }
}

wrap_load_handler! {
    struct PageLoadHandler {
        id: Arc<String>,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            let Some(frame) = frame else { return };
            if frame.is_main() == 0 {
                return;
            }
            let css = lock()
                .get(self.id.as_str())
                .map(|p| p.config.css.clone())
                .unwrap_or_default();
            if !css.trim().is_empty() {
                let script = format!(
                    "(function(){{var s=document.createElement('style');s.textContent={};\
                     (document.head||document.documentElement).appendChild(s);}})();",
                    serde_json::Value::from(css)
                );
                let code: CefString = script.as_str().into();
                frame.execute_java_script(Some(&code), None, 0);
            }
            emit(serde_json::json!({ "ev": "loaded", "id": *self.id }));
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            if frame.is_none_or(|f| f.is_main() == 0) || error_code == Errorcode::ABORTED {
                return;
            }
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            let url = failed_url.map(|t| t.to_string()).unwrap_or_default();
            emit(serde_json::json!({
                "ev": "error", "id": *self.id,
                "message": format!("Couldn't load {url} ({text})"),
            }));
        }
    }
}

wrap_life_span_handler! {
    struct PageLifeSpanHandler {
        id: Arc<String>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return };
            if let Some(page) = lock().get_mut(self.id.as_str()) {
                page.browser = Some(browser.clone());
            } else {
                // Closed before it finished opening.
                if let Some(host) = browser.host() {
                    host.close_browser(1);
                }
            }
        }
    }
}

wrap_client! {
    struct PageClient {
        render: RenderHandler,
        audio: AudioHandler,
        load: LoadHandler,
        life_span: LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }

        fn audio_handler(&self) -> Option<AudioHandler> {
            Some(self.audio.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life_span.clone())
        }
    }
}

wrap_app! {
    struct HelperApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(cmd) = command_line else { return };
            // Alerts play sound and video without a click.
            let name: CefString = "autoplay-policy".into();
            let value: CefString = "no-user-gesture-required".into();
            cmd.append_switch_with_value(Some(&name), Some(&value));
            // Don't fight eStream for the system's media keys.
            let name: CefString = "disable-features".into();
            let value: CefString = "HardwareMediaKeyHandling,MediaSessionService".into();
            cmd.append_switch_with_value(Some(&name), Some(&value));
        }
    }
}

// ---------------------------------------------------------------- commands

fn create(id: String, config: PageConfig) {
    let fps = config.fps.clamp(1, 60) as i32;
    let url: CefString = config.url.as_str().into();
    lock().insert(
        id.clone(),
        Page {
            config,
            browser: None,
            frame: None,
            frame_n: 0,
            audio: None,
            audio_channels: 2,
        },
    );
    let id = Arc::new(id);
    let mut client = PageClient::new(
        PageRenderHandler::new(id.clone()),
        PageAudioHandler::new(id.clone()),
        PageLoadHandler::new(id.clone()),
        PageLifeSpanHandler::new(id.clone()),
    );
    let window_info = WindowInfo {
        windowless_rendering_enabled: 1,
        ..Default::default()
    };
    let settings = BrowserSettings {
        windowless_frame_rate: fps,
        background_color: 0, // transparent
        ..Default::default()
    };
    let ok = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url),
        Some(&settings),
        None,
        None,
    );
    if ok == 0 {
        emit(
            serde_json::json!({ "ev": "error", "id": *id, "message": "the page could not be created" }),
        );
    }
}

fn update(id: String, config: PageConfig) {
    let mut pages = lock();
    let Some(page) = pages.get_mut(&id) else {
        drop(pages);
        create(id, config);
        return;
    };
    let old = std::mem::replace(&mut page.config, config.clone());
    let Some(browser) = page.browser.clone() else {
        return;
    };
    drop(pages);
    if (old.width != config.width || old.height != config.height)
        && let Some(host) = browser.host()
    {
        host.was_resized();
    }
    if old.fps != config.fps
        && let Some(host) = browser.host()
    {
        host.set_windowless_frame_rate(config.fps.clamp(1, 60) as i32);
    }
    if old.url != config.url {
        if let Some(frame) = browser.main_frame() {
            let url: CefString = config.url.as_str().into();
            frame.load_url(Some(&url));
        }
    } else if old.css != config.css {
        browser.reload();
    }
}

fn close(id: &str) {
    let page = lock().remove(id);
    if let Some(host) = page.and_then(|p| p.browser).and_then(|b| b.host()) {
        host.close_browser(1);
    }
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let mut app = HelperApp::new();
    // CEF re-launches this executable for its renderer, GPU and utility processes.
    let code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if code >= 0 {
        std::process::exit(code);
    }

    let cache = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("eStream")
        .join("browser-cache");
    let settings = Settings {
        windowless_rendering_enabled: 1,
        multi_threaded_message_loop: 1,
        no_sandbox: 1,
        root_cache_path: cache.to_string_lossy().as_ref().into(),
        log_severity: LogSeverity::WARNING,
        background_color: 0,
        ..Default::default()
    };
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        eprintln!("CEF failed to initialize");
        std::process::exit(2);
    }
    emit(serde_json::json!({ "ev": "ready" }));

    // eStream drives everything over stdin; EOF means eStream is gone.
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        match serde_json::from_str::<Command>(&line) {
            Ok(Command::Create { id, page }) => create(id, page),
            Ok(Command::Update { id, page }) => update(id, page),
            Ok(Command::Reload { id }) => {
                if let Some(b) = lock().get(&id).and_then(|p| p.browser.clone()) {
                    b.reload();
                }
            }
            Ok(Command::Close { id }) => close(&id),
            Err(e) => eprintln!("bad command: {e}"),
        }
    }

    let ids: Vec<String> = lock().keys().cloned().collect();
    for id in ids {
        close(&id);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    shutdown();
}
