//! The studio controller: owns the engine, turns UI callbacks into engine
//! commands, and turns engine snapshots/events back into UI models.

use crate::actions::{self, Action, Key};
use crate::interact::{self, Drag, DragKind, Handle, ItemGeom, Modifiers};
use crate::presets::{self, PRESETS};
use crate::settings::{PanelLayout, Paths, Settings, ThemeMode, WindowState};
use crate::ui::{
    AppWindow, AudioInspect, CanvasBox, CanvasTab, DestinationRow, FilterParamRow, FilterRow,
    Guide, Inspect, LayerRow, LiveRow, MediaInspect, MixerStrip, PaletteEntry, Preset, PropRow,
    SceneRow, SettingsForm, SourceType, Theme, VideoFilterRow,
};
use estream_core::{BlendMode, BoundsMode, CanvasId, ItemId, Rgba8, SceneId, SourceId};
use estream_engine::{
    Command, Engine, EngineConfig, Event, ItemPatch, PropKind, SourceInfo, SourceStatus,
    Subscription,
};
use estream_gpu::GpuContext;
use estream_project::{
    AudioConfig, AudioFilterDef, ItemContent, Monitoring, Project, Scene, SceneItem, SourceDef,
    persist,
};
use glam::Vec2;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type Shared = Rc<RefCell<App>>;

const BLEND_MODES: [BlendMode; 4] = BlendMode::ALL;
const SIZE_MODES: [BoundsMode; 4] = [
    BoundsMode::None,
    BoundsMode::Stretch,
    BoundsMode::Fit,
    BoundsMode::Fill,
];

pub struct App {
    window: slint::Weak<AppWindow>,
    paths: Paths,
    pub settings: Settings,
    gpu: GpuContext,
    engine: Option<Engine>,
    subscription: Option<Subscription>,
    collection: PathBuf,

    scenes_model: Rc<VecModel<SceneRow>>,
    layers_model: Rc<VecModel<LayerRow>>,
    guides_model: Rc<VecModel<Guide>>,
    palette_model: Rc<VecModel<PaletteEntry>>,
    props_model: Rc<VecModel<PropRow>>,
    /// Cache key and schema behind the current property rows.
    props: Option<((SourceId, String, u64), estream_engine::PropertySchema)>,
    props_generation: u64,

    geoms: Vec<ItemGeom>,
    selection: Option<ItemId>,
    /// A source picked in the mixer that has no item in the active scene.
    selected_source: Option<SourceId>,
    mixer_model: Rc<VecModel<MixerStrip>>,
    meter_holds: HashMap<SourceId, [(f32, Instant); 2]>,
    mixer_at: Instant,
    hover: Option<ItemId>,
    drag: Option<Drag>,
    shut_down: bool,
    clipboard: Option<SceneItem>,
    dirty: bool,

    last_sequence: u64,
    last_infos: Option<Arc<HashMap<SourceId, SourceInfo>>>,
    stats_at: Instant,
    stats_rendered: u64,
    toast_until: Option<Instant>,
    theme_checked_at: Instant,
    recording: Option<estream_media::Recording>,
    replay: Option<estream_media::ReplayBuffer>,
    stream: Option<estream_media::MultiStream>,
    /// Per live destination (same order as `MultiStream::destinations`): its settings index,
    /// last bitrate sample (time, bytes, kbps) and whether its error was already shown.
    stream_dests: Vec<LiveDest>,
    /// Destination being edited in Settings.
    dest_selected: Option<usize>,
    /// Studio mode: edit the Preview scene off-air, program shown beside it.
    studio: bool,
    preview_scene: Option<SceneId>,
    last_preview_sequence: u64,
    /// What the shader editor is editing.
    shader_target: ShaderTarget,
    /// Canvas shown and edited in the studio (`None` = the first).
    current_canvas: Option<CanvasId>,
    /// Last file written by a recording or replay save, for "Show in folder".
    last_output: Option<std::path::PathBuf>,
}

/// What the shader editor dialog applies to.
#[derive(Clone, Debug, PartialEq)]
enum ShaderTarget {
    Transition,
    /// A custom video filter: (source, filter id).
    Filter(SourceId, String),
}

const TRANSITION_SHADER_HELP: &str = "Write fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> in WGSL. Available: from_color(uv), to_color(uv), resolution(), ease(t), over(a, b). Colors are linear and premultiplied.";
const FILTER_SHADER_HELP: &str = "Write fn effect(uv: vec2<f32>) -> vec4<f32> in WGSL. Available: source(uv), size(), time(), param(0u..3u) (the four sliders), to_srgb(c), to_linear(c), luma(c). Colors are linear with straight alpha.";

/// A live destination as the app tracks it.
struct LiveDest {
    /// Index into `settings.destinations`.
    config: usize,
    /// Last bitrate sample: (time, bytes sent, kbps).
    rate: (Instant, u64, f64),
    error_shown: bool,
}

impl LiveDest {
    fn new(config: usize) -> Self {
        Self {
            config,
            rate: (Instant::now(), 0, 0.0),
            error_shown: false,
        }
    }
}

/// Short transport label for a destination, e.g. "RTMPS" or "SRT".
fn protocol_label(s: &estream_media::StreamSettings, long: bool) -> String {
    use estream_media::{Protocol, VideoCodec};
    match s.protocol() {
        Some(Protocol::Rtmp) => {
            let secure = s.server.trim().to_ascii_lowercase().starts_with("rtmps");
            let base = match (secure, long) {
                (true, true) => "RTMPS (encrypted)",
                (true, false) => "RTMPS",
                (false, _) => "RTMP",
            };
            if long && s.codec != VideoCodec::H264 {
                format!("{base}, Enhanced RTMP")
            } else {
                base.to_owned()
            }
        }
        Some(p) => p.label().to_owned(),
        None => String::new(),
    }
}

/// The Stream section of the settings form: the destination list and the selected
/// destination's editor fields (other fields left default).
fn stream_form(
    dests: &[crate::settings::DestinationConfig],
    selected: Option<usize>,
) -> SettingsForm {
    use estream_media::{Service, VideoCodec, encode};
    let rows: Vec<DestinationRow> = dests
        .iter()
        .map(|d| DestinationRow {
            name: d.name.clone().into(),
            enabled: d.enabled,
            protocol: protocol_label(&d.stream, false).into(),
        })
        .collect();
    let selected = selected.filter(|i| *i < dests.len());
    let Some(d) = selected.map(|i| &dests[i]) else {
        return SettingsForm {
            destinations: ModelRc::new(VecModel::from(rows)),
            dest_selected: -1,
            ..Default::default()
        };
    };
    let s = &d.stream;
    let encoders = encode::available(s.codec);
    let mut encoder_names = vec![SharedString::from("Automatic (best available)")];
    encoder_names.extend(encoders.iter().map(|e| SharedString::from(e.label)));
    let protocol = protocol_label(s, true);
    let warning = match s.validate() {
        // Nothing entered yet: no need to nag.
        Err(_) if s.server.trim().is_empty() && s.key.is_empty() => String::new(),
        Err(problem) => problem,
        Ok(_) => match s.service.max_video_kbps() {
            Some(max) if s.codec == VideoCodec::H264 && s.bitrate_kbps > max => format!(
                "{} accepts at most {max} kbps for H.264; higher bitrates may be rejected.",
                s.service.label()
            ),
            _ => String::new(),
        },
    };
    SettingsForm {
        destinations: ModelRc::new(VecModel::from(rows)),
        dest_selected: selected.map_or(-1, |i| i as i32),
        dest_name: d.name.clone().into(),
        stream_services: ModelRc::new(VecModel::from(
            Service::ALL
                .iter()
                .map(|s| SharedString::from(s.label()))
                .collect::<Vec<_>>(),
        )),
        stream_service: Service::ALL
            .iter()
            .position(|v| *v == s.service)
            .unwrap_or(0) as i32,
        stream_server: s.server.clone().into(),
        stream_key_set: !s.key.is_empty(),
        stream_protocol: protocol.into(),
        stream_warning: warning.into(),
        stream_codec: VideoCodec::ALL
            .iter()
            .position(|c| *c == s.codec)
            .unwrap_or(0) as i32,
        stream_encoders: ModelRc::new(VecModel::from(encoder_names)),
        stream_encoder: encoders
            .iter()
            .position(|e| e.name == s.encoder)
            .map_or(0, |i| i + 1) as i32,
        stream_bitrate: s.bitrate_kbps as f32,
        stream_audio_bitrate: s.audio_bitrate_kbps as f32,
        stream_track: s.audio_track as i32,
        stream_reconnect: s.reconnect,
        ..Default::default()
    }
}

fn id_str(id: impl std::fmt::Display) -> SharedString {
    id.to_string().into()
}

fn parse_id(s: &str) -> Option<uuid::Uuid> {
    s.parse().ok()
}

fn to_color(c: Rgba8) -> slint::Color {
    slint::Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

fn from_color(c: slint::Color) -> Rgba8 {
    Rgba8::new(c.red(), c.green(), c.blue(), c.alpha())
}

fn parse_hex_color(s: &str) -> Option<Rgba8> {
    let hex = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(Rgba8::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)),
        8 => Some(Rgba8::new(
            (v >> 24) as u8,
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        )),
        _ => None,
    }
}

fn hex(c: Rgba8) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

/// Updates a model in place, touching only rows that changed.
fn sync_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

