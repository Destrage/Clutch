//! Clutch browser helper: renders web pages offscreen with CEF in its own process.
//!
//! Commands arrive as JSON lines on stdin (`create`, `update`, `reload`, `close`); events go
//! out as JSON lines on stdout. Frames are written into named shared memory (seqlocked,
//! straight alpha) and page audio into a shared ring, both read by Clutch. Closing stdin
//! (Clutch exiting) closes every page and ends the process.

// No console windows for the helper or the CEF subprocesses it relaunches (its pipes to
// Clutch are redirected handles, which work for GUI-subsystem processes too).
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
    /// Pointer input for interactive pages (chat panels). `kind`: move, down, up, wheel, leave.
    Mouse {
        id: String,
        kind: String,
        x: i32,
        y: i32,
        /// 0 left, 1 middle, 2 right.
        #[serde(default)]
        button: u8,
        #[serde(default)]
        clicks: i32,
        #[serde(default)]
        dx: i32,
        #[serde(default)]
        dy: i32,
        #[serde(default)]
        mods: u32,
    },
    /// Keyboard input. `kind`: down, up or char; `code` is the Windows virtual-key code.
    Key {
        id: String,
        kind: String,
        #[serde(default)]
        code: i32,
        #[serde(default)]
        ch: u32,
        #[serde(default)]
        mods: u32,
    },
    Focus {
        id: String,
        focused: bool,
    },
    /// Back to the page's own address (after a sign-in page, for example).
    Home {
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
        "Local\\clutch-browser-{}-{id}-{kind}-{n}",
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
                // CEF paints premultiplied BGRA; Clutch sources are straight-alpha RGBA.
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
        // Sign-in and other pop-ups open in the same view (there are no windows to show them).
        #[allow(clippy::too_many_arguments)]
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let (Some(browser), Some(url)) = (browser, target_url)
                && let Some(frame) = browser.main_frame()
            {
                frame.load_url(Some(url));
            }
            1
        }

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
            // Don't fight Clutch for the system's media keys.
            let name: CefString = "disable-features".into();
            let value: CefString = "HardwareMediaKeyHandling,MediaSessionService".into();
            cmd.append_switch_with_value(Some(&name), Some(&value));
            // Pages render offscreen only: on Linux don't connect to the display at all, so
            // the helper also runs without one (and never talks to X11 or Wayland).
            if cfg!(target_os = "linux") {
                let name: CefString = "ozone-platform".into();
                let value: CefString = "headless".into();
                cmd.append_switch_with_value(Some(&name), Some(&value));
            }
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

fn browser_of(id: &str) -> Option<Browser> {
    lock().get(id).and_then(|p| p.browser.clone())
}

/// Pointer input for an interactive page.
#[allow(clippy::too_many_arguments)]
fn mouse(
    id: &str,
    kind: &str,
    x: i32,
    y: i32,
    button: u8,
    clicks: i32,
    dx: i32,
    dy: i32,
    mods: u32,
) {
    let Some(host) = browser_of(id).and_then(|b| b.host()) else {
        return;
    };
    let event = MouseEvent {
        x,
        y,
        modifiers: mods,
    };
    let button = match button {
        1 => MouseButtonType::MIDDLE,
        2 => MouseButtonType::RIGHT,
        _ => MouseButtonType::LEFT,
    };
    match kind {
        "move" => host.send_mouse_move_event(Some(&event), 0),
        "leave" => host.send_mouse_move_event(Some(&event), 1),
        "down" | "up" => {
            if kind == "down" {
                host.set_focus(1);
            }
            host.send_mouse_click_event(Some(&event), button, (kind == "up") as i32, clicks.max(1));
        }
        "wheel" => host.send_mouse_wheel_event(Some(&event), dx, dy),
        _ => {}
    }
}

/// Keyboard input for an interactive page.
fn key(id: &str, kind: &str, code: i32, ch: u32, mods: u32) {
    let Some(host) = browser_of(id).and_then(|b| b.host()) else {
        return;
    };
    let type_ = match kind {
        "down" => KeyEventType::RAWKEYDOWN,
        "up" => KeyEventType::KEYUP,
        _ => KeyEventType::CHAR,
    };
    let event = KeyEvent {
        type_,
        modifiers: mods,
        windows_key_code: if kind == "char" { ch as i32 } else { code },
        character: ch as u16,
        unmodified_character: ch as u16,
        ..Default::default()
    };
    host.send_key_event(Some(&event));
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

    let local = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cache = local.join("Clutch").join("browser-cache");
    // Clutch was called eStream up to 0.2: keep its cache (page logins) on the first start.
    let legacy = local.join("eStream").join("browser-cache");
    if !cache.exists() && legacy.is_dir() {
        let _ = std::fs::create_dir_all(local.join("Clutch"));
        let _ = std::fs::rename(&legacy, &cache);
        let _ = std::fs::remove_dir(local.join("eStream"));
    }
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

    // Clutch drives everything over stdin; EOF means Clutch is gone.
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
            Ok(Command::Mouse {
                id,
                kind,
                x,
                y,
                button,
                clicks,
                dx,
                dy,
                mods,
            }) => mouse(&id, &kind, x, y, button, clicks, dx, dy, mods),
            Ok(Command::Key {
                id,
                kind,
                code,
                ch,
                mods,
            }) => key(&id, &kind, code, ch, mods),
            Ok(Command::Focus { id, focused }) => {
                if let Some(host) = browser_of(&id).and_then(|b| b.host()) {
                    host.set_focus(focused as i32);
                }
            }
            Ok(Command::Home { id }) => {
                let url = lock().get(&id).map(|p| p.config.url.clone());
                if let (Some(url), Some(frame)) =
                    (url, browser_of(&id).and_then(|b| b.main_frame()))
                {
                    let url: CefString = url.as_str().into();
                    frame.load_url(Some(&url));
                }
            }
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
