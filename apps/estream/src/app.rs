//! The studio controller: owns the engine, turns UI callbacks into engine
//! commands, and turns engine snapshots/events back into UI models.

use crate::actions::{self, Action, Key};
use crate::interact::{self, Drag, DragKind, Handle, ItemGeom, Modifiers};
use crate::presets::{self, PRESETS};
use crate::settings::{PanelLayout, Paths, Settings, ThemeMode, WindowState};
use crate::ui::{
    AppWindow, CanvasBox, Guide, Inspect, LayerRow, PaletteEntry, Preset, PropRow, SceneRow,
    SourceType, Theme,
};
use estream_core::{BlendMode, BoundsMode, CanvasId, ItemId, Rgba8, SceneId, SourceId};
use estream_engine::{
    Command, Engine, EngineConfig, Event, ItemPatch, PropKind, SourceInfo, SourceStatus,
    Subscription,
};
use estream_gpu::GpuContext;
use estream_project::{ItemContent, Project, Scene, SceneItem, SourceDef, persist};
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
        };
        window.set_scenes(ModelRc::from(app.scenes_model.clone()));
        window.set_layers(ModelRc::from(app.layers_model.clone()));
        window.set_guides(ModelRc::from(app.guides_model.clone()));
        window.set_palette_entries(ModelRc::from(app.palette_model.clone()));
        window.set_props(ModelRc::from(app.props_model.clone()));
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
            if let Some(canvas) = engine.snapshot().canvases.first().map(|c| c.id)
                && let Some(output) = engine.canvas_output(canvas)
                && let Some(frame) = output.latest()
                && frame.sequence != self.last_sequence
            {
                self.last_sequence = frame.sequence;
                match slint::Image::try_from(frame.preview.clone()) {
                    Ok(image) => w.set_preview_frame(image),
                    Err(e) => tracing::error!(error = %e, "preview texture rejected"),
                }
            }
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
        if self.stats_at.elapsed() >= Duration::from_millis(500) {
            self.update_stats();
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
        let Some(canvas) = snapshot.canvases.first() else {
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

    fn canvas_and_scene(&self) -> Option<(Arc<Project>, CanvasId, Option<SceneId>)> {
        let snapshot = self.engine.as_ref()?.snapshot();
        let canvas = snapshot.canvases.first()?;
        let (id, active) = (canvas.id, canvas.active_scene);
        Some((snapshot, id, active))
    }

    /// Rebuilds every UI model from the latest engine snapshot.
    pub fn refresh(&mut self) {
        let Some(w) = self.window() else { return };
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        let Some(canvas) = snapshot.canvases.first() else {
            return;
        };
        let scene = canvas.active_scene.and_then(|id| snapshot.scene(id));
        let infos = engine.source_infos();

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
                        active: Some(s.id) == canvas.active_scene,
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

        let (selection_box, inspect) = match (self.selection, scene) {
            (Some(sel), Some(scene)) => self.describe_selection(sel, scene, &snapshot, &infos),
            _ => (CanvasBox::default(), Inspect::default()),
        };
        w.set_selection_box(selection_box);
        w.set_inspect(inspect);
        self.refresh_props(&snapshot);
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
        self.exec(Command::SetActiveScene {
            canvas,
            scene: Some(SceneId(uuid)),
        });
    }

    pub fn scene_add(&mut self) {
        let Some((snapshot, canvas, _)) = self.canvas_and_scene() else {
            return;
        };
        let name = unique_name("Scene", snapshot.scenes.iter().map(|s| s.name.as_str()));
        let scene = Scene::new(name, canvas);
        let id = scene.id;
        if self.exec(Command::Batch {
            commands: vec![
                Command::AddScene {
                    scene,
                    index: usize::MAX,
                },
                Command::SetActiveScene {
                    canvas,
                    scene: Some(id),
                },
            ],
        }) {
            self.selection = None;
            if let Some(w) = self.window() {
                w.set_renaming_scene(id_str(id));
            }
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
        let new_id = copy.id;
        self.exec(Command::Batch {
            commands: vec![
                Command::AddScene { scene: copy, index },
                Command::SetActiveScene {
                    canvas,
                    scene: Some(new_id),
                },
            ],
        });
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
        let Some(canvas) = snapshot.canvases.first() else {
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

    // ------------------------------------------------------------ source properties

    /// Rebuilds the generic property rows when the selected source or its settings change.
    fn refresh_props(&mut self, snapshot: &Project) {
        let source = self
            .selection
            .and_then(|sel| {
                snapshot
                    .scenes
                    .iter()
                    .find_map(|s| s.find_item(sel))
                    .and_then(|i| i.source_id())
            })
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
        let Some((_, item)) = self.selected() else {
            return;
        };
        let name = value.trim();
        if field != "name" || name.is_empty() {
            return;
        }
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
        let Some((_, item)) = self.selected() else {
            return;
        };
        let Some(source) = item.source_id() else {
            return;
        };
        let Some((snapshot, _, _)) = self.canvas_and_scene() else {
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
        let Some((snapshot, _, Some(scene_id))) = self.canvas_and_scene() else {
            return;
        };
        let Some(scene) = snapshot.scene(scene_id) else {
            return;
        };
        let Some(canvas) = snapshot.canvases.first() else {
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