fn unique_name<'a>(base: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing: Vec<&str> = existing.collect();
    if !existing.contains(&base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !existing.contains(&name.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

/// Deep copy of an item with fresh IDs (sources are shared, not copied).
fn fresh_ids(item: &SceneItem) -> SceneItem {
    let mut copy = item.clone();
    copy.id = ItemId::new();
    if let ItemContent::Group { items } = &mut copy.content {
        *items = items.iter().map(fresh_ids).collect();
    }
    copy
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "color" => "Color source",
        "test_pattern" => "Test pattern",
        "group" => "Group",
        "display_capture" => "Display capture",
        "camera" => "Video capture device",
        other => other,
    }
}

fn status_problem(info: Option<&SourceInfo>) -> String {
    match info.map(|i| &i.status) {
        Some(SourceStatus::Unavailable(r)) | Some(SourceStatus::Error(r)) => r.clone(),
        _ => String::new(),
    }
}

fn key_from_text(text: &str) -> Option<Key> {
    use slint::platform::Key as K;
    let is = |k: K| text == SharedString::from(k).as_str();
    Some(if is(K::Delete) || is(K::Backspace) {
        Key::Delete
    } else if is(K::Escape) {
        Key::Escape
    } else if is(K::UpArrow) {
        Key::Up
    } else if is(K::DownArrow) {
        Key::Down
    } else if is(K::LeftArrow) {
        Key::Left
    } else if is(K::RightArrow) {
        Key::Right
    } else if is(K::F2) {
        Key::F2
    } else {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        // Ctrl+letter may arrive as an ASCII control character.
        let c = if (c as u32) >= 1 && (c as u32) <= 26 {
            char::from(b'a' + (c as u8) - 1)
        } else {
            c
        };
        Key::Char(c.to_ascii_lowercase())
    })
}

impl App {
    pub fn new(window: &AppWindow, paths: Paths, settings: Settings, gpu: GpuContext) -> Shared {
        let app = App {
            window: window.as_weak(),
            collection: paths.default_collection(),
            paths,
            settings,
            gpu,
            engine: None,
            subscription: None,
            scenes_model: Rc::new(VecModel::default()),
            layers_model: Rc::new(VecModel::default()),
            guides_model: Rc::new(VecModel::default()),
            palette_model: Rc::new(VecModel::default()),
            props_model: Rc::new(VecModel::default()),
            props: None,
            props_generation: 0,
            geoms: Vec::new(),
            selection: None,
            selected_source: None,
            mixer_model: Rc::new(VecModel::default()),
            meter_holds: HashMap::new(),
            mixer_at: Instant::now(),
            hover: None,
            drag: None,
            shut_down: false,
            clipboard: None,
            dirty: false,
            last_sequence: 0,
            last_infos: None,
            stats_at: Instant::now(),
            stats_rendered: 0,
            toast_until: None,
            theme_checked_at: Instant::now(),
            recording: None,
            replay: None,
            stream: None,
            stream_dests: Vec::new(),
            dest_selected: None,
            studio: false,
            preview_scene: None,
            last_preview_sequence: 0,
            shader_target: ShaderTarget::Transition,
            current_canvas: None,
            last_output: None,
        };
        window.set_scenes(ModelRc::from(app.scenes_model.clone()));
        window.set_layers(ModelRc::from(app.layers_model.clone()));
        window.set_guides(ModelRc::from(app.guides_model.clone()));
        window.set_palette_entries(ModelRc::from(app.palette_model.clone()));
        window.set_props(ModelRc::from(app.props_model.clone()));
        window.set_mixer_strips(ModelRc::from(app.mixer_model.clone()));
        Rc::new(RefCell::new(app))
    }

    fn window(&self) -> Option<AppWindow> {
        self.window.upgrade()
    }

    // ------------------------------------------------------------ startup

    /// Decides the startup flow: first-run wizard, crash recovery, or normal load.
    pub fn begin(&mut self) {
        let Some(w) = self.window() else { return };
        self.apply_theme();
        self.apply_layout(self.settings.layout);
        let info = &self.gpu.info;
        w.set_gpu_info(format!("{} · {:?}", info.name, info.backend).into());

        self.collection = self
            .settings
            .last_collection
            .clone()
            .filter(|p| p.exists())
            .unwrap_or_else(|| self.paths.default_collection());
        w.set_collection_name(self.collection_name().into());

        if !self.collection.exists() {
            w.set_presets(ModelRc::new(VecModel::from(
                PRESETS
                    .iter()
                    .map(|p| Preset {
                        label: p.label.into(),
                        detail: p.detail.into(),
                    })
                    .collect::<Vec<_>>(),
            )));
            w.set_wizard_gpu(info.name.clone().into());
            w.set_wizard_encoders(presets::likely_encoders(info.vendor).into());
            w.set_startup("wizard".into());
        } else if persist::find_recovery(&self.collection).is_some() {
            w.set_startup("recovery".into());
        } else {
            match persist::load(&self.collection) {
                Ok(loaded) => self.start(loaded.project),
                Err(e) => {
                    tracing::error!(error = %e, "failed to load scene collection");
                    let broken = self.collection.with_extension("broken.json");
                    let _ = std::fs::rename(&self.collection, &broken);
                    self.toast(
                        format!(
                            "Couldn't open the scene collection ({e}). It was kept as {}.",
                            broken.display()
                        ),
                        true,
                    );
                    self.begin();
                }
            }
        }
    }

    fn collection_name(&self) -> String {
        self.collection
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn wizard_create(&mut self, index: i32) {
        let preset = PRESETS
            .get(index.max(0) as usize)
            .copied()
            .unwrap_or(PRESETS[0]);
        let mut project = Project::new_default(preset.width, preset.height, preset.fps);
        project.scenes[0].name = "Main".into();
        // Like OBS: desktop audio and the default microphone in every scene.
        for (name, kind) in [
            ("Desktop Audio", "audio_output"),
            ("Microphone", "audio_input"),
        ] {
            let mut source = SourceDef::new(name, kind, serde_json::json!({ "device": "" }));
            source.audio.global = true;
            project.sources.push(source);
        }
        if let Err(e) = persist::save(&self.collection, &project) {
            self.toast(format!("Couldn't create the scene collection: {e}"), true);
            return;
        }
        self.start(project);
        self.toast(format!("Canvas set to {}", preset.label), false);
    }

    pub fn recovery(&mut self, restore: bool) {
        let loaded = if restore {
            persist::load(&persist::recovery_path(&self.collection)).and_then(|l| {
                persist::save(&self.collection, &l.project)?;
                Ok(l)
            })
        } else {
            persist::load(&self.collection)
        };
        let _ = persist::discard_recovery(&self.collection);
        match loaded {
            Ok(l) => {
                self.start(l.project);
                if restore {
                    self.toast("Restored your unsaved changes".into(), false);
                }
            }
            Err(e) => {
                self.toast(format!("Recovery failed: {e}"), true);
                if let Ok(l) = persist::load(&self.collection) {
                    self.start(l.project);
                }
            }
        }
    }

    fn start(&mut self, project: Project) {
        let mut registry = estream_engine::SourceRegistry::with_builtins();
        estream_capture::register_all(&mut registry);
        estream_media::media_source::register(&mut registry);
        if let Some(w) = self.window() {
            w.set_source_types(ModelRc::new(VecModel::from(source_types(&registry))));
        }
        let config = EngineConfig {
            registry,
            gpu: Some(self.gpu.clone()),
            collection_path: Some(self.collection.clone()),
            ..Default::default()
        };
        match Engine::start(project, config) {
            Ok(engine) => {
                self.subscription = Some(engine.subscribe());
                self.engine = Some(engine);
                self.settings.last_collection = Some(self.collection.clone());
                if let Some(w) = self.window() {
                    w.set_startup("ready".into());
                    w.set_collection_name(self.collection_name().into());
                    w.set_window_title(format!("eStream — {}", self.collection_name()).into());
                    w.invoke_zoom_fit();
                }
                self.refresh();
            }
            Err(e) => self.toast(format!("Engine failed to start: {e}"), true),
        }
    }

    /// Saves settings and stops the engine (which saves the collection).
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        // Finish files cleanly before the engine goes away.
        if let Some(engine) = &self.engine {
            if let Some(rec) = self.recording.take() {
                let _ = rec.stop(engine);
            }
            if let Some(replay) = self.replay.take() {
                let _ = replay.stop(engine);
            }
            if let Some(stream) = self.stream.take() {
                let _ = stream.stop(engine);
            }
        }
        if let Some(w) = self.window() {
            let win = w.window();
            let size = win.size();
            let pos = win.position();
            self.settings.window = Some(WindowState {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
                maximized: win.is_maximized(),
            });
            self.settings.layout = PanelLayout {
                scenes_width: w.get_scenes_width(),
                inspector_width: w.get_inspector_width(),
                bottom_height: w.get_bottom_height(),
                sources_width: w.get_sources_width(),
            };
        }
        self.settings.save(&self.paths.settings());
        self.subscription = None;
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }

    // ------------------------------------------------------------ periodic

    /// Called from a fast UI timer: preview frames, engine events, stats, toasts.
    pub fn tick(&mut self) {
        let Some(w) = self.window() else { return };
        let mut changed = false;
        if let Some(engine) = &self.engine {
            let image = |frame: estream_engine::CanvasFrame<'_>| {
                slint::Image::try_from(frame.preview.clone())
                    .map_err(|e| tracing::error!(error = %e, "preview texture rejected"))
                    .ok()
            };
            let snapshot = engine.snapshot();
            if let Some(canvas) = self.canvas_of(&snapshot).map(|c| c.id) {
                // Program: the main view normally, the Program view in studio mode.
                if let Some(output) = engine.canvas_output(canvas)
                    && let Some(frame) = output.latest()
                    && frame.sequence != self.last_sequence
                {
                    self.last_sequence = frame.sequence;
                    if let Some(img) = image(frame) {
                        if self.studio {
                            w.set_program_frame(img);
                        } else {
                            w.set_preview_frame(img);
                        }
                    }
                }
                // Studio Preview: the editable main view.
                if self.studio
                    && let Some(output) = engine.preview_output(canvas)
                    && let Some(frame) = output.latest()
                    && frame.sequence != self.last_preview_sequence
                {
                    self.last_preview_sequence = frame.sequence;
                    if let Some(img) = image(frame) {
                        w.set_preview_frame(img);
                    }
                }
                w.set_transition_busy(engine.is_transitioning(canvas));
            }
            w.set_media_inspect(self.media_inspect());
            let (browser, status) = self.browser_inspect();
            w.set_browser_selected(browser);
            w.set_browser_status(status.into());
            let infos = engine.source_infos();
            if self
                .last_infos
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &infos))
            {
                self.last_infos = Some(infos);
                changed = true;
            }
        }
        let mut events = Vec::new();
        if let Some(sub) = &self.subscription {
            if sub.take_lagged() {
                changed = true;
            }
            while let Some(event) = sub.try_recv() {
                events.push(event);
            }
        }
        {
            for event in events {
                match event {
                    Event::Changed { .. } => {
                        self.dirty = true;
                        changed = true;
                    }
                    Event::Saved { .. } => {
                        self.dirty = false;
                        changed = true;
                    }
                    Event::CommandFailed { error } => self.toast(error, true),
                    Event::RecoverySaved { .. } => {
                        w.set_status_message("Autosaved".into());
                    }
                }
            }
        }
        if changed {
            self.refresh();
        }
        if !changed && self.mixer_at.elapsed() >= Duration::from_millis(33) {
            self.mixer_at = Instant::now();
            self.update_mixer();
        }
        if self.stats_at.elapsed() >= Duration::from_millis(500) {
            self.update_stats();
            self.update_outputs();
        }
        if self.toast_until.is_some_and(|t| Instant::now() >= t) {
            self.toast_until = None;
            w.set_toast_text("".into());
        }
        if self.settings.theme == ThemeMode::System
            && self.theme_checked_at.elapsed() >= Duration::from_secs(3)
        {
            self.theme_checked_at = Instant::now();
            self.apply_theme();
        }
    }

    fn update_stats(&mut self) {
        let Some(w) = self.window() else { return };
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        let Some(canvas) = self.canvas_of(&snapshot) else {
            return;
        };
        let Some(output) = engine.canvas_output(canvas.id) else {
            return;
        };
        let rendered = output
            .stats
            .rendered
            .load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = self.stats_at.elapsed().as_secs_f32();
        if self.stats_rendered > 0 && elapsed > 0.0 {
            w.set_fps(rendered.saturating_sub(self.stats_rendered) as f32 / elapsed);
        }
        self.stats_rendered = rendered;
        self.stats_at = Instant::now();
        w.set_target_fps(canvas.fps.as_f64() as f32);
        w.set_render_ms(
            output
                .stats
                .last_render_ns
                .load(std::sync::atomic::Ordering::Relaxed) as f32
                / 1e6,
        );
        w.set_lagged(
            output
                .stats
                .lagged
                .load(std::sync::atomic::Ordering::Relaxed) as i32,
        );
    }

    fn toast(&mut self, text: String, error: bool) {
        if error {
            tracing::warn!(%text, "toast");
        }
        if let Some(w) = self.window() {
            w.set_toast_text(text.into());
            w.set_toast_error(error);
        }
        self.toast_until =
            Some(Instant::now() + Duration::from_millis(if error { 4500 } else { 2200 }));
    }

    // ------------------------------------------------------------ appearance

    pub fn apply_theme(&self) {
        let Some(w) = self.window() else { return };
        let dark = match self.settings.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => crate::settings::system_prefers_dark(),
        };
        let theme = w.global::<Theme>();
        theme.set_dark(dark);
        if let Some((r, g, b)) = crate::settings::parse_hex(&self.settings.accent) {
            theme.set_accent(slint::Color::from_rgb_u8(r, g, b));
        }
    }

    fn set_theme(&mut self, mode: ThemeMode) {
        self.settings.theme = mode;
        self.apply_theme();
        self.settings.save(&self.paths.settings());
    }

    pub fn toggle_theme(&mut self) {
        let dark = self
            .window()
            .is_some_and(|w| w.global::<Theme>().get_dark());
        self.set_theme(if dark {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        });
    }

    fn apply_layout(&self, layout: PanelLayout) {
        let Some(w) = self.window() else { return };
        w.set_scenes_width(layout.scenes_width);
        w.set_inspector_width(layout.inspector_width);
        w.set_bottom_height(layout.bottom_height);
        w.set_sources_width(layout.sources_width);
    }

    // ------------------------------------------------------------ model → UI

    /// The canvas being edited: the selected one, or the first if it no longer exists.
    fn canvas_of<'a>(&self, snapshot: &'a Project) -> Option<&'a estream_project::Canvas> {
        self.current_canvas
            .and_then(|id| snapshot.canvas(id))
            .or_else(|| snapshot.canvases.first())
    }

    /// The scene being edited: the Preview scene in studio mode, else the program scene.
    fn edit_scene(&self, canvas: &estream_project::Canvas, snapshot: &Project) -> Option<SceneId> {
        match self.preview_scene {
            Some(id) if self.studio && snapshot.scene(id).is_some() => Some(id),
            _ => canvas.active_scene,
        }
    }

    fn canvas_and_scene(&self) -> Option<(Arc<Project>, CanvasId, Option<SceneId>)> {
        let snapshot = self.engine.as_ref()?.snapshot();
        let canvas = self.canvas_of(&snapshot)?;
        let (id, edit) = (canvas.id, self.edit_scene(canvas, &snapshot));
        Some((snapshot, id, edit))
    }

    /// Rebuilds every UI model from the latest engine snapshot.
    pub fn refresh(&mut self) {
        let Some(w) = self.window() else { return };
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        let Some(canvas) = self.canvas_of(&snapshot) else {
            return;
        };
        let editing = self.edit_scene(canvas, &snapshot);
        let scene = editing.and_then(|id| snapshot.scene(id));
        let infos = engine.source_infos();
        self.refresh_transition(&w, &snapshot);
        self.refresh_canvases(&w, &snapshot);

        if let Some(sel) = self.selection
            && scene.is_none_or(|s| s.find_item(sel).is_none())
        {
            self.selection = None;
            self.drag = None;
        }
        let size_of = |id: SourceId| infos.get(&id).and_then(|i| i.size);
        self.geoms = scene
            .map(|s| interact::scene_geometry(s, &size_of))
            .unwrap_or_default();

        sync_model(
            &self.scenes_model,
            snapshot
                .scenes_of(canvas.id)
                .map(|s| {
                    let mut items = 0;
                    s.walk(&mut |_| items += 1);
                    SceneRow {
                        id: id_str(s.id),
                        name: s.name.clone().into(),
                        active: Some(s.id) == editing,
                        live: Some(s.id) == canvas.active_scene,
                        items,
                    }
                })
                .collect(),
        );

        let mut layers = Vec::new();
        if let Some(scene) = scene {
            push_layers(
                &mut layers,
                &scene.items,
                0,
                &snapshot,
                &infos,
                self.selection,
            );
        }
        sync_model(&self.layers_model, layers);

        w.set_canvas_w(canvas.width as f32);
        w.set_canvas_h(canvas.height as f32);
        let fps = canvas.fps.as_f64();
        let fps = if fps.fract() == 0.0 {
            format!("{fps}")
        } else {
            format!("{fps:.2}")
        };
        w.set_canvas_info(format!("{} × {} · {fps} fps", canvas.width, canvas.height).into());
        w.set_can_undo(engine.can_undo());
        w.set_can_redo(engine.can_redo());
        w.set_dirty(self.dirty);

        if self
            .selected_source
            .is_some_and(|id| snapshot.source(id).is_none())
        {
            self.selected_source = None;
        }
        let (selection_box, mut inspect) = match (self.selection, scene) {
            (Some(sel), Some(scene)) => self.describe_selection(sel, scene, &snapshot, &infos),
            _ => match self.selected_source.and_then(|id| snapshot.source(id)) {
                // A global audio source picked in the mixer (not placed in this scene).
                Some(source) => (
                    CanvasBox::default(),
                    Inspect {
                        has: true,
                        id: id_str(source.id),
                        name: source.name.clone().into(),
                        kind: source.kind.clone().into(),
                        kind_label: engine
                            .registry()
                            .get(&source.kind)
                            .map_or(source.kind.clone(), |t| t.name.clone())
                            .into(),
                        problem: status_problem(infos.get(&source.id)).into(),
                        ..Default::default()
                    },
                ),
                None => (CanvasBox::default(), Inspect::default()),
            },
        };
        if inspect.has {
            inspect.no_video = !engine
                .registry()
                .get(inspect.kind.as_str())
                .is_some_and(|t| t.has_video())
                && inspect.kind != "group";
        }
        w.set_selection_box(selection_box);
        w.set_inspect(inspect);
        w.set_audio_inspect(self.audio_inspect(&snapshot));
        self.refresh_props(&snapshot);
        self.refresh_video_filters(&w, &snapshot);
        self.update_mixer();
        w.set_hover_box(self.hover_box());
    }

    fn hover_box(&self) -> CanvasBox {
        self.hover
            .and_then(|id| self.geoms.iter().find(|g| g.id == id))
            .map(|g| {
                let (c, s, r) = g.display_box();
                CanvasBox {
                    visible: true,
                    cx: c.x,
                    cy: c.y,
                    w: s.x,
                    h: s.y,
                    rotation: r,
                    handles: false,
                    locked: g.locked,
                }
            })
            .unwrap_or_default()
    }

    fn describe_selection(
        &self,
        sel: ItemId,
        scene: &Scene,
        snapshot: &Project,
        infos: &HashMap<SourceId, SourceInfo>,
    ) -> (CanvasBox, Inspect) {
        let (Some(item), Some(geom)) = (
            scene.find_item(sel),
            self.geoms.iter().find(|g| g.id == sel),
        ) else {
            return Default::default();
        };
        let (center, size, rotation) = geom.display_box();
        let selection_box = CanvasBox {
            visible: true,
            cx: center.x,
            cy: center.y,
            w: size.x,
            h: size.y,
            rotation,
            handles: geom.editable_box(),
            locked: item.locked,
        };
        let source = item.source_id().and_then(|id| snapshot.source(id));
        let kind = source.map_or("group", |s| s.kind.as_str());
        let name = item
            .name
            .clone()
            .or_else(|| source.map(|s| s.name.clone()))
            .unwrap_or_else(|| "Group".into());
        let kind_label = self
            .engine
            .as_ref()
            .and_then(|e| e.registry().get(kind).map(|t| t.name.clone()))
            .unwrap_or_else(|| kind_label(kind).to_owned());
        let t = &item.transform;
        let inspect = Inspect {
            has: true,
            id: id_str(item.id),
            name: name.into(),
            kind: kind.into(),
            kind_label: kind_label.into(),
            group: item.is_group(),
            problem: status_problem(item.source_id().and_then(|id| infos.get(&id))).into(),
            x: t.position.x,
            y: t.position.y,
            w: geom.box_size.x,
            h: geom.box_size.y,
            rotation: t.rotation,
            size_mode: SIZE_MODES
                .iter()
                .position(|m| *m == t.bounds_mode)
                .unwrap_or(0) as i32,
            crop_l: item.crop.left as f32,
            crop_t: item.crop.top as f32,
            crop_r: item.crop.right as f32,
            crop_b: item.crop.bottom as f32,
            opacity: item.opacity,
            blend: BLEND_MODES
                .iter()
                .position(|m| *m == item.blend)
                .unwrap_or(0) as i32,
            visible: item.visible,
            locked: item.locked,
            no_video: false,
        };
        (selection_box, inspect)
    }

    // ------------------------------------------------------------ commands

    fn exec(&mut self, command: Command) -> bool {
        let Some(engine) = &self.engine else {
            return false;
        };
        match engine.execute(command) {
            Ok(_) => {
                self.dirty = true;
                self.refresh();
                true
            }
            Err(e) => {
                self.toast(e.to_string(), true);
                false
            }
        }
    }

    fn selected(&self) -> Option<(SceneId, SceneItem)> {
        let (snapshot, _, scene) = self.canvas_and_scene()?;
        let scene = snapshot.scene(scene?)?;
        let item = scene.find_item(self.selection?)?.clone();
        Some((scene.id, item))
    }

    fn update_selected(&mut self, patch: ItemPatch) {
        if let Some((scene, item)) = self.selected() {
            self.exec(Command::UpdateItem {
                scene,
                item: item.id,
                patch,
            });
        }
    }

    pub fn scene_activate(&mut self, id: &str) {
        let Some((_, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(uuid) = parse_id(id) else { return };
        self.selection = None;
        let scene = SceneId(uuid);
        if self.studio {
            self.show_in_preview(canvas, scene);
        } else if let Some(engine) = &self.engine
            && let Err(e) = engine.transition_to(canvas, scene, None)
        {
            self.toast(e.to_string(), true);
        }
    }

    /// Studio mode: shows `scene` in the Preview (off-air).
    fn show_in_preview(&mut self, canvas: CanvasId, scene: SceneId) {
        self.preview_scene = Some(scene);
        if let Some(engine) = &self.engine {
            engine.set_preview_scene(canvas, Some(scene));
        }
        self.refresh();
    }

    /// Adds a scene and shows it: in the Preview in studio mode, else on program.
    fn add_scene_and_show(&mut self, canvas: CanvasId, scene: Scene, index: usize) -> bool {
        let id = scene.id;
        let mut commands = vec![Command::AddScene { scene, index }];
        if !self.studio {
            commands.push(Command::SetActiveScene {
                canvas,
                scene: Some(id),
            });
        }
        if !self.exec(Command::Batch { commands }) {
            return false;
        }
        self.selection = None;
        if self.studio {
            self.show_in_preview(canvas, id);
        }
        true
    }

    pub fn scene_add(&mut self) {
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        let name = unique_name("Scene", snapshot.scenes.iter().map(|s| s.name.as_str()));
        let scene = Scene::new(name, canvas);
        let id = scene.id;
        if self.add_scene_and_show(canvas, scene, usize::MAX)
            && let Some(w) = self.window()
        {
            w.set_renaming_scene(id_str(id));
        }
    }

    pub fn scene_duplicate(&mut self, id: &str) {
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(source) = parse_id(id).and_then(|u| snapshot.scene(SceneId(u))) else {
            return;
        };
        let index = snapshot
            .scenes
            .iter()
            .position(|s| s.id == source.id)
            .map_or(usize::MAX, |i| i + 1);
        let mut copy = Scene::new(
            unique_name(
                &format!("{} copy", source.name),
                snapshot.scenes.iter().map(|s| s.name.as_str()),
            ),
            canvas,
        );
        copy.items = source.items.iter().map(fresh_ids).collect();
        self.add_scene_and_show(canvas, copy, index);
    }

    pub fn scene_remove(&mut self, id: &str) {
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        if snapshot.scenes_of(canvas).count() <= 1 {
            self.toast("A canvas needs at least one scene".into(), true);
            return;
        }
        if let Some(uuid) = parse_id(id) {
            let name = snapshot
                .scene(SceneId(uuid))
                .map(|s| s.name.clone())
                .unwrap_or_default();
            if self.exec(Command::RemoveScene {
                scene: SceneId(uuid),
            }) {
                self.toast(format!("Deleted “{name}” — Ctrl+Z to undo"), false);
            }
        }
    }

    pub fn scene_rename(&mut self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        if let Some(uuid) = parse_id(id)
            && snapshot
                .scene(SceneId(uuid))
                .is_some_and(|s| s.name != name)
        {
            self.exec(Command::RenameScene {
                scene: SceneId(uuid),
                name: name.into(),
            });
        }
    }

    pub fn layer_select(&mut self, id: &str) {
        self.selected_source = None;
        self.selection = parse_id(id).map(ItemId);
        self.refresh();
    }

    pub fn layer_toggle(&mut self, id: &str, lock: bool) {
        let Some((snapshot, _, Some(scene))) = self.canvas_and_scene() else {
            return;
        };
        let Some(item_id) = parse_id(id).map(ItemId) else {
            return;
        };
        let Some(item) = snapshot.scene(scene).and_then(|s| s.find_item(item_id)) else {
            return;
        };
        let patch = if lock {
            ItemPatch {
                locked: Some(!item.locked),
                ..Default::default()
            }
        } else {
            ItemPatch {
                visible: Some(!item.visible),
                ..Default::default()
            }
        };
        self.exec(Command::UpdateItem {
            scene,
            item: item_id,
            patch,
        });
    }

    pub fn add_source(&mut self, kind: &str) {
        let Some((snapshot, _, Some(scene))) = self.canvas_and_scene() else {
            self.toast("Create a scene first".into(), true);
            return;
        };
        let Some(canvas) = self.canvas_of(&snapshot) else {
            return;
        };
        let canvas_size = Vec2::new(canvas.width as f32, canvas.height as f32);
        let Some(source_type) = self
            .engine
            .as_ref()
            .and_then(|e| e.registry().get(kind).cloned())
        else {
            self.toast(format!("“{kind}” sources aren't available yet"), true);
            return;
        };
        // Start from the schema defaults (first monitor, first camera, …).
        let mut settings = source_type
            .schema(&serde_json::json!({}))
            .with_defaults(&serde_json::json!({}));
        let transform = match kind {
            "color" => {
                let (w, h) = (canvas.width / 3, canvas.height / 3);
                let accent =
                    crate::settings::parse_hex(&self.settings.accent).unwrap_or((108, 140, 255));
                settings["width"] = w.into();
                settings["height"] = h.into();
                settings["color"] =
                    serde_json::to_value(Rgba8::new(accent.0, accent.1, accent.2, 255))
                        .unwrap_or_default();
                estream_core::ItemTransform {
                    position: (canvas_size - Vec2::new(w as f32, h as f32)) * 0.5,
                    ..Default::default()
                }
            }
            "test_pattern" => {
                settings["width"] = canvas.width.into();
                settings["height"] = canvas.height.into();
                interact::fit_to_canvas(canvas_size)
            }
            "display_capture" | "window_capture" | "game_capture" | "camera" => {
                interact::fit_to_canvas(canvas_size)
            }
            "text" => estream_core::ItemTransform {
                position: canvas_size * 0.08,
                ..Default::default()
            },
            _ => estream_core::ItemTransform::default(),
        };
        let name = unique_name(
            &source_type.name,
            snapshot.sources.iter().map(|s| s.name.as_str()),
        );
        let source = SourceDef::new(name, kind, settings);
        let mut item = SceneItem::source(source.id);
        item.transform = transform;
        let item_id = item.id;
        if self.exec(Command::Batch {
            commands: vec![
                Command::AddSource {
                    source,
                    index: usize::MAX,
                },
                Command::AddItem {
                    scene,
                    parent: None,
                    index: usize::MAX,
                    item,
                },
            ],
        }) {
            self.selection = Some(item_id);
            self.refresh();
        }
    }

    // ------------------------------------------------------------ outputs

    /// Opens Settings at the stream destinations (e.g. when something is missing).
    fn open_stream_settings(&mut self) {
        if self.dest_selected.is_none() && !self.settings.destinations.is_empty() {
            self.dest_selected = Some(0);
        }
        self.refresh_settings_form();
        if let Some(w) = self.window() {
            w.set_settings_open(true);
        }
    }

    // ------------------------------------------------------------ canvases

    fn refresh_canvases(&self, w: &AppWindow, snapshot: &Project) {
        let current = self.canvas_of(snapshot).map(|c| c.id);
        let tabs: Vec<CanvasTab> = snapshot
            .canvases
            .iter()
            .map(|c| {
                let fps = c.fps.as_f64();
                let fps = if (fps - fps.round()).abs() < 0.01 {
                    format!("{fps:.0}")
                } else {
                    format!("{fps:.2}")
                };
                CanvasTab {
                    id: id_str(c.id),
                    name: c.name.clone().into(),
                    info: format!("{} × {} · {fps} fps", c.width, c.height).into(),
                    active: Some(c.id) == current,
                }
            })
            .collect();
        w.set_canvas_tabs(ModelRc::new(VecModel::from(tabs)));
        let mut presets = vec![SharedString::from("+ Add canvas…")];
        presets.extend(PRESETS.iter().map(|p| SharedString::from(p.label)));
        w.set_canvas_presets(ModelRc::new(VecModel::from(presets)));
    }

    /// Switches the canvas shown and edited in the studio.
    pub fn canvas_select(&mut self, id: &str) {
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        let Some(new) = parse_id(id)
            .map(CanvasId)
            .filter(|c| snapshot.canvas(*c).is_some())
        else {
            return;
        };
        let old = self.canvas_of(&snapshot).map(|c| c.id);
        if old == Some(new) {
            return;
        }
        if self.studio {
            // The Preview follows the canvas being edited.
            if let Some(old) = old {
                engine.set_preview_scene(old, None);
            }
            self.preview_scene = snapshot.canvas(new).and_then(|c| c.active_scene);
            engine.set_preview_scene(new, self.preview_scene);
        }
        self.current_canvas = Some(new);
        self.selection = None;
        self.drag = None;
        self.last_sequence = 0;
        self.last_preview_sequence = 0;
        self.refresh();
    }

    /// Adds a canvas from preset `index` with one empty scene, and selects it.
    pub fn canvas_add(&mut self, index: i32) {
        let Some(preset) = PRESETS.get(index.max(0) as usize) else {
            return;
        };
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        let base = if preset.height > preset.width {
            "Vertical"
        } else if preset.height == preset.width {
            "Square"
        } else {
            "Canvas"
        };
        let name = unique_name(base, snapshot.canvases.iter().map(|c| c.name.as_str()));
        let canvas =
            estream_project::Canvas::new(name.clone(), preset.width, preset.height, preset.fps);
        let id = canvas.id;
        let scene = Scene::new(format!("{name} scene"), id);
        let scene_id = scene.id;
        if self.exec(Command::Batch {
            commands: vec![
                Command::AddCanvas {
                    canvas,
                    index: usize::MAX,
                },
                Command::AddScene {
                    scene,
                    index: usize::MAX,
                },
                Command::SetActiveScene {
                    canvas: id,
                    scene: Some(scene_id),
                },
            ],
        }) {
            self.toast(
                format!("Added {name} ({} × {})", preset.width, preset.height),
                false,
            );
            self.canvas_select(&id_str(id));
        }
    }

    pub fn canvas_remove(&mut self, id: &str) {
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        if snapshot.canvases.len() < 2 {
            self.toast("A collection needs at least one canvas".into(), true);
            return;
        }
        let Some(canvas) = parse_id(id).map(CanvasId).and_then(|c| snapshot.canvas(c)) else {
            return;
        };
        let (cid, name) = (canvas.id, canvas.name.clone());
        if self.studio {
            engine.set_preview_scene(cid, None);
        }
        if self.exec(Command::RemoveCanvas { canvas: cid }) {
            if self.current_canvas == Some(cid) {
                self.current_canvas = None;
                self.selection = None;
                self.last_sequence = 0;
            }
            self.toast(format!("Removed “{name}” — Ctrl+Z to undo"), false);
            self.refresh();
        }
    }

    pub fn canvas_rename(&mut self, id: &str, name: &str) {
        let name = name.trim();
        let Some(canvas) = parse_id(id).map(CanvasId) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        self.exec(Command::UpdateCanvas {
            canvas,
            patch: estream_engine::CanvasPatch {
                name: Some(name.to_owned()),
                ..Default::default()
            },
        });
    }

    // ------------------------------------------------------------ studio mode & transitions

    fn toggle_studio(&mut self) {
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        self.studio = !self.studio;
        self.selection = None;
        self.last_sequence = 0;
        self.last_preview_sequence = 0;
        let program = snapshot.canvas(canvas).and_then(|c| c.active_scene);
        if let Some(engine) = &self.engine {
            if self.studio {
                // Start by preparing a copy of what's on air.
                self.preview_scene = self
                    .preview_scene
                    .filter(|id| snapshot.scene(*id).is_some())
                    .or(program);
                engine.set_preview_scene(canvas, self.preview_scene);
            } else {
                engine.set_preview_scene(canvas, None);
            }
        }
        if let Some(w) = self.window() {
            w.set_studio_mode(self.studio);
        }
        self.refresh();
    }

    /// Studio mode: send the Preview to Program ("go" = chosen transition, "cut", "fade").
    pub fn studio_go(&mut self, mode: &str) {
        if !self.studio {
            self.toast("Turn on studio mode to use Preview / Program".into(), false);
            return;
        }
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(scene) = self.preview_scene else {
            return;
        };
        let transition = match mode {
            "cut" => Some(estream_project::Transition::CUT),
            "fade" => Some(estream_project::Transition {
                kind: estream_project::TransitionKind::Fade,
                duration_ms: snapshot.transition.duration_ms.max(100),
            }),
            _ => None,
        };
        if let Some(engine) = &self.engine
            && let Err(e) = engine.transition_to(canvas, scene, transition)
        {
            self.toast(e.to_string(), true);
        }
    }

    /// Transition picker state for the UI.
    fn refresh_transition(&self, w: &AppWindow, snapshot: &Project) {
        use estream_project::{Direction, TransitionKind as K};
        let t = &snapshot.transition;
        let (kind, direction) = match &t.kind {
            K::Cut => (0, None),
            K::Fade => (1, None),
            K::FadeToColor { .. } => (2, None),
            K::Slide { direction } => (3, Some(*direction)),
            K::Swipe { direction } => (4, Some(*direction)),
            K::Wipe { direction, .. } => (5, Some(*direction)),
            K::Shader { .. } => (6, None),
            K::Stinger { .. } => (7, None),
        };
        w.set_transition_kind(kind);
        w.set_transition_direction(
            direction
                .and_then(|d| Direction::ALL.iter().position(|x| *x == d))
                .unwrap_or(0) as i32,
        );
        w.set_transition_duration(t.duration_ms as f32);
        let (file, point) = match &t.kind {
            K::Stinger { file, point_ms } => (
                std::path::Path::new(file)
                    .file_name()
                    .map_or(file.clone(), |n| n.to_string_lossy().into_owned()),
                *point_ms as f32,
            ),
            _ => (String::new(), 0.0),
        };
        w.set_transition_file(file.into());
        w.set_transition_point(point);
        w.set_transition_label(
            if t.is_instant() {
                "Cut".to_owned()
            } else {
                format!("{} · {} ms", t.kind.label(), t.duration_ms)
            }
            .into(),
        );
        w.set_studio_mode(self.studio);
    }

    /// Changes the program transition (undoable).
    pub fn transition_set(&mut self, field: &str, value: f32) {
        use estream_project::{Direction, Transition, TransitionKind as K};
        let Some(engine) = &self.engine else { return };
        let current = engine.snapshot().transition.clone();
        let direction = match &current.kind {
            K::Slide { direction } | K::Swipe { direction } | K::Wipe { direction, .. } => {
                *direction
            }
            _ => Direction::Left,
        };
        let i = value.round().max(0.0) as usize;
        let transition = match field {
            "kind" => {
                let kind = match i {
                    0 => K::Cut,
                    1 => K::Fade,
                    2 => K::FadeToColor {
                        color: estream_core::Rgba8::BLACK,
                    },
                    3 => K::Slide { direction },
                    4 => K::Swipe { direction },
                    5 => K::Wipe {
                        direction,
                        softness: 0.1,
                    },
                    7 => {
                        // Stinger: keep the current video, or ask for one.
                        if !matches!(current.kind, K::Stinger { .. }) {
                            self.choose_stinger();
                        }
                        return;
                    }
                    _ => {
                        // Custom shader: keep the current code if there is one.
                        if !matches!(current.kind, K::Shader { .. }) {
                            self.open_shader_editor();
                        }
                        return;
                    }
                };
                Transition {
                    kind,
                    duration_ms: if current.duration_ms == 0 {
                        300
                    } else {
                        current.duration_ms
                    },
                }
            }
            "duration" => Transition {
                duration_ms: value.clamp(50.0, 10_000.0) as u32,
                ..current.clone()
            },
            "point" => match current.kind.clone() {
                K::Stinger { file, .. } => Transition {
                    kind: K::Stinger {
                        file,
                        point_ms: value.clamp(0.0, current.duration_ms as f32) as u32,
                    },
                    ..current.clone()
                },
                _ => return,
            },
            "stinger-file" => {
                self.choose_stinger();
                return;
            }
            "direction" => {
                let d = Direction::ALL.get(i).copied().unwrap_or_default();
                let kind = match current.kind.clone() {
                    K::Slide { .. } => K::Slide { direction: d },
                    K::Swipe { .. } => K::Swipe { direction: d },
                    K::Wipe { softness, .. } => K::Wipe {
                        direction: d,
                        softness,
                    },
                    other => other,
                };
                Transition {
                    kind,
                    ..current.clone()
                }
            }
            _ => return,
        };
        if transition != current {
            self.exec(Command::SetTransition { transition });
        }
    }

    /// Picks a stinger video and makes it the transition (switching halfway by default).
    fn choose_stinger(&mut self) {
        use estream_project::{Transition, TransitionKind as K};
        let Some(path) = rfd::FileDialog::new()
            .set_title("Stinger video (with transparency)")
            .add_filter("Video", estream_media::player::VIDEO_EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        let Some(engine) = &self.engine else { return };
        let current = engine.snapshot().transition.clone();
        let duration_ms = match current.duration_ms {
            d if d >= 500 => d,
            _ => 1500,
        };
        let point_ms = match current.kind {
            K::Stinger { point_ms, .. } => point_ms.min(duration_ms),
            _ => duration_ms / 2,
        };
        self.exec(Command::SetTransition {
            transition: Transition {
                kind: K::Stinger {
                    file: path.to_string_lossy().into_owned(),
                    point_ms,
                },
                duration_ms,
            },
        });
    }

    fn open_shader_editor(&mut self) {
        let Some(engine) = &self.engine else { return };
        let code = match &engine.snapshot().transition.kind {
            estream_project::TransitionKind::Shader { code } => code.clone(),
            _ => estream_gpu::EXAMPLE_CUSTOM_TRANSITION.to_owned(),
        };
        self.shader_target = ShaderTarget::Transition;
        self.show_shader_dialog("Custom transition shader", TRANSITION_SHADER_HELP, code);
    }

    fn show_shader_dialog(&self, title: &str, help: &str, code: String) {
        if let Some(w) = self.window() {
            w.set_shader_title(title.into());
            w.set_shader_help(help.into());
            w.set_shader_code(code.into());
            w.set_shader_error("".into());
            w.set_shader_open(true);
        }
    }

    pub fn shader_example(&mut self) {
        let example = match self.shader_target {
            ShaderTarget::Transition => estream_gpu::EXAMPLE_CUSTOM_TRANSITION,
            ShaderTarget::Filter(..) => estream_gpu::EXAMPLE_CUSTOM_FILTER,
        };
        if let Some(w) = self.window() {
            w.set_shader_code(example.into());
            w.set_shader_error("".into());
        }
    }

    /// Validates and applies the shader being edited; errors stay in the dialog.
    pub fn shader_apply(&mut self, code: &str) {
        let Some(engine) = &self.engine else { return };
        let (command, done) = match &self.shader_target {
            ShaderTarget::Transition => {
                let duration_ms = match engine.snapshot().transition.duration_ms {
                    0 => 800,
                    d => d,
                };
                let transition = estream_project::Transition {
                    kind: estream_project::TransitionKind::Shader {
                        code: code.to_owned(),
                    },
                    duration_ms,
                };
                (
                    Command::SetTransition { transition },
                    "Custom transition applied",
                )
            }
            ShaderTarget::Filter(source, id) => {
                let Some(def) = engine.snapshot().source(*source).cloned() else {
                    return;
                };
                let mut filters = def.filters.clone();
                let Some(f) = filters.iter_mut().find(|f| &f.id == id) else {
                    return;
                };
                if !f.settings.is_object() {
                    f.settings = serde_json::json!({});
                }
                f.settings["code"] = serde_json::Value::from(code);
                (
                    Command::SetSourceFilters {
                        source: *source,
                        filters,
                    },
                    "Custom filter applied",
                )
            }
        };
        let result = engine.execute(command);
        let Some(w) = self.window() else { return };
        match result {
            Ok(_) => {
                w.set_shader_open(false);
                self.toast(done.into(), false);
                self.refresh();
            }
            Err(e) => {
                // Keep the user's text in the editor and show what to fix.
                let message = match e {
                    estream_engine::EngineError::Command(
                        estream_engine::CommandError::Invalid(message),
                    ) => message,
                    other => other.to_string(),
                };
                w.set_shader_code(code.into());
                w.set_shader_error(message.into());
            }
        }
    }

    /// Go Live on every enabled destination, or end the stream everywhere.
    fn toggle_stream(&mut self) {
        let Some(engine) = &self.engine else { return };
        if let Some(stream) = self.stream.take() {
            stream.stop_in_background(engine);
            self.stream_dests.clear();
            self.toast("Stream ended".into(), false);
            self.update_outputs();
            return;
        }
        let enabled = self.settings.enabled_destinations();
        if enabled.is_empty() {
            self.toast("Add a stream destination first".into(), true);
            self.open_stream_settings();
            return;
        }
        let dests: Vec<_> = enabled.iter().map(|(_, d)| d.clone()).collect();
        match estream_media::MultiStream::start(engine, &dests) {
            Ok(stream) => {
                let names: Vec<&str> = dests.iter().map(|d| d.name.as_str()).collect();
                self.toast(format!("Going live on {}", names.join(", ")), false);
                self.stream_dests = enabled.iter().map(|(i, _)| LiveDest::new(*i)).collect();
                self.stream = Some(stream);
            }
            Err(e) => {
                self.toast(format!("Couldn't go live: {e}"), true);
                self.open_stream_settings();
            }
        }
        self.update_outputs();
    }

    /// Start or stop one destination (by settings index) from the live panel.
    pub fn live_toggle(&mut self, config: usize) {
        let Some(engine) = &self.engine else { return };
        let Some(dest) = self
            .settings
            .destinations
            .get(config)
            .map(|d| d.to_destination())
        else {
            return;
        };
        let live = self.stream_dests.iter().position(|d| d.config == config);
        match (&mut self.stream, live) {
            (None, _) => match estream_media::MultiStream::start(engine, &[dest]) {
                Ok(stream) => {
                    self.stream = Some(stream);
                    self.stream_dests = vec![LiveDest::new(config)];
                }
                Err(e) => self.toast(format!("Couldn't go live: {e}"), true),
            },
            (Some(stream), Some(k)) if stream.destinations()[k].is_active() => {
                stream.stop_destination(engine, k);
                self.toast(format!("Ended the stream on {}", dest.name), false);
            }
            (Some(stream), Some(k)) => {
                stream.restart_destination(engine, k, &dest);
                self.stream_dests[k] = LiveDest::new(config);
            }
            (Some(stream), None) => {
                stream.add_destination(engine, &dest);
                self.stream_dests.push(LiveDest::new(config));
            }
        }
        self.update_outputs();
    }

    fn toggle_recording(&mut self) {
        let Some(engine) = &self.engine else { return };
        if let Some(rec) = self.recording.take() {
            match rec.stop_all(engine) {
                Ok(paths) => {
                    let message = match paths.as_slice() {
                        [one] => format!("Saved {}", one.display()),
                        many => format!(
                            "Saved {} files (one per canvas) in {}",
                            many.len(),
                            many.first()
                                .and_then(|p| p.parent())
                                .map_or(String::new(), |d| d.display().to_string())
                        ),
                    };
                    self.toast(message, false);
                    self.last_output = paths.into_iter().next();
                }
                Err(e) => self.toast(format!("Recording failed: {e}"), true),
            }
        } else {
            match estream_media::Recording::start(engine, &self.settings.recording) {
                Ok(rec) => {
                    self.recording = Some(rec);
                    self.toast("Recording started".into(), false);
                }
                Err(e) => self.toast(format!("Couldn't start recording: {e}"), true),
            }
        }
        self.update_outputs();
    }

    fn toggle_replay(&mut self) {
        let Some(engine) = &self.engine else { return };
        if let Some(replay) = self.replay.take() {
            let _ = replay.stop(engine);
            self.toast("Replay buffer stopped".into(), false);
        } else {
            match estream_media::ReplayBuffer::start(engine, &self.settings.recording) {
                Ok(r) => {
                    self.replay = Some(r);
                    let secs = self.settings.recording.replay_seconds;
                    self.toast(
                        format!(
                            "Replay buffer on — keeps the last {secs} s (Ctrl+Shift+R to save)"
                        ),
                        false,
                    );
                }
                Err(e) => self.toast(format!("Couldn't start the replay buffer: {e}"), true),
            }
        }
        self.update_outputs();
    }

    fn save_replay(&mut self) {
        let Some(replay) = &self.replay else {
            self.toast("Start the replay buffer first".into(), false);
            return;
        };
        match replay.save() {
            Ok(path) => {
                self.toast(format!("Replay saved: {}", path.display()), false);
                self.last_output = Some(path);
            }
            Err(e) => self.toast(format!("Couldn't save the replay: {e}"), true),
        }
    }

    /// Opens the file manager at the last saved recording (or the recording folder).
    fn reveal_output(&mut self) {
        let file = self.last_output.clone().filter(|p| p.exists());
        let folder = self.settings.recording.folder.clone();
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("explorer");
            match &file {
                Some(f) => c.arg(format!("/select,{}", f.display())),
                None => c.arg(&folder),
            };
            c
        } else if cfg!(target_os = "macos") {
            let mut c = std::process::Command::new("open");
            match &file {
                Some(f) => c.arg("-R").arg(f),
                None => c.arg(&folder),
            };
            c
        } else {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(file.as_ref().and_then(|f| f.parent()).unwrap_or(&folder));
            c
        };
        if let Err(e) = cmd.spawn() {
            self.toast(format!("Couldn't open the folder: {e}"), true);
        }
    }

    /// Updates the record button, timer and status message; surfaces encoder failures.
    fn update_outputs(&mut self) {
        let Some(w) = self.window() else { return };
        let failed = self
            .recording
            .as_ref()
            .and_then(|r| r.stats().error.lock().ok().and_then(|e| e.clone()));
        if let Some(error) = failed {
            if let (Some(engine), Some(rec)) = (&self.engine, self.recording.take()) {
                let _ = rec.stop(engine);
            }
            self.toast(format!("Recording stopped: {error}"), true);
        }
        self.update_stream();
        w.set_recording(self.recording.is_some());
        w.set_replay_active(self.replay.is_some());
        let Some(rec) = &self.recording else {
            w.set_status_message(
                if self.replay.is_some() {
                    "Replay buffer active"
                } else {
                    ""
                }
                .into(),
            );
            return;
        };
        if let Some(engine) = &self.engine {
            rec.poll(engine);
        }
        let secs = rec.elapsed().as_secs();
        w.set_record_time(
            format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60).into(),
        );
        let stats = rec.stats();
        let bytes = stats.bytes.load(std::sync::atomic::Ordering::Relaxed);
        let dropped = stats.dropped.load(std::sync::atomic::Ordering::Relaxed);
        let mbps = if secs > 0 {
            bytes as f64 * 8.0 / secs as f64 / 1e6
        } else {
            0.0
        };
        let encoder = stats.encoder.lock().map(|e| e.clone()).unwrap_or_default();
        w.set_status_message(
            format!(
                "● REC {:.1} MB · {mbps:.1} Mb/s · {dropped} dropped · {encoder}",
                bytes as f64 / 1e6
            )
            .into(),
        );
    }

    /// Go Live state and timer, per-destination failures, the live panel and the status bar.
    fn update_stream(&mut self) {
        use estream_media::StreamState;
        use std::sync::atomic::Ordering::Relaxed;
        let Some(w) = self.window() else { return };
        if let (Some(engine), Some(stream)) = (&self.engine, &mut self.stream) {
            stream.poll(engine);
        }

        // Report each destination's failure once; the others keep going.
        let mut failures = Vec::new();
        if let Some(stream) = &self.stream {
            for (k, live) in stream.destinations().iter().enumerate() {
                if let (Some(error), Some(d)) = (live.error(), self.stream_dests.get_mut(k))
                    && !d.error_shown
                {
                    d.error_shown = true;
                    failures.push(format!("{}: {error}", live.name()));
                }
            }
        }
        for f in failures {
            self.toast(f, true);
        }
        // Everything ended (stopped or failed): the stream is over.
        if self.stream.as_ref().is_some_and(|s| !s.is_active()) {
            if let (Some(engine), Some(stream)) = (&self.engine, self.stream.take()) {
                stream.stop_in_background(engine);
            }
            self.stream_dests.clear();
        }

        // Per-destination rows for every configured destination.
        let mut rows: Vec<LiveRow> = self
            .settings
            .destinations
            .iter()
            .map(|d| LiveRow {
                name: d.name.clone().into(),
                state: 0,
                info: if d.enabled {
                    "Off".into()
                } else {
                    "Off · not included in Go Live".into()
                },
                health: 0,
            })
            .collect();
        let mut summary = Vec::new();
        let (mut any_live, mut any_connecting, mut worst) = (false, false, 0);
        if let Some(stream) = &self.stream {
            for (k, live) in stream.destinations().iter().enumerate() {
                let Some(tracked) = self.stream_dests.get_mut(k) else {
                    continue;
                };
                let stats = &live.stats;
                let bytes = stats.bytes.load(Relaxed);
                let (at, prev, _) = tracked.rate;
                let dt = at.elapsed().as_secs_f64();
                if dt >= 1.0 {
                    let kbps = bytes.saturating_sub(prev) as f64 * 8.0 / dt / 1000.0;
                    tracked.rate = (Instant::now(), bytes, kbps);
                }
                let kbps = tracked.rate.2;
                let encoder = stream.encoder_stats(k);
                let frames = encoder.map_or(0, |e| e.frames.load(Relaxed)).max(1);
                let dropped = stats.network.dropped.load(Relaxed)
                    + encoder.map_or(0, |e| e.dropped.load(Relaxed));
                let percent = dropped as f64 * 100.0 / frames as f64;
                let (state, health, info) = match live.state() {
                    StreamState::Live => {
                        any_live = true;
                        let congested = stats.network.queue_ms.load(Relaxed) > 300 || percent > 1.0;
                        summary.push(format!("{} {:.1} Mb/s", live.name(), kbps / 1000.0));
                        (
                            2,
                            if congested { 2 } else { 1 },
                            format!("Live · {kbps:.0} kb/s · {dropped} dropped ({percent:.1}%)"),
                        )
                    }
                    StreamState::Connecting => {
                        any_connecting = true;
                        summary.push(format!("{} connecting", live.name()));
                        (1, 2, format!("Connecting to {}…", stats.destination))
                    }
                    StreamState::Reconnecting => {
                        summary.push(format!("{} reconnecting", live.name()));
                        (
                            3,
                            3,
                            format!(
                                "Reconnecting (attempt {})…",
                                stats.network.attempt.load(Relaxed)
                            ),
                        )
                    }
                    StreamState::Stopped => match live.error() {
                        Some(error) => (4, 3, error),
                        None => (0, 0, "Off".into()),
                    },
                };
                if state != 0 && state != 4 {
                    worst = worst.max(health);
                }
                if let Some(row) = rows.get_mut(tracked.config) {
                    *row = LiveRow {
                        name: live.name().into(),
                        state,
                        info: info.into(),
                        health,
                    };
                }
            }
        }
        w.set_live_rows(ModelRc::new(VecModel::from(rows)));

        let Some(stream) = &self.stream else {
            w.set_stream_state(0);
            w.set_stream_health(0);
            w.set_stream_info("".into());
            return;
        };
        w.set_stream_state(if any_live {
            2
        } else if any_connecting {
            1
        } else {
            3
        });
        let secs = stream.elapsed().as_secs();
        w.set_stream_time(
            format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60).into(),
        );
        w.set_stream_health(worst.max(1));
        w.set_stream_info(summary.join(" · ").into());
    }

    // ------------------------------------------------------------ settings dialog

    pub fn refresh_settings_form(&mut self) {
        let Some(w) = self.window() else { return };
        // Always show an editor when there is something to edit.
        if self
            .dest_selected
            .is_none_or(|i| i >= self.settings.destinations.len())
        {
            self.dest_selected = (!self.settings.destinations.is_empty()).then_some(0);
        }
        let (canvas_ids, canvas_labels) = self.canvas_choices();
        let index_of = |c: Option<CanvasId>| {
            c.and_then(|c| canvas_ids.iter().position(|x| *x == c))
                .unwrap_or(0) as i32
        };
        use estream_media::{AudioCodec, Container, Preset, RateControl, VideoCodec, encode};
        let r = &self.settings.recording;
        let encoders = encode::available(r.codec);
        let mut names = vec![SharedString::from("Automatic (best available)")];
        names.extend(encoders.iter().map(|e| SharedString::from(e.label)));
        let encoder = encoders
            .iter()
            .position(|e| e.name == r.encoder)
            .map_or(0, |i| i + 1);
        let index = |v: usize| v as i32;
        w.set_settings_form(SettingsForm {
            folder: r.folder.to_string_lossy().into_owned().into(),
            filename: r.filename.clone().into(),
            container: index(
                Container::ALL
                    .iter()
                    .position(|c| *c == r.container)
                    .unwrap_or(0),
            ),
            codec: index(
                VideoCodec::ALL
                    .iter()
                    .position(|c| *c == r.codec)
                    .unwrap_or(0),
            ),
            encoders: ModelRc::new(VecModel::from(names)),
            encoder: encoder as i32,
            rate_control: match r.rate_control {
                RateControl::Cbr => 0,
                RateControl::Vbr => 1,
                RateControl::ConstantQuality => 2,
            },
            bitrate: r.bitrate_kbps as f32,
            quality: r.quality as f32,
            keyframe: r.keyframe_seconds as f32,
            preset: match r.preset {
                Preset::Performance => 0,
                Preset::Balanced => 1,
                Preset::Quality => 2,
            },
            audio_codec: match r.audio_codec {
                AudioCodec::Aac => 0,
                AudioCodec::Opus => 1,
            },
            audio_bitrate: r.audio_bitrate_kbps as f32,
            tracks: r.tracks as i32,
            replay_seconds: r.replay_seconds as f32,
            auto_remux: r.auto_remux,
            theme: match self.settings.theme {
                ThemeMode::Dark => 0,
                ThemeMode::Light => 1,
                ThemeMode::System => 2,
            },
            ffmpeg: format!("FFmpeg libavutil {}", estream_media::version()).into(),
            canvas_names: ModelRc::new(VecModel::from(canvas_labels.clone())),
            rec_canvas: if r.all_canvases {
                canvas_ids.len() as i32
            } else {
                index_of(r.canvas)
            },
            rec_canvas_names: ModelRc::new(VecModel::from(
                canvas_labels
                    .iter()
                    .cloned()
                    .chain(std::iter::once(SharedString::from("All canvases")))
                    .collect::<Vec<_>>(),
            )),
            dest_canvas: self
                .dest_selected
                .and_then(|i| self.settings.destinations.get(i))
                .map_or(0, |d| index_of(d.stream.canvas)),
            ..stream_form(&self.settings.destinations, self.dest_selected)
        });
    }

    /// The current collection's canvases (ids and names), for output canvas pickers.
    fn canvas_choices(&self) -> (Vec<CanvasId>, Vec<SharedString>) {
        let Some(engine) = &self.engine else {
            return (Vec::new(), Vec::new());
        };
        let snapshot = engine.snapshot();
        snapshot
            .canvases
            .iter()
            .map(|c| {
                (
                    c.id,
                    SharedString::from(format!("{} ({} × {})", c.name, c.width, c.height)),
                )
            })
            .unzip()
    }

    pub fn settings_set(&mut self, field: &str, value: f32) {
        use estream_media::{AudioCodec, Container, Preset, RateControl, VideoCodec, encode};
        let i = value.round().max(0.0) as usize;
        let (canvas_ids, _) = self.canvas_choices();
        if field == "dest-canvas"
            && let Some(d) = self
                .dest_selected
                .and_then(|i| self.settings.destinations.get_mut(i))
        {
            d.stream.canvas = canvas_ids.get(i).copied();
        }
        // Destination list.
        let live_configs: Vec<usize> = self.stream_dests.iter().map(|d| d.config).collect();
        match field {
            "dest-select" => self.dest_selected = Some(i),
            "dest-add" => {
                let n = self.settings.destinations.len() + 1;
                self.settings
                    .destinations
                    .push(crate::settings::DestinationConfig {
                        name: format!("Destination {n}"),
                        ..Default::default()
                    });
                self.dest_selected = Some(self.settings.destinations.len() - 1);
            }
            "dest-remove" if live_configs.contains(&i) => {
                self.toast(
                    "End the stream before removing a live destination".into(),
                    true,
                );
                return;
            }
            "dest-remove" if i < self.settings.destinations.len() => {
                self.settings.destinations.remove(i);
                // Keep live destinations pointing at the right settings entries.
                for d in &mut self.stream_dests {
                    if d.config > i {
                        d.config -= 1;
                    }
                }
                self.dest_selected = match self.dest_selected {
                    _ if self.settings.destinations.is_empty() => None,
                    Some(s) if s >= i => Some(s.saturating_sub(1)),
                    other => other,
                };
            }
            "dest-enable" => {
                if let Some(d) = self.settings.destinations.get_mut(i) {
                    d.enabled = !d.enabled;
                }
            }
            _ => {}
        }
        // Editor fields act on the selected destination.
        let selected = self.dest_selected;
        if let Some(s) = selected
            .and_then(|i| self.settings.destinations.get_mut(i))
            .map(|d| &mut d.stream)
        {
            Self::set_stream_field(s, field, value, i);
        }
        let r = &mut self.settings.recording;
        match field {
            "container" => r.container = Container::ALL.get(i).copied().unwrap_or_default(),
            "codec" => {
                r.codec = VideoCodec::ALL.get(i).copied().unwrap_or_default();
                r.encoder = "auto".into();
            }
            "encoder" => {
                r.encoder = if i == 0 {
                    "auto".into()
                } else {
                    encode::available(r.codec)
                        .get(i - 1)
                        .map_or("auto".into(), |e| e.name.to_owned())
                }
            }
            "rate-control" => {
                r.rate_control = [
                    RateControl::Cbr,
                    RateControl::Vbr,
                    RateControl::ConstantQuality,
                ]
                .get(i)
                .copied()
                .unwrap_or_default()
            }
            "bitrate" => r.bitrate_kbps = value.clamp(500.0, 150_000.0) as u32,
            "quality" => r.quality = value.clamp(1.0, 51.0) as u32,
            "keyframe" => r.keyframe_seconds = value.clamp(1.0, 10.0) as u32,
            "preset" => {
                r.preset = [Preset::Performance, Preset::Balanced, Preset::Quality]
                    .get(i)
                    .copied()
                    .unwrap_or_default()
            }
            "audio-codec" => {
                r.audio_codec = if i == 1 {
                    AudioCodec::Opus
                } else {
                    AudioCodec::Aac
                }
            }
            "audio-bitrate" => r.audio_bitrate_kbps = value.clamp(64.0, 512.0) as u32,
            "tracks" => r.tracks = (value.round() as i32).clamp(1, 63) as u8,
            "replay-seconds" => r.replay_seconds = value.clamp(5.0, 300.0) as u32,
            "auto-remux" => r.auto_remux = value > 0.5,
            "rec-canvas" => {
                let ids = &canvas_ids;
                r.all_canvases = i >= ids.len();
                r.canvas = ids.get(i).copied();
            }
            "theme" => {
                self.settings.theme = match i {
                    1 => ThemeMode::Light,
                    2 => ThemeMode::System,
                    _ => ThemeMode::Dark,
                };
                self.apply_theme();
            }
            _ => {}
        }
        self.settings.save(&self.paths.settings());
        self.refresh_settings_form();
    }

    /// Applies a numeric editor field to a destination's stream settings.
    fn set_stream_field(s: &mut estream_media::StreamSettings, field: &str, value: f32, i: usize) {
        use estream_media::{Service, VideoCodec, encode};
        match field {
            "stream-service" => {
                s.service = Service::ALL.get(i).copied().unwrap_or_default();
                if s.service != Service::Custom {
                    s.server = s.service.default_server().to_owned();
                }
            }
            "stream-codec" => {
                s.codec = VideoCodec::ALL.get(i).copied().unwrap_or_default();
                s.encoder = "auto".into();
            }
            "stream-encoder" => {
                s.encoder = if i == 0 {
                    "auto".into()
                } else {
                    encode::available(s.codec)
                        .get(i - 1)
                        .map_or("auto".into(), |e| e.name.to_owned())
                }
            }
            "stream-bitrate" => s.bitrate_kbps = value.clamp(500.0, 50_000.0) as u32,
            "stream-audio-bitrate" => s.audio_bitrate_kbps = value.clamp(64.0, 320.0) as u32,
            "stream-track" => s.audio_track = i.min(5) as u8,
            "stream-reconnect" => s.reconnect = value > 0.5,
            _ => {}
        }
    }

    pub fn settings_text(&mut self, field: &str, value: &str) {
        let value = value.trim();
        let selected = self
            .dest_selected
            .and_then(|i| self.settings.destinations.get_mut(i));
        match (field, selected) {
            ("dest-name", Some(d)) if !value.is_empty() => d.name = value.to_owned(),
            ("stream-server", Some(d)) => d.stream.server = value.to_owned(),
            ("stream-key", Some(d)) => d.set_key(value),
            ("folder", _) if !value.is_empty() => {
                self.settings.recording.folder = PathBuf::from(value)
            }
            ("filename", _) if !value.is_empty() => {
                self.settings.recording.filename = value.to_owned()
            }
            _ => return,
        }
        self.settings.save(&self.paths.settings());
        self.refresh_settings_form();
    }

    pub fn settings_browse(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Recording folder")
            .set_directory(&self.settings.recording.folder)
            .pick_folder()
        {
            self.settings.recording.folder = folder;
            self.settings.save(&self.paths.settings());
            self.refresh_settings_form();
        }
    }

    // ------------------------------------------------------------ audio

    /// The source being edited: the selected item's source, or a source picked in the mixer.
    fn current_source(&self, snapshot: &Project) -> Option<SourceId> {
        match self.selection {
            Some(sel) => snapshot
                .scenes
                .iter()
                .find_map(|s| s.find_item(sel))
                .and_then(|i| i.source_id()),
            None => self.selected_source,
        }
    }

    fn has_audio(&self, kind: &str) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|e| e.registry().get(kind).is_some_and(|t| t.has_audio()))
    }

    fn audio_inspect(&self, snapshot: &Project) -> AudioInspect {
        let Some(source) = self
            .current_source(snapshot)
            .and_then(|id| snapshot.source(id))
            .filter(|s| self.has_audio(&s.kind))
        else {
            return AudioInspect::default();
        };
        let a = &source.audio;
        let filters: Vec<FilterRow> = a
            .filters
            .iter()
            .map(|f| {
                let kind = estream_engine::audio_engine::filters::kind(&f.kind);
                let params: Vec<FilterParamRow> = kind
                    .map(|k| k.params)
                    .unwrap_or_default()
                    .iter()
                    .map(|p| FilterParamRow {
                        key: p.key.into(),
                        label: p.label.into(),
                        value: f
                            .settings
                            .get(p.key)
                            .and_then(serde_json::Value::as_f64)
                            .map_or(p.default, |v| v as f32),
                        minimum: p.min,
                        maximum: p.max,
                        step: p.step,
                        unit: p.unit.into(),
                    })
                    .collect();
                FilterRow {
                    id: f.id.clone().into(),
                    name: kind.map_or(f.kind.as_str(), |k| k.name).into(),
                    enabled: f.enabled,
                    params: ModelRc::new(VecModel::from(params)),
                }
            })
            .collect();
        AudioInspect {
            has: true,
            volume_db: a.volume_db,
            muted: a.muted,
            balance: a.balance,
            mono: a.mono,
            sync_ms: a.sync_offset_ms as f32,
            monitoring: match a.monitoring {
                Monitoring::Off => 0,
                Monitoring::MonitorOnly => 1,
                Monitoring::MonitorAndOutput => 2,
            },
            tracks: a.tracks as i32,
            global: a.global,
            filters: ModelRc::new(VecModel::from(filters)),
        }
    }

    /// Applies `change` to a source's mixer settings (one undoable command).
    fn update_audio(&mut self, source: SourceId, change: impl FnOnce(&mut AudioConfig)) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(def) = snapshot.source(source) else {
            return;
        };
        let mut audio = def.audio.clone();
        change(&mut audio);
        if audio != def.audio {
            self.exec(Command::UpdateSourceAudio { source, audio });
        }
    }

    fn update_current_audio(&mut self, change: impl FnOnce(&mut AudioConfig)) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        if let Some(source) = self.current_source(&snapshot) {
            self.update_audio(source, change);
        }
    }

    pub fn audio_set(&mut self, field: &str, value: f32) {
        self.update_current_audio(|a| match field {
            "volume" => a.volume_db = value.clamp(-100.0, 26.0),
            "muted" => a.muted = value > 0.5,
            "balance" => a.balance = value.clamp(0.0, 1.0),
            "mono" => a.mono = value > 0.5,
            "global" => a.global = value > 0.5,
            "sync" => a.sync_offset_ms = value.clamp(0.0, 2000.0).round() as u32,
            "tracks" => a.tracks = (value.round() as i32).clamp(0, 63) as u8,
            "monitoring" => {
                a.monitoring = match value.round() as i32 {
                    1 => Monitoring::MonitorOnly,
                    2 => Monitoring::MonitorAndOutput,
                    _ => Monitoring::Off,
                }
            }
            _ => {}
        });
    }

    pub fn filter_add(&mut self, index: i32) {
        let Some(kind) = estream_engine::audio_engine::filters::KINDS.get(index.max(0) as usize)
        else {
            return;
        };
        self.update_current_audio(|a| {
            a.filters.push(AudioFilterDef {
                id: uuid::Uuid::now_v7().to_string(),
                kind: kind.kind.into(),
                enabled: true,
                settings: serde_json::json!({}),
            });
        });
    }

    pub fn filter_remove(&mut self, id: &str) {
        self.update_current_audio(|a| a.filters.retain(|f| f.id != id));
    }

    pub fn filter_toggle(&mut self, id: &str, enabled: bool) {
        self.update_current_audio(|a| {
            if let Some(f) = a.filters.iter_mut().find(|f| f.id == id) {
                f.enabled = enabled;
            }
        });
    }

    pub fn filter_param(&mut self, id: &str, key: &str, value: f32) {
        self.update_current_audio(|a| {
            if let Some(f) = a.filters.iter_mut().find(|f| f.id == id) {
                if !f.settings.is_object() {
                    f.settings = serde_json::json!({});
                }
                f.settings[key] = serde_json::Value::from(value as f64);
            }
        });
    }

    pub fn filter_move(&mut self, id: &str, up: bool) {
        self.update_current_audio(|a| {
            if let Some(i) = a.filters.iter().position(|f| f.id == id) {
                let j = if up {
                    i.checked_sub(1)
                } else {
                    (i + 1 < a.filters.len()).then_some(i + 1)
                };
                if let Some(j) = j {
                    a.filters.swap(i, j);
                }
            }
        });
    }

    // ------------------------------------------------------------ media controls

    /// The player of the selected media source, if any.
    fn selected_player(&self) -> Option<Arc<estream_media::player::Player>> {
        let snapshot = self.engine.as_ref()?.snapshot();
        let source = self
            .current_source(&snapshot)
            .and_then(|id| snapshot.source(id))
            .filter(|s| s.kind == "media")?;
        estream_media::player::get(&source.id.to_string())
    }

    fn media_inspect(&self) -> MediaInspect {
        use estream_media::player::PlayState;
        let Some(player) = self.selected_player() else {
            return MediaInspect::default();
        };
        let s = player.status();
        let clock = |d: Duration| {
            let secs = d.as_secs();
            if secs >= 3600 {
                format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
            } else {
                format!("{}:{:02}", secs / 60, secs % 60)
            }
        };
        let time = match s.duration {
            Some(d) => format!("{} / {}", clock(s.position), clock(d)),
            None => clock(s.position),
        };
        MediaInspect {
            has: true,
            item: match (&s.state, s.item.is_empty()) {
                (PlayState::Idle, _) | (_, true) => s
                    .error
                    .clone()
                    .unwrap_or_else(|| "Choose a file".into())
                    .into(),
                _ => s.item.clone().into(),
            },
            index: if s.count > 1 {
                format!("{} / {}", s.index + 1, s.count).into()
            } else {
                "".into()
            },
            time: time.into(),
            progress: s.duration.map_or(0.0, |d| {
                (s.position.as_secs_f32() / d.as_secs_f32().max(0.001)).clamp(0.0, 1.0)
            }),
            paused: s.state == PlayState::Paused,
            ended: s.state == PlayState::Ended,
        }
    }

    pub fn media_control(&mut self, command: &str) {
        let Some(player) = self.selected_player() else {
            return;
        };
        match command {
            "toggle" => player.toggle_pause(),
            "restart" => {
                player.set_paused(false);
                player.restart();
            }
            "next" => player.next(),
            "previous" => player.previous(),
            _ => {}
        }
    }

    pub fn media_seek(&mut self, fraction: f32) {
        if let Some(player) = self.selected_player()
            && let Some(duration) = player.status().duration
        {
            player.seek(duration.mul_f32(fraction.clamp(0.0, 1.0)));
        }
    }

    // ------------------------------------------------------------ browser sources

    /// The id of the selected source if it is a browser source.
    fn selected_browser(&self) -> Option<String> {
        let snapshot = self.engine.as_ref()?.snapshot();
        let source = self
            .current_source(&snapshot)
            .and_then(|id| snapshot.source(id))
            .filter(|s| s.kind == "browser")?;
        Some(source.id.to_string())
    }

    /// Whether a browser source is selected, and its page status.
    fn browser_inspect(&self) -> (bool, String) {
        use estream_capture::media::browser::{HelperState, host};
        let Some(id) = self.selected_browser() else {
            return (false, String::new());
        };
        let status = match host().page(&id) {
            None => return (true, String::new()),
            Some((_, HelperState::Missing(e), _)) => e,
            Some((_, HelperState::Restarting, _)) => "Browser stopped; restarting…".into(),
            Some((page, _, _)) => match (page.error, page.loaded) {
                (Some(e), _) => e,
                (None, true) => "Page loaded".into(),
                (None, false) => "Loading…".into(),
            },
        };
        (true, status)
    }

    pub fn browser_reload(&mut self) {
        if let Some(id) = self.selected_browser() {
            estream_capture::media::browser::reload(&id);
        }
    }

    // ------------------------------------------------------------ video filters

    /// Inspector rows for the selected source's video filters.
    fn refresh_video_filters(&self, w: &AppWindow, snapshot: &Project) {
        use estream_gpu::filter::{self, KINDS};
        let source = self
            .current_source(snapshot)
            .and_then(|id| snapshot.source(id))
            .filter(|s| {
                self.engine
                    .as_ref()
                    .and_then(|e| e.registry().get(&s.kind))
                    .is_some_and(|t| t.has_video())
            });
        let Some(source) = source else {
            w.set_video_filter_kinds(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            w.set_video_filters(ModelRc::new(VecModel::from(Vec::<VideoFilterRow>::new())));
            return;
        };
        let mut kinds = vec![SharedString::from("+ Add filter…")];
        kinds.extend(KINDS.iter().map(|k| SharedString::from(k.name)));
        w.set_video_filter_kinds(ModelRc::new(VecModel::from(kinds)));
        let rows: Vec<VideoFilterRow> = source
            .filters
            .iter()
            .filter_map(|f| {
                let info = filter::kind(&f.kind)?;
                let value = |key: &str, default: f32| {
                    f.settings
                        .get(key)
                        .and_then(|v| v.as_f64())
                        .map_or(default, |v| v as f32)
                };
                let params: Vec<FilterParamRow> = info
                    .params
                    .iter()
                    .map(|p| FilterParamRow {
                        key: p.key.into(),
                        label: p.label.into(),
                        value: value(p.key, p.default),
                        minimum: p.min,
                        maximum: p.max,
                        step: p.step,
                        unit: p.unit.into(),
                    })
                    .collect();
                let file = f
                    .settings
                    .get("file")
                    .and_then(|v| v.as_str())
                    .map(|p| {
                        std::path::Path::new(p)
                            .file_name()
                            .map_or(p.to_owned(), |n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default();
                Some(VideoFilterRow {
                    id: f.id.clone().into(),
                    name: info.name.into(),
                    enabled: f.enabled,
                    params: ModelRc::new(VecModel::from(params)),
                    file_label: info.file.map_or("", |f| f.label).into(),
                    file: file.into(),
                    code: info.code,
                })
            })
            .collect();
        w.set_video_filters(ModelRc::new(VecModel::from(rows)));
    }

    /// Applies `change` to the selected source's video filters (one undoable step).
    fn update_video_filters(&mut self, change: impl FnOnce(&mut Vec<estream_project::FilterDef>)) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(source) = self.current_source(&snapshot) else {
            return;
        };
        let Some(def) = snapshot.source(source) else {
            return;
        };
        let mut filters = def.filters.clone();
        change(&mut filters);
        if filters != def.filters {
            self.exec(Command::SetSourceFilters { source, filters });
        }
    }

    pub fn vfilter_add(&mut self, index: i32) {
        let Some(kind) = estream_gpu::filter::KINDS.get(index.max(0) as usize) else {
            return;
        };
        let id = uuid::Uuid::now_v7().to_string();
        let settings = if kind.code {
            serde_json::json!({ "code": estream_gpu::EXAMPLE_CUSTOM_FILTER })
        } else {
            serde_json::json!({})
        };
        self.update_video_filters(|f| {
            f.push(estream_project::FilterDef {
                id: id.clone(),
                kind: kind.kind.into(),
                enabled: true,
                settings,
            })
        });
        // Filters that need a file ask for it right away.
        if kind.file.is_some() {
            self.vfilter_browse(&id);
        }
    }

    pub fn vfilter_remove(&mut self, id: &str) {
        self.update_video_filters(|f| f.retain(|x| x.id != id));
    }

    pub fn vfilter_toggle(&mut self, id: &str, enabled: bool) {
        self.update_video_filters(|f| {
            if let Some(x) = f.iter_mut().find(|x| x.id == id) {
                x.enabled = enabled;
            }
        });
    }

    pub fn vfilter_param(&mut self, id: &str, key: &str, value: f32) {
        self.update_video_filters(|f| {
            if let Some(x) = f.iter_mut().find(|x| x.id == id) {
                if !x.settings.is_object() {
                    x.settings = serde_json::json!({});
                }
                x.settings[key] = serde_json::Value::from(value as f64);
            }
        });
    }

    pub fn vfilter_move(&mut self, id: &str, up: bool) {
        self.update_video_filters(|f| {
            if let Some(i) = f.iter().position(|x| x.id == id) {
                let j = if up {
                    i.checked_sub(1)
                } else {
                    (i + 1 < f.len()).then_some(i + 1)
                };
                if let Some(j) = j {
                    f.swap(i, j);
                }
            }
        });
    }

    pub fn vfilter_browse(&mut self, id: &str) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(def) = self
            .current_source(&snapshot)
            .and_then(|s| snapshot.source(s))
            .and_then(|s| s.filters.iter().find(|f| f.id == id))
        else {
            return;
        };
        let Some(file) = estream_gpu::filter::kind(&def.kind).and_then(|k| k.file) else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title(file.label)
            .add_filter(file.label, file.extensions)
            .pick_file()
        else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        self.update_video_filters(|f| {
            if let Some(x) = f.iter_mut().find(|x| x.id == id) {
                if !x.settings.is_object() {
                    x.settings = serde_json::json!({});
                }
                x.settings["file"] = serde_json::Value::from(path);
            }
        });
    }

    pub fn vfilter_code(&mut self, id: &str) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(source) = self.current_source(&snapshot) else {
            return;
        };
        let Some(def) = snapshot
            .source(source)
            .and_then(|s| s.filters.iter().find(|f| f.id == id))
        else {
            return;
        };
        let code = def
            .settings
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or(estream_gpu::EXAMPLE_CUSTOM_FILTER)
            .to_owned();
        self.shader_target = ShaderTarget::Filter(source, id.to_owned());
        self.show_shader_dialog("Custom filter shader", FILTER_SHADER_HELP, code);
    }

    pub fn mixer_select(&mut self, id: &str) {
        let Some(source) = parse_id(id).map(SourceId) else {
            return;
        };
        // Prefer selecting the scene item that shows this source.
        let item = self.canvas_and_scene().and_then(|(snapshot, _, scene)| {
            let scene = snapshot.scene(scene?)?;
            let mut found = None;
            scene.walk(&mut |i| {
                if found.is_none() && i.source_id() == Some(source) {
                    found = Some(i.id);
                }
            });
            found
        });
        self.selection = item;
        self.selected_source = if item.is_none() { Some(source) } else { None };
        self.refresh();
    }

    pub fn mixer_fader(&mut self, id: &str, position: f32) {
        let Some(source) = parse_id(id).map(SourceId) else {
            return;
        };
        self.update_audio(source, |a| a.volume_db = fader_to_db(position));
    }

    pub fn mixer_mute(&mut self, id: &str) {
        let Some(source) = parse_id(id).map(SourceId) else {
            return;
        };
        self.update_audio(source, |a| a.muted = !a.muted);
    }

    /// Rebuilds mixer strips from the latest meters (called ~30 times per second).
    fn update_mixer(&mut self) {
        let Some(w) = self.window() else { return };
        let Some(engine) = &self.engine else { return };
        let meters = engine.audio_meters();
        let snapshot = engine.snapshot();
        let infos = engine.source_infos();
        let current = self.current_source(&snapshot);
        let now = Instant::now();
        let holds = &mut self.meter_holds;
        let mut strip = |id: SourceId, reading: estream_engine::audio_engine::MeterReading| {
            let peak = [
                meter_position(reading.peak[0]),
                meter_position(reading.peak[1]),
            ];
            let hold = holds.entry(id).or_insert([(0.0, now); 2]);
            for ch in 0..2 {
                if peak[ch] >= hold[ch].0
                    || now.duration_since(hold[ch].1) > Duration::from_millis(1500)
                {
                    hold[ch] = (peak[ch], now);
                }
            }
            MixerStrip {
                id: id_str(id),
                level_l: peak[0],
                level_r: peak[1],
                rms_l: meter_position(reading.rms[0]),
                rms_r: meter_position(reading.rms[1]),
                hold_l: hold[0].0,
                hold_r: hold[1].0,
                ..Default::default()
            }
        };
        let strips: Vec<MixerStrip> = meters
            .sources
            .iter()
            .filter_map(|(id, meter)| {
                let def = snapshot.source(*id)?;
                let mut s = strip(*id, meter.read());
                s.name = def.name.clone().into();
                s.kind = def.kind.clone().into();
                s.volume_db = def.audio.volume_db;
                s.fader = db_to_fader(def.audio.volume_db);
                s.muted = def.audio.muted;
                s.global = def.audio.global;
                s.monitoring = def.audio.monitoring != Monitoring::Off;
                s.selected = Some(*id) == current;
                s.problem = status_problem(infos.get(id)).into();
                Some(s)
            })
            .collect();
        let mut master = strip(SourceId(uuid::Uuid::nil()), meters.master.read());
        master.name = "Master (track 1)".into();
        w.set_mixer_master(master);
        sync_model(&self.mixer_model, strips);
    }

    // ------------------------------------------------------------ source properties

    /// Rebuilds the generic property rows when the selected source or its settings change.
    fn refresh_props(&mut self, snapshot: &Project) {
        let source = self
            .current_source(snapshot)
            .and_then(|id| snapshot.source(id));
        let Some(source) = source else {
            self.props = None;
            self.props_model.set_vec(Vec::new());
            return;
        };
        let key = (
            source.id,
            source.settings.to_string(),
            self.props_generation,
        );
        if self.props.as_ref().is_some_and(|p| p.0 == key) {
            return;
        }
        let Some(schema) = self.engine.as_ref().and_then(|e| {
            e.registry()
                .get(&source.kind)
                .map(|t| t.schema(&source.settings))
        }) else {
            self.props = None;
            self.props_model.set_vec(Vec::new());
            return;
        };
        let settings = schema.with_defaults(&source.settings);
        let rows = schema
            .properties
            .iter()
            .map(|p| prop_row(p, &settings[&p.key]))
            .collect();
        sync_model(&self.props_model, rows);
        self.props = Some((key, schema));
    }

    fn prop(&self, key: &str) -> Option<estream_engine::Property> {
        self.props.as_ref()?.1.get(key).cloned()
    }

    pub fn prop_flag(&mut self, key: &str, value: bool) {
        self.set_setting(key, value.into());
    }

    pub fn prop_number(&mut self, key: &str, value: f32) {
        let value = match self.prop(key).map(|p| p.kind) {
            Some(PropKind::Int { min, max, .. }) => {
                serde_json::Value::from((value.round() as i64).clamp(min, max))
            }
            Some(PropKind::Float { min, max, .. }) => {
                serde_json::Value::from((value as f64).clamp(min, max))
            }
            _ => return,
        };
        self.set_setting(key, value);
    }

    pub fn prop_text(&mut self, key: &str, value: &str) {
        match self.prop(key).map(|p| p.kind) {
            Some(PropKind::Color) => match parse_hex_color(value) {
                Some(c) => self.set_setting(key, serde_json::to_value(c).unwrap_or_default()),
                None => self.toast("Use a color like #ff8800 or #ff880080".into(), true),
            },
            Some(_) => self.set_setting(key, value.into()),
            None => {}
        }
    }

    pub fn prop_color(&mut self, key: &str, color: slint::Color) {
        self.set_setting(
            key,
            serde_json::to_value(from_color(color)).unwrap_or_default(),
        );
    }

    pub fn prop_index(&mut self, key: &str, index: i32) {
        if let Some(PropKind::Select { options }) = self.prop(key).map(|p| p.kind)
            && let Some(option) = options.get(index.max(0) as usize)
        {
            self.set_setting(key, option.value.clone());
        }
    }

    pub fn prop_browse(&mut self, key: &str) {
        let Some(PropKind::Path { filters }) = self.prop(key).map(|p| p.kind) else {
            return;
        };
        let filters: Vec<&str> = filters.iter().map(String::as_str).collect();
        let mut dialog = rfd::FileDialog::new().set_title("Choose a file");
        if !filters.is_empty() {
            dialog = dialog.add_filter("Supported files", &filters);
        }
        if let Some(path) = dialog.pick_file() {
            self.set_setting(key, path.to_string_lossy().into_owned().into());
        }
    }

    /// Re-enumerates devices/windows for the property dropdowns.
    pub fn prop_refresh(&mut self) {
        self.props_generation += 1;
        self.refresh();
    }

    // ------------------------------------------------------------ inspector

    pub fn inspect_edit(&mut self, field: &str, value: f32) {
        let Some((scene, item)) = self.selected() else {
            return;
        };
        let geom = self.geoms.iter().find(|g| g.id == item.id).cloned();
        let mut t = item.transform;
        let mut crop = item.crop;
        let mut patch = ItemPatch::default();
        match field {
            "x" => t.position.x = value,
            "y" => t.position.y = value,
            "rotation" => t.rotation = value,
            "w" | "h" => {
                let Some(g) = geom else { return };
                let mut size = g.box_size;
                if field == "w" {
                    size.x = value.max(1.0);
                } else {
                    size.y = value.max(1.0);
                }
                match t.bounds_mode {
                    BoundsMode::None if g.content.x > 0.0 && g.content.y > 0.0 => {
                        t.scale = size / g.content
                    }
                    BoundsMode::None => return,
                    _ => t.bounds = size,
                }
            }
            "opacity" => {
                self.update_selected(ItemPatch {
                    opacity: Some(value.clamp(0.0, 1.0)),
                    ..Default::default()
                });
                return;
            }
            "crop-l" | "crop-r" | "crop-t" | "crop-b" => {
                let v = value.max(0.0).round() as u32;
                match field {
                    "crop-l" => crop.left = v,
                    "crop-r" => crop.right = v,
                    "crop-t" => crop.top = v,
                    _ => crop.bottom = v,
                }
                patch.crop = Some(crop);
                self.exec(Command::UpdateItem {
                    scene,
                    item: item.id,
                    patch,
                });
                return;
            }
            _ => return,
        }
        patch.transform = Some(t);
        self.exec(Command::UpdateItem {
            scene,
            item: item.id,
            patch,
        });
    }

    pub fn inspect_text(&mut self, field: &str, value: &str) {
        let name = value.trim();
        if field != "name" || name.is_empty() {
            return;
        }
        let Some((_, item)) = self.selected() else {
            // A source picked in the mixer (no item in this scene).
            if let Some(source) = self.selected_source {
                self.exec(Command::RenameSource {
                    source,
                    name: name.into(),
                });
            }
            return;
        };
        match item.source_id() {
            Some(source) => {
                self.exec(Command::RenameSource {
                    source,
                    name: name.into(),
                });
            }
            None => self.update_selected(ItemPatch {
                name: Some(Some(name.into())),
                ..Default::default()
            }),
        }
    }

    /// Sets one key of the selected source's settings.
    fn set_setting(&mut self, key: &str, value: serde_json::Value) {
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(source) = self.current_source(&snapshot) else {
            return;
        };
        let Some(def) = snapshot.source(source) else {
            return;
        };
        let mut settings = if def.settings.is_object() {
            def.settings.clone()
        } else {
            serde_json::json!({})
        };
        if settings.get(key) == Some(&value) {
            return;
        }
        settings[key] = value;
        self.exec(Command::SetSourceSettings { source, settings });
    }

    pub fn inspect_color(&mut self, color: slint::Color) {
        self.set_setting(
            "color",
            serde_json::to_value(from_color(color)).unwrap_or_default(),
        );
    }

    pub fn inspect_option(&mut self, field: &str, index: i32) {
        let Some((_, item)) = self.selected() else {
            return;
        };
        let index = index.max(0) as usize;
        match field {
            "blend" => {
                let blend = BLEND_MODES.get(index).copied().unwrap_or_default();
                self.update_selected(ItemPatch {
                    blend: Some(blend),
                    ..Default::default()
                });
            }
            "size-mode" => {
                let mode = SIZE_MODES.get(index).copied().unwrap_or_default();
                let Some(g) = self.geoms.iter().find(|g| g.id == item.id).cloned() else {
                    return;
                };
                let mut t = item.transform;
                if mode == t.bounds_mode {
                    return;
                }
                if t.bounds_mode == BoundsMode::None {
                    // Keep the current on-screen box as the bounding box.
                    t.bounds = g.box_size;
                } else if mode == BoundsMode::None && g.content.x > 0.0 && g.content.y > 0.0 {
                    // Keep the content's current rendered size.
                    let rendered = t
                        .layout(
                            item.crop,
                            estream_core::Size::new(g.source_size.x, g.source_size.y),
                        )
                        .size;
                    t.scale = rendered / g.content;
                }
                t.bounds_mode = mode;
                self.update_selected(ItemPatch {
                    transform: Some(t),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    pub fn inspect_toggle(&mut self, field: &str, value: bool) {
        match field {
            "visible" => self.update_selected(ItemPatch {
                visible: Some(value),
                ..Default::default()
            }),
            "locked" => self.update_selected(ItemPatch {
                locked: Some(value),
                ..Default::default()
            }),
            _ => {}
        }
    }

    pub fn commit(&mut self) {
        if let Some(engine) = &self.engine {
            engine.seal_history();
        }
    }

    // ------------------------------------------------------------ preview

    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, px: f32, mods: Modifiers) {
        let p = Vec2::new(x, y);
        match kind {
            "down" => self.pointer_down(p, px),
            "move" => self.pointer_move(p, px, mods),
            "up" if self.drag.take().is_some() => {
                self.commit();
                self.guides_model.set_vec(Vec::new());
            }
            _ => {}
        }
    }

    fn pointer_down(&mut self, p: Vec2, px: f32) {
        self.selected_source = None;
        let Some((snapshot, _, Some(scene_id))) = self.canvas_and_scene() else {
            return;
        };
        let Some(scene) = snapshot.scene(scene_id) else {
            return;
        };
        let Some(canvas) = self.canvas_of(&snapshot) else {
            return;
        };
        let canvas_size = Vec2::new(canvas.width as f32, canvas.height as f32);

        let current = self
            .selection
            .and_then(|id| self.geoms.iter().find(|g| g.id == id));
        let handle = current.and_then(|g| interact::handle_at(g, p, px));
        let (target, kind) = match (current, handle) {
            (Some(g), Some(Handle::Resize { fx, fy })) => (g.id, DragKind::Resize { fx, fy }),
            (Some(g), Some(Handle::Rotate)) => (g.id, DragKind::Rotate),
            _ => match interact::hit_test(&self.geoms, p, self.selection) {
                Some(id) => (id, DragKind::Move),
                None => {
                    self.selection = None;
                    self.refresh();
                    return;
                }
            },
        };
        self.selection = Some(target);
        let Some(item) = scene.find_item(target) else {
            return;
        };
        let Some(geom) = self.geoms.iter().find(|g| g.id == target) else {
            return;
        };
        if !item.locked {
            let others = self
                .geoms
                .iter()
                .filter(|g| g.visible && !geom.path.contains(&g.id) && !g.path.contains(&geom.id))
                .map(|g| g.aabb)
                .collect::<Vec<_>>();
            self.drag = Some(Drag::new(
                kind,
                geom,
                item.transform,
                item.crop,
                p,
                canvas_size,
                others.into_iter(),
            ));
        }
        self.refresh();
    }

    fn pointer_move(&mut self, p: Vec2, px: f32, mods: Modifiers) {
        let Some(w) = self.window() else { return };
        if let Some(drag) = &self.drag {
            let result = drag.update(p, mods, px, self.settings.snapping);
            let item_id = drag.item;
            self.guides_model.set_vec(
                result
                    .guides
                    .iter()
                    .map(|g| Guide {
                        vertical: g.vertical,
                        pos: g.pos,
                    })
                    .collect::<Vec<_>>(),
            );
            if let Some((scene, item)) = self.selected().filter(|(_, i)| i.id == item_id) {
                let mut patch = ItemPatch::default();
                if result.transform != item.transform {
                    patch.transform = Some(result.transform);
                }
                if result.crop != item.crop {
                    patch.crop = Some(result.crop);
                }
                if patch != ItemPatch::default() {
                    self.exec(Command::UpdateItem {
                        scene,
                        item: item_id,
                        patch,
                    });
                }
            }
            return;
        }

        // Hover feedback.
        let current = self
            .selection
            .and_then(|id| self.geoms.iter().find(|g| g.id == id));
        let cursor = match current.and_then(|g| interact::handle_at(g, p, px)) {
            Some(Handle::Rotate) => "rotate",
            Some(Handle::Resize { fx, fy }) => match (fx, fy) {
                (0, 0) | (2, 2) => "nwse",
                (2, 0) | (0, 2) => "nesw",
                (1, _) => "ns",
                _ => "ew",
            },
            None => match interact::hit_test(&self.geoms, p, self.selection) {
                Some(id) if Some(id) == self.selection => "move",
                Some(_) => "pointer",
                None => "default",
            },
        };
        w.set_cursor(cursor.into());
        self.hover = interact::hit_test(&self.geoms, p, self.selection);
        w.set_hover_box(self.hover_box());
    }

    fn nudge(&mut self, delta: Vec2) {
        if let Some((scene, item)) = self.selected() {
            if item.locked {
                return;
            }
            let mut t = item.transform;
            t.position += delta;
            self.exec(Command::UpdateItem {
                scene,
                item: item.id,
                patch: ItemPatch {
                    transform: Some(t),
                    ..Default::default()
                },
            });
        }
    }

    // ------------------------------------------------------------ keys & actions

    /// Handles a key press that no focused widget consumed. Returns true if used.
    pub fn key(&mut self, text: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        let Some(key) = key_from_text(text) else {
            return false;
        };
        if !ctrl && !alt && self.selection.is_some() {
            let step = if shift { 10.0 } else { 1.0 };
            let delta = match key {
                Key::Up => Some(Vec2::new(0.0, -step)),
                Key::Down => Some(Vec2::new(0.0, step)),
                Key::Left => Some(Vec2::new(-step, 0.0)),
                Key::Right => Some(Vec2::new(step, 0.0)),
                _ => None,
            };
            if let Some(d) = delta {
                self.nudge(d);
                return true;
            }
        }
        match actions::for_key(key, ctrl, shift, alt) {
            Some(action) => {
                self.run(action);
                true
            }
            None => false,
        }
    }

    pub fn run_id(&mut self, id: &str) {
        if let Some(scene) = id.strip_prefix("scene:") {
            self.scene_activate(scene);
        } else if let Some(item) = id.strip_prefix("item:") {
            self.layer_select(item);
        } else if let Some(action) = actions::by_id(id) {
            self.run(action);
        }
    }

    pub fn run(&mut self, action: Action) {
        let Some(w) = self.window() else { return };
        if self.engine.is_none()
            && !matches!(
                action,
                Action::ThemeDark | Action::ThemeLight | Action::ThemeSystem
            )
        {
            return;
        }
        let canvas_size = self
            .canvas_and_scene()
            .and_then(|(s, _, _)| {
                s.canvases
                    .first()
                    .map(|c| Vec2::new(c.width as f32, c.height as f32))
            })
            .unwrap_or(Vec2::new(1920.0, 1080.0));
        match action {
            Action::CommandPalette => {
                w.set_palette_query("".into());
                w.set_palette_current(0);
                self.palette_filter("");
                w.set_palette_open(true);
            }
            Action::Undo => {
                if let Some(engine) = &self.engine {
                    match engine.undo() {
                        Ok(true) => self.refresh(),
                        Ok(false) => self.toast("Nothing to undo".into(), false),
                        Err(e) => self.toast(e.to_string(), true),
                    }
                }
            }
            Action::Redo => {
                if let Some(engine) = &self.engine {
                    match engine.redo() {
                        Ok(true) => self.refresh(),
                        Ok(false) => self.toast("Nothing to redo".into(), false),
                        Err(e) => self.toast(e.to_string(), true),
                    }
                }
            }
            Action::Save => {
                if let Some(engine) = &self.engine {
                    match engine.save() {
                        Ok(path) => {
                            self.dirty = false;
                            self.refresh();
                            self.toast(
                                format!(
                                    "Saved {}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                false,
                            );
                        }
                        Err(e) => self.toast(format!("Save failed: {e}"), true),
                    }
                }
            }
            Action::AddScene => self.scene_add(),
            Action::DuplicateScene | Action::RemoveScene | Action::RenameScene => {
                let Some((_, _, Some(scene))) = self.canvas_and_scene() else {
                    return;
                };
                let id = scene.to_string();
                match action {
                    Action::DuplicateScene => self.scene_duplicate(&id),
                    Action::RemoveScene => self.scene_remove(&id),
                    _ => w.set_renaming_scene(id.into()),
                }
            }
            Action::AddSource => w.set_add_source_open(true),
            Action::AddColorSource => self.add_source("color"),
            Action::AddTestPattern => self.add_source("test_pattern"),
            Action::RemoveSelected => self.remove_selected(),
            Action::DuplicateSelected => self.duplicate_selected(),
            Action::CopySelected => {
                if let Some((_, item)) = self.selected() {
                    self.clipboard = Some(item);
                    self.toast("Copied".into(), false);
                }
            }
            Action::PasteItem => self.paste(),
            Action::ToggleVisibility | Action::ToggleLock => {
                if let Some(id) = self.selection {
                    self.layer_toggle(&id.to_string(), action == Action::ToggleLock);
                }
            }
            Action::MoveUp | Action::MoveDown => self.move_selected(action == Action::MoveUp),
            Action::GroupSelected => self.group_selected(),
            Action::FitToCanvas
            | Action::StretchToCanvas
            | Action::CenterOnCanvas
            | Action::ResetTransform => {
                let Some((_, item)) = self.selected() else {
                    self.toast("Select a source first".into(), false);
                    return;
                };
                let transform = match action {
                    Action::FitToCanvas => interact::fit_to_canvas(canvas_size),
                    Action::StretchToCanvas => interact::stretch_to_canvas(canvas_size),
                    Action::CenterOnCanvas => {
                        let Some(g) = self.geoms.iter().find(|g| g.id == item.id) else {
                            return;
                        };
                        interact::centered(g, item.transform, canvas_size)
                    }
                    _ => estream_core::ItemTransform::default(),
                };
                self.update_selected(ItemPatch {
                    transform: Some(transform),
                    ..Default::default()
                });
            }
            Action::ZoomFit => w.invoke_zoom_fit(),
            Action::Zoom100 => w.invoke_zoom_actual(),
            Action::ToggleSnapping => {
                self.settings.snapping = !self.settings.snapping;
                let state = if self.settings.snapping { "on" } else { "off" };
                self.toast(format!("Snapping {state}"), false);
            }
            Action::ThemeDark => self.set_theme(ThemeMode::Dark),
            Action::ThemeLight => self.set_theme(ThemeMode::Light),
            Action::ThemeSystem => self.set_theme(ThemeMode::System),
            Action::ResetLayout => {
                self.apply_layout(PanelLayout::default());
                w.invoke_zoom_fit();
            }
            Action::Deselect => {
                self.selection = None;
                self.refresh();
            }
            Action::ToggleRecording => self.toggle_recording(),
            Action::ToggleReplay => self.toggle_replay(),
            Action::ToggleStream => self.toggle_stream(),
            Action::ToggleStudio => self.toggle_studio(),
            Action::TransitionGo => self.studio_go("go"),
            Action::TransitionCut => self.studio_go("cut"),
            Action::TransitionShader => self.open_shader_editor(),
            Action::SaveReplay => self.save_replay(),
            Action::RevealOutput => self.reveal_output(),
            Action::OpenSettings => {
                self.refresh_settings_form();
                w.set_settings_open(true);
            }
        }
    }

    fn remove_selected(&mut self) {
        let Some((scene, item)) = self.selected() else {
            return;
        };
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        // Removing the last use of a source removes the source too (undoable as one step).
        let command = match item.source_id() {
            Some(source) if snapshot.items_using(source).len() == 1 => {
                Command::RemoveSource { source }
            }
            _ => Command::RemoveItem {
                scene,
                item: item.id,
            },
        };
        if self.exec(command) {
            self.selection = None;
            self.refresh();
            self.toast("Deleted — Ctrl+Z to undo".into(), false);
        }
    }

    fn duplicate_selected(&mut self) {
        let Some((scene, item)) = self.selected() else {
            return;
        };
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(s) = snapshot.scene(scene) else {
            return;
        };
        let Some((parent, index)) = s.locate_item(item.id) else {
            return;
        };
        let mut commands = Vec::new();
        let mut copy = fresh_ids(&item);
        copy.transform.position += Vec2::splat(24.0);
        // Source items get their own independent copy of the source.
        if let Some(source) = item.source_id().and_then(|id| snapshot.source(id)) {
            let mut def = source.clone();
            def.id = SourceId::new();
            def.name = unique_name(
                &source.name,
                snapshot.sources.iter().map(|s| s.name.as_str()),
            );
            copy.content = ItemContent::Source { source: def.id };
            commands.push(Command::AddSource {
                source: def,
                index: usize::MAX,
            });
        }
        let id = copy.id;
        commands.push(Command::AddItem {
            scene,
            parent,
            index: index + 1,
            item: copy,
        });
        if self.exec(Command::Batch { commands }) {
            self.selection = Some(id);
            self.refresh();
        }
    }

    fn paste(&mut self) {
        let Some(item) = self.clipboard.clone() else {
            return;
        };
        let Some((snapshot, _, Some(scene))) = self.canvas_and_scene() else {
            return;
        };
        let mut missing = false;
        fn check(item: &SceneItem, snapshot: &Project, missing: &mut bool) {
            match &item.content {
                ItemContent::Source { source } => *missing |= snapshot.source(*source).is_none(),
                ItemContent::Group { items } => {
                    items.iter().for_each(|i| check(i, snapshot, missing))
                }
            }
        }
        check(&item, &snapshot, &mut missing);
        if missing {
            self.toast("The copied source no longer exists".into(), true);
            return;
        }
        let mut copy = fresh_ids(&item);
        copy.transform.position += Vec2::splat(24.0);
        let id = copy.id;
        if self.exec(Command::AddItem {
            scene,
            parent: None,
            index: usize::MAX,
            item: copy,
        }) {
            self.selection = Some(id);
            self.clipboard = Some(SceneItem {
                transform: estream_core::ItemTransform {
                    position: item.transform.position + Vec2::splat(24.0),
                    ..item.transform
                },
                ..item
            });
            self.refresh();
        }
    }

    fn move_selected(&mut self, up: bool) {
        let Some((scene, item)) = self.selected() else {
            return;
        };
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(s) = snapshot.scene(scene) else {
            return;
        };
        let Some((parent, index)) = s.locate_item(item.id) else {
            return;
        };
        let siblings = match parent {
            None => s.items.len(),
            Some(p) => match s.find_item(p).map(|i| &i.content) {
                Some(ItemContent::Group { items }) => items.len(),
                _ => return,
            },
        };
        // Higher index = drawn on top = "up" in the list.
        let target = if up {
            if index + 1 >= siblings {
                return;
            }
            index + 1
        } else {
            if index == 0 {
                return;
            }
            index - 1
        };
        self.exec(Command::MoveItem {
            scene,
            item: item.id,
            parent,
            index: target,
        });
    }

    fn group_selected(&mut self) {
        let Some((scene, item)) = self.selected() else {
            return;
        };
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
            return;
        };
        let Some(s) = snapshot.scene(scene) else {
            return;
        };
        let Some((parent, index)) = s.locate_item(item.id) else {
            return;
        };
        let name = unique_name(
            "Group",
            {
                let mut names = Vec::new();
                s.walk(&mut |i| {
                    if let Some(n) = &i.name {
                        names.push(n.clone())
                    }
                });
                names
            }
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .into_iter(),
        );
        let group = SceneItem::group(name, vec![item.clone()]);
        let group_id = group.id;
        if self.exec(Command::Batch {
            commands: vec![
                Command::RemoveItem {
                    scene,
                    item: item.id,
                },
                Command::AddItem {
                    scene,
                    parent,
                    index,
                    item: group,
                },
            ],
        }) {
            self.selection = Some(group_id);
            self.refresh();
        }
    }

    // ------------------------------------------------------------ palette

    pub fn palette_filter(&mut self, query: &str) {
        let mut candidates: Vec<(String, String, String, String)> = actions::ACTIONS
            .iter()
            .filter(|a| a.in_palette)
            .map(|a| {
                (
                    a.id.to_owned(),
                    a.title.to_owned(),
                    a.category.to_owned(),
                    a.shortcut.map(|s| s.label()).unwrap_or_default(),
                )
            })
            .collect();
        if let Some((snapshot, canvas, active)) = self.canvas_and_scene() {
            for scene in snapshot.scenes_of(canvas) {
                if Some(scene.id) != active {
                    candidates.push((
                        format!("scene:{}", scene.id),
                        format!("Go to scene: {}", scene.name),
                        "Scenes".into(),
                        String::new(),
                    ));
                }
            }
            if let Some(scene) = active.and_then(|id| snapshot.scene(id)) {
                scene.walk(&mut |item| {
                    let name = item
                        .name
                        .clone()
                        .or_else(|| {
                            item.source_id()
                                .and_then(|s| snapshot.source(s))
                                .map(|s| s.name.clone())
                        })
                        .unwrap_or_else(|| "Group".into());
                    candidates.push((
                        format!("item:{}", item.id),
                        format!("Select: {name}"),
                        "Sources".into(),
                        String::new(),
                    ));
                });
            }
        }
        let mut scored: Vec<_> = candidates
            .into_iter()
            .enumerate()
            .filter_map(|(order, c)| {
                actions::fuzzy_score(query, &format!("{} {}", c.1, c.2)).map(|s| (s, order, c))
            })
            .collect();
        if !query.trim().is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        let entries: Vec<PaletteEntry> = scored
            .into_iter()
            .take(12)
            .map(|(_, _, (id, title, category, hint))| PaletteEntry {
                id: id.into(),
                title: title.into(),
                category: category.into(),
                hint: hint.into(),
            })
            .collect();
        self.palette_model.set_vec(entries);
    }
}

fn push_layers(
    out: &mut Vec<LayerRow>,
    items: &[SceneItem],
    depth: i32,
    snapshot: &Project,
    infos: &HashMap<SourceId, SourceInfo>,
    selection: Option<ItemId>,
) {
    // Topmost first, like a layer stack.
    for item in items.iter().rev() {
        let source = item.source_id().and_then(|id| snapshot.source(id));
        out.push(LayerRow {
            id: id_str(item.id),
            name: item
                .name
                .clone()
                .or_else(|| source.map(|s| s.name.clone()))
                .unwrap_or_else(|| "Group".into())
                .into(),
            kind: source.map_or("group", |s| s.kind.as_str()).into(),
            visible: item.visible,
            locked: item.locked,
            depth,
            group: item.is_group(),
            selected: Some(item.id) == selection,
            problem: status_problem(item.source_id().and_then(|id| infos.get(&id))).into(),
        });
        if let ItemContent::Group { items } = &item.content {
            push_layers(out, items, depth + 1, snapshot, infos, selection);
        }
    }
}

/// Meter position (0..1) for a linear level on a -60..0 dB scale.
fn meter_position(level: f32) -> f32 {
    let db = estream_engine::audio_engine::db::from_gain(level);
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Fader position (0..1) to dB: linear in dB from -60 to 0; the bottom is silence.
fn fader_to_db(position: f32) -> f32 {
    if position <= 0.005 {
        -100.0
    } else {
        60.0 * (position.clamp(0.0, 1.0) - 1.0)
    }
}

fn db_to_fader(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        (db / 60.0 + 1.0).clamp(0.0, 1.0)
    }
}

/// Builds a property row for the inspector from a schema property and its current value.
fn prop_row(p: &estream_engine::Property, value: &serde_json::Value) -> PropRow {
    let mut row = PropRow {
        key: p.key.clone().into(),
        label: p.label.clone().into(),
        hint: p.hint.clone().into(),
        ..Default::default()
    };
    match &p.kind {
        PropKind::Bool => {
            row.kind = "bool".into();
            row.flag = value.as_bool().unwrap_or_default();
        }
        PropKind::Int { min, max, step } => {
            row.kind = "int".into();
            row.number = value.as_f64().unwrap_or_default() as f32;
            (row.minimum, row.maximum, row.step) = (*min as f32, *max as f32, *step as f32);
        }
        PropKind::Float { min, max, step } => {
            row.kind = "float".into();
            row.number = value.as_f64().unwrap_or_default() as f32;
            (row.minimum, row.maximum, row.step) = (*min as f32, *max as f32, *step as f32);
            row.decimals = if *step >= 1.0 { 0 } else { 2 };
        }
        PropKind::Text { multiline } => {
            row.kind = if *multiline { "multiline" } else { "text" }.into();
            row.text = value.as_str().unwrap_or_default().into();
        }
        PropKind::Font => {
            row.kind = "text".into();
            row.text = value.as_str().unwrap_or_default().into();
        }
        PropKind::Path { .. } => {
            row.kind = "path".into();
            row.text = value.as_str().unwrap_or_default().into();
        }
        PropKind::Color => {
            row.kind = "color".into();
            let c = serde_json::from_value::<Rgba8>(value.clone()).unwrap_or(Rgba8::WHITE);
            row.color = to_color(c);
            row.text = hex(c).into();
        }
        PropKind::Select { options } => {
            row.kind = "select".into();
            row.options = ModelRc::new(VecModel::from(
                options
                    .iter()
                    .map(|o| SharedString::from(o.label.as_str()))
                    .collect::<Vec<_>>(),
            ));
            row.index = options
                .iter()
                .position(|o| o.value == *value)
                .map_or(-1, |i| i as i32);
        }
    }
    row
}

/// Source types for the "Add source" dialog: everything registered, plus upcoming types.
fn source_types(registry: &estream_engine::SourceRegistry) -> Vec<SourceType> {
    let order = [
        "display_capture",
        "window_capture",
        "game_capture",
        "camera",
        "image",
        "text",
        "color",
        "test_pattern",
    ];
    let mut types: Vec<SourceType> = order
        .iter()
        .filter_map(|k| registry.get(k))
        .chain(
            registry
                .types()
                .iter()
                .filter(|t| !order.contains(&t.kind.as_str())),
        )
        .map(|t| SourceType {
            kind: t.kind.clone().into(),
            title: t.name.clone().into(),
            description: t.description.clone().into(),
            available: true,
        })
        .collect();
    for (kind, title, description) in [
        ("media", "Media file", "Video and audio files and playlists"),
        ("browser", "Browser", "Web pages, alerts and chat widgets"),
    ] {
        if registry.get(kind).is_none() {
            types.push(SourceType {
                kind: kind.into(),
                title: title.into(),
                description: description.into(),
                available: false,
            });
        }
    }
    types
}
/// Wires every UI callback to the controller and starts the UI timers.
pub fn install(app: &Shared, w: &AppWindow) -> Vec<slint::Timer> {
    // Runs `f` with the controller; defers if the controller is already borrowed
    // (e.g. a callback fired while another one is still running).
    fn with(app: &Shared, f: impl FnOnce(&mut App) + 'static) {
        match app.try_borrow_mut() {
            Ok(mut a) => f(&mut a),
            Err(_) => {
                let app = app.clone();
                slint::Timer::single_shot(Duration::ZERO, move || with(&app, f));
            }
        }
    }
    macro_rules! on {
        ($setter:ident, |$a:ident $(, $arg:ident)*| $body:expr) => {{
            let app = app.clone();
            w.$setter(move |$($arg),*| {
                $(let $arg = $arg.clone();)*
                with(&app, move |$a| { $body; });
            });
        }};
    }

    {
        let app = app.clone();
        w.on_key(move |text, ctrl, shift, alt| match app.try_borrow_mut() {
            Ok(mut a) => a.key(&text, ctrl, shift, alt),
            Err(_) => false,
        });
    }
    on!(on_preview_pointer, |a, kind, x, y, px, shift, alt, ctrl| a
        .pointer(&kind, x, y, px, Modifiers { shift, alt, ctrl }));
    on!(on_scene_activate, |a, id| a.scene_activate(&id));
    on!(on_scene_add, |a| a.scene_add());
    on!(on_scene_duplicate, |a, id| a.scene_duplicate(&id));
    on!(on_scene_remove, |a, id| a.scene_remove(&id));
    on!(on_scene_rename, |a, id, name| a.scene_rename(&id, &name));
    on!(on_layer_select, |a, id| a.layer_select(&id));
    on!(on_layer_toggle_visible, |a, id| a.layer_toggle(&id, false));
    on!(on_layer_toggle_lock, |a, id| a.layer_toggle(&id, true));
    on!(on_action, |a, id| a.run_id(&id));
    on!(on_inspect_edit, |a, field, v| a.inspect_edit(&field, v));
    on!(on_inspect_text, |a, field, v| a.inspect_text(&field, &v));
    on!(on_inspect_color, |a, c| a.inspect_color(c));
    on!(on_inspect_option, |a, field, i| a.inspect_option(&field, i));
    on!(on_inspect_toggle, |a, field, v| a.inspect_toggle(&field, v));
    on!(on_inspect_commit, |a| a.commit());
    on!(on_palette_query_changed, |a, q| a.palette_filter(&q));
    on!(on_palette_run, |a, id| a.run_id(&id));
    on!(on_add_source_pick, |a, kind| a.add_source(&kind));
    on!(on_wizard_create, |a, i| a.wizard_create(i));
    on!(on_recovery_restore, |a| a.recovery(true));
    on!(on_recovery_discard, |a| a.recovery(false));
    on!(on_toggle_theme, |a| a.toggle_theme());
    on!(on_prop_flag, |a, k, v| a.prop_flag(&k, v));
    on!(on_prop_number, |a, k, v| a.prop_number(&k, v));
    on!(on_prop_text, |a, k, v| a.prop_text(&k, &v));
    on!(on_prop_color, |a, k, v| a.prop_color(&k, v));
    on!(on_prop_index, |a, k, i| a.prop_index(&k, i));
    on!(on_prop_browse, |a, k| a.prop_browse(&k));
    on!(on_prop_refresh, |a| a.prop_refresh());
    on!(on_audio_set, |a, k, v| a.audio_set(&k, v));
    on!(on_filter_add, |a, i| a.filter_add(i));
    on!(on_filter_remove, |a, id| a.filter_remove(&id));
    on!(on_filter_toggle, |a, id, v| a.filter_toggle(&id, v));
    on!(on_filter_param, |a, id, k, v| a.filter_param(&id, &k, v));
    on!(on_filter_move, |a, id, up| a.filter_move(&id, up));
    on!(on_mixer_select, |a, id| a.mixer_select(&id));
    on!(on_mixer_fader, |a, id, v| a.mixer_fader(&id, v));
    on!(on_mixer_mute, |a, id| a.mixer_mute(&id));
    on!(on_settings_set, |a, k, v| a.settings_set(&k, v));
    on!(on_settings_text, |a, k, v| a.settings_text(&k, &v));
    on!(on_live_toggle, |a, i| a.live_toggle(i.max(0) as usize));
    on!(on_transition_set, |a, k, v| a.transition_set(&k, v));
    on!(on_studio_go, |a, m| a.studio_go(&m));
    on!(on_shader_apply, |a, c| a.shader_apply(&c));
    on!(on_shader_example, |a| a.shader_example());
    on!(on_vfilter_add, |a, i| a.vfilter_add(i));
    on!(on_vfilter_remove, |a, id| a.vfilter_remove(&id));
    on!(on_vfilter_toggle, |a, id, v| a.vfilter_toggle(&id, v));
    on!(on_vfilter_param, |a, id, k, v| a.vfilter_param(&id, &k, v));
    on!(on_vfilter_move, |a, id, up| a.vfilter_move(&id, up));
    on!(on_vfilter_browse, |a, id| a.vfilter_browse(&id));
    on!(on_vfilter_code, |a, id| a.vfilter_code(&id));
    on!(on_media_control, |a, c| a.media_control(&c));
    on!(on_media_seek, |a, f| a.media_seek(f));
    on!(on_browser_reload, |a| a.browser_reload());
    on!(on_canvas_select, |a, id| a.canvas_select(&id));
    on!(on_canvas_add, |a, i| a.canvas_add(i));
    on!(on_canvas_remove, |a, id| a.canvas_remove(&id));
    on!(on_canvas_rename, |a, id, name| a.canvas_rename(&id, &name));
    on!(on_settings_browse, |a| a.settings_browse());

    {
        let app = app.clone();
        w.window().on_close_requested(move || {
            if let Ok(mut a) = app.try_borrow_mut() {
                a.shutdown();
            }
            slint::CloseRequestResponse::HideWindow
        });
    }

    let tick = slint::Timer::default();
    {
        let app = app.clone();
        tick.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(4),
            move || {
                if let Ok(mut a) = app.try_borrow_mut() {
                    a.tick();
                }
            },
        );
    }
    vec![tick]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_unique() {
        let existing = ["Scene", "Scene 2"];
        assert_eq!(unique_name("Scene", existing.into_iter()), "Scene 3");
        assert_eq!(unique_name("Intro", existing.into_iter()), "Intro");
    }

    #[test]
    fn hex_colors() {
        assert_eq!(
            parse_hex_color("#ff8800"),
            Some(Rgba8::new(255, 136, 0, 255))
        );
        assert_eq!(
            parse_hex_color("ff880080"),
            Some(Rgba8::new(255, 136, 0, 128))
        );
        assert_eq!(parse_hex_color("#xyz"), None);
        assert_eq!(hex(Rgba8::new(255, 136, 0, 255)), "#ff8800");
    }

    #[test]
    fn fresh_ids_are_deep() {
        let src = SourceId::new();
        let group = SceneItem::group("G", vec![SceneItem::source(src)]);
        let copy = fresh_ids(&group);
        assert_ne!(copy.id, group.id);
        let (ItemContent::Group { items: a }, ItemContent::Group { items: b }) =
            (&group.content, &copy.content)
        else {
            panic!("groups expected");
        };
        assert_ne!(a[0].id, b[0].id);
        assert_eq!(a[0].source_id(), b[0].source_id(), "sources are shared");
    }

    #[test]
    fn control_characters_map_to_letters() {
        assert_eq!(key_from_text("\u{b}"), Some(Key::Char('k')));
        assert_eq!(key_from_text("K"), Some(Key::Char('k')));
        assert_eq!(key_from_text("ab"), None);
    }
}
