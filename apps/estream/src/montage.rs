//! Montages in the studio (the `add-montage-editor` change): the saved montages, the New
//! montage wizard and the editor. Heavy work (scans, beat detection, assembly, thumbnails,
//! export) runs on its own threads; [`MontageUi::tick`] picks up the results.

use crate::ui::{BlockRow, ChipRow, MontageForm, MontageRow, PickRow, RecordingRow, TitleRow};
use estream_clips::library::{Library, Trigger};
use estream_gpu::GpuContext;
use estream_montage::assemble::{self, Item, MusicInput, Order};
use estream_montage::beats::{self, Beats};
use estream_montage::export::{ExportOptions, ExportPreset};
use estream_montage::model::{Anchor, Format, Montage, Music, Title, TitleStyle, Transition};
use estream_montage::preview::Preview;
use estream_montage::scan::Highlight;
use estream_montage::{History, MontageCommand};
use slint::{Image, ModelRc, SharedString, VecModel};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;

/// What the app should do after a montage action.
pub enum Effect {
    Toast(String, bool),
    /// An export landed in the clips folder: list it in the library.
    AddClip {
        path: PathBuf,
        title: String,
        duration: f64,
    },
}

/// Folders and shared things the montage pages need from the app.
pub struct Ctx<'a> {
    pub gpu: &'a GpuContext,
    pub montages: PathBuf,
    pub recordings: PathBuf,
    pub library: &'a Library,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Home,
    Wizard,
    Editor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    None,
    Segment(usize),
    Transition(usize),
    Title(u64),
    Music,
}

/// A background job's progress (0..=1000) and cancel flag.
#[derive(Default)]
struct Progress {
    permille: AtomicU32,
    cancel: AtomicBool,
}

impl Progress {
    fn get(&self) -> f32 {
        self.permille.load(Ordering::Relaxed) as f32 / 1000.0
    }
    fn set(&self, p: f32) -> bool {
        self.permille
            .store((p.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
        !self.cancel.load(Ordering::Acquire)
    }
}

struct Job<T> {
    progress: Arc<Progress>,
    done: Receiver<T>,
}

fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce(&Progress) -> T + Send + 'static,
) -> Option<Job<T>> {
    let progress = Arc::new(Progress::default());
    let (tx, done) = channel();
    let p = progress.clone();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _ = tx.send(work(&p));
        })
        .ok()
        .map(|_| Job { progress, done })
}

/// What music analysis found.
#[derive(Clone)]
struct MusicInfo {
    duration: f64,
    /// Peak level per 50 ms.
    peaks: Vec<f32>,
    beats: Option<Beats>,
}

fn analyze_music(path: &Path) -> Result<MusicInfo, String> {
    let stereo = estream_media::audio_file::decode_stereo(path).map_err(|e| e.to_string())?;
    let mono = estream_media::audio_file::to_mono(&stereo);
    let step = 2400; // 50 ms at 48 kHz
    let peaks = mono
        .chunks(step)
        .map(|c| c.iter().fold(0.0f32, |m, s| m.max(s.abs())))
        .collect();
    Ok(MusicInfo {
        duration: mono.len() as f64 / 48_000.0,
        peaks,
        beats: beats::detect(&mono, 48_000),
    })
}

struct Recording {
    path: PathBuf,
    name: String,
    detail: String,
    scan: Option<Job<Result<Vec<Highlight>, String>>>,
    highlights: Option<Vec<Highlight>>,
}

struct Editor {
    path: PathBuf,
    montage: Montage,
    history: History,
    preview: Preview,
    selection: Selection,
    /// Pixels per second on the timeline.
    zoom: f64,
    preview_sequence: u64,
    preview_image: Option<Image>,
    music: Option<MusicInfo>,
    music_job: Option<(PathBuf, Job<Result<MusicInfo, String>>)>,
    /// The segment as it was when a trim or slider drag started.
    drag_base: Option<(usize, estream_montage::Segment)>,
    export: Option<Job<Result<PathBuf, String>>>,
    export_adds_clip: bool,
}

pub struct MontageUi {
    page: Page,
    montages: Vec<(PathBuf, String, String)>,
    // Wizard
    step: usize,
    source_tab: usize,
    clips: Vec<PathBuf>,
    picked_clips: HashSet<PathBuf>,
    recordings: Vec<Recording>,
    /// (recording, highlight) in display order.
    highlights: Vec<(PathBuf, Highlight)>,
    picked_highlights: HashSet<(PathBuf, u64)>,
    music_path: Option<PathBuf>,
    music_info: Option<MusicInfo>,
    music_job: Option<Job<Result<MusicInfo, String>>>,
    target: f64,
    format: usize,
    order: usize,
    slow_motion: bool,
    title_text: String,
    create_job: Option<Job<Result<(PathBuf, Montage), String>>>,
    // Editor
    editor: Option<Editor>,
    export_preset: usize,
    add_to_clips: bool,
    error: String,
    images: HashMap<PathBuf, Option<Image>>,
    thumbs: HashMap<(PathBuf, i64), Option<Image>>,
    thumb_tx: std::sync::mpsc::Sender<((PathBuf, i64), PathBuf)>,
    thumb_rx: Receiver<((PathBuf, i64), PathBuf)>,
}

impl Default for MontageUi {
    fn default() -> Self {
        let (thumb_tx, thumb_rx) = channel();
        Self {
            page: Page::Home,
            montages: Vec::new(),
            step: 0,
            source_tab: 0,
            clips: Vec::new(),
            picked_clips: HashSet::new(),
            recordings: Vec::new(),
            highlights: Vec::new(),
            picked_highlights: HashSet::new(),
            music_path: None,
            music_info: None,
            music_job: None,
            target: 60.0,
            format: 0,
            order: 0,
            slow_motion: true,
            title_text: String::new(),
            create_job: None,
            editor: None,
            export_preset: 0,
            add_to_clips: true,
            error: String::new(),
            images: HashMap::new(),
            thumbs: HashMap::new(),
            thumb_tx,
            thumb_rx,
        }
    }
}

fn format_of(index: usize) -> Format {
    match index {
        1 => Format::VERTICAL,
        2 => Format::SQUARE,
        _ => Format::LANDSCAPE,
    }
}

fn index_of(format: Format) -> i32 {
    if format.width < format.height {
        1
    } else if format.width == format.height {
        2
    } else {
        0
    }
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0);
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s % 60.0)
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["mp4", "mkv", "mov"].contains(&e.to_ascii_lowercase().as_str()))
        && !path.to_string_lossy().contains(".part.")
}

/// A file name that doesn't exist yet in `dir`: `<name>.<ext>`, `<name> (2).<ext>`, …
fn unique(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let name = estream_clips::library::sanitize(name);
    let mut path = dir.join(format!("{name}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// Width ÷ height of a video, 16:9 if it can't be read.
fn aspect_of(path: &Path) -> f64 {
    estream_media::frames::FrameReader::open(path)
        .map(|r| r.width as f64 / r.height.max(1) as f64)
        .unwrap_or(16.0 / 9.0)
}

impl MontageUi {
    /// Opens the Montages page (refreshing the lists).
    pub fn open(&mut self, ctx: &Ctx<'_>) {
        if self.page != Page::Editor {
            self.page = Page::Home;
        }
        self.refresh_home(ctx);
    }

    /// Starts the wizard with these clips already picked (from the Clips view).
    pub fn new_with_clips(&mut self, ctx: &Ctx<'_>, clips: Vec<PathBuf>) {
        self.start_wizard(ctx);
        self.picked_clips = clips.into_iter().collect();
    }

    /// Closing the dialog: stop playback (an export keeps going).
    pub fn close(&mut self) {
        if let Some(e) = &self.editor {
            e.preview.pause();
        }
    }

    fn refresh_home(&mut self, ctx: &Ctx<'_>) {
        let mut list = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&ctx.montages) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str())
                    != Some(estream_montage::model::EXTENSION)
                {
                    continue;
                }
                let modified = entry.metadata().and_then(|m| m.modified()).ok();
                let (name, detail) = match Montage::load(&path) {
                    Ok(m) => {
                        let shape = match index_of(m.format) {
                            1 => "9:16",
                            2 => "1:1",
                            _ => "16:9",
                        };
                        let when = modified
                            .map(|t| crate::clips::format_when(t.into()))
                            .unwrap_or_default();
                        (
                            m.name.clone(),
                            format!(
                                "{} · {shape} · {} clips · {when}",
                                crate::clips::format_length(m.duration()),
                                m.segments.len()
                            ),
                        )
                    }
                    Err(e) => (
                        path.file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        format!("Can't open: {e}"),
                    ),
                };
                list.push((modified, path, name, detail));
            }
        }
        list.sort_by_key(|a| std::cmp::Reverse(a.0));
        self.montages = list.into_iter().map(|(_, p, n, d)| (p, n, d)).collect();
    }

    fn start_wizard(&mut self, ctx: &Ctx<'_>) {
        self.page = Page::Wizard;
        self.step = 0;
        self.source_tab = 0;
        self.picked_clips.clear();
        self.picked_highlights.clear();
        self.highlights.clear();
        self.clips = ctx.library.clips().iter().map(|c| c.path.clone()).collect();
        let mut recordings: Vec<(std::time::SystemTime, Recording)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&ctx.recordings) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || !is_video(&path) {
                    continue;
                }
                let meta = entry.metadata().ok();
                let modified = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::UNIX_EPOCH);
                let markers =
                    estream_clips::markers::Markers::load(&path).map_or(0, |m| m.markers.len());
                let size = meta.map_or(0, |m| m.len()) as f64 / 1e9;
                let mut detail = format!(
                    "{} · {size:.1} GB",
                    crate::clips::format_when(modified.into())
                );
                if markers > 0 {
                    detail.push_str(&format!(" · {markers} game events"));
                }
                recordings.push((
                    modified,
                    Recording {
                        name: path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        path,
                        detail,
                        scan: None,
                        highlights: None,
                    },
                ));
            }
        }
        recordings.sort_by_key(|a| std::cmp::Reverse(a.0));
        self.recordings = recordings.into_iter().map(|(_, r)| r).collect();
    }

    fn picked_count(&self) -> usize {
        self.picked_clips.len() + self.picked_highlights.len()
    }

    fn highlight_key(path: &Path, h: &Highlight) -> (PathBuf, u64) {
        (path.to_path_buf(), (h.start * 1000.0) as u64)
    }

    fn rebuild_highlights(&mut self) {
        self.highlights = self
            .recordings
            .iter()
            .flat_map(|r| {
                r.highlights
                    .iter()
                    .flatten()
                    .map(move |h| (r.path.clone(), h.clone()))
            })
            .collect();
    }

    fn start_music_analysis(path: PathBuf) -> Option<Job<Result<MusicInfo, String>>> {
        spawn("estream-music-analysis", move |_| analyze_music(&path))
    }

    /// The items to assemble from what's picked.
    fn items(&self, ctx: &Ctx<'_>) -> Vec<Item> {
        let mut items = Vec::new();
        for clip in ctx.library.clips() {
            if !self.picked_clips.contains(&clip.path) {
                continue;
            }
            let m = &clip.meta;
            let strongest = m
                .events
                .iter()
                .max_by(|a, b| a.kind.weight().total_cmp(&b.kind.weight()));
            items.push(Item {
                source: clip.path.clone(),
                start: 0.0,
                end: m.duration.max(0.5),
                anchor: strongest.map_or(m.duration * 0.6, |e| e.at),
                score: strongest.map_or(0.75, |e| e.kind.weight()),
                events: m.events.iter().map(|e| e.at).collect(),
                when: m.created.timestamp() as f64,
                group: m.game.clone().unwrap_or_else(|| "Clips".into()),
                aspect: 16.0 / 9.0,
            });
        }
        for (path, h) in &self.highlights {
            if !self
                .picked_highlights
                .contains(&Self::highlight_key(path, h))
            {
                continue;
            }
            let base = std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |d| d.as_secs_f64());
            items.push(Item {
                source: path.clone(),
                start: h.start,
                end: h.end,
                anchor: h.anchor,
                score: h.score,
                events: h.events.clone(),
                when: base + h.start,
                group: path.to_string_lossy().into_owned(),
                aspect: 16.0 / 9.0,
            });
        }
        items
    }

    fn create(&mut self, ctx: &Ctx<'_>) {
        let mut items = self.items(ctx);
        if items.is_empty() {
            return;
        }
        let options = assemble::Options {
            name: if self.title_text.trim().is_empty() {
                "Montage".into()
            } else {
                self.title_text.trim().to_owned()
            },
            target: self.target,
            format: format_of(self.format),
            order: if self.order == 1 {
                Order::BestFirst
            } else {
                Order::Chronological
            },
            music: None,
            slow_motion: self.slow_motion,
            title: Some(self.title_text.clone()).filter(|t| !t.trim().is_empty()),
        };
        let music = self.music_path.clone();
        let analysed = self.music_info.clone();
        let dir = ctx.montages.clone();
        self.create_job = spawn("estream-montage-create", move |_| {
            // Each source's real shape, for framing.
            let mut aspects: HashMap<PathBuf, f64> = HashMap::new();
            for item in &mut items {
                item.aspect = *aspects
                    .entry(item.source.clone())
                    .or_insert_with(|| aspect_of(&item.source));
            }
            let music_input = match &music {
                Some(path) => {
                    let info = match analysed {
                        Some(i) => i,
                        None => analyze_music(path)?,
                    };
                    info.beats.map(|beats| MusicInput {
                        source: path.clone(),
                        beats,
                        gain_db: -4.0,
                    })
                }
                None => None,
            };
            let mut montage = assemble::assemble(
                &items,
                &assemble::Options {
                    music: music_input,
                    ..options
                },
            );
            // Music without beats still plays.
            if montage.music.is_none()
                && let Some(path) = &music
            {
                montage.music = Some(Music {
                    source: path.clone(),
                    source_rel: None,
                    src_in: 0.0,
                    gain_db: -4.0,
                    fade_in: 0.0,
                    fade_out: 2.0,
                    beats: Vec::new(),
                    missing: false,
                });
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = unique(&dir, &montage.name, estream_montage::model::EXTENSION);
            montage.save(&path).map_err(|e| e.to_string())?;
            Ok((path, montage))
        });
    }

    fn open_editor(&mut self, ctx: &Ctx<'_>, path: PathBuf, montage: Montage) {
        let preview = Preview::start(ctx.gpu, Arc::new(montage.clone()));
        let music_job = montage.music.as_ref().filter(|m| !m.missing).and_then(|m| {
            Self::start_music_analysis(m.source.clone()).map(|j| (m.source.clone(), j))
        });
        let zoom = (900.0 / montage.duration().max(1.0)).clamp(8.0, 200.0);
        self.editor = Some(Editor {
            path,
            montage,
            history: History::default(),
            preview,
            selection: Selection::None,
            zoom,
            preview_sequence: 0,
            preview_image: None,
            music: None,
            music_job,
            drag_base: None,
            export: None,
            export_adds_clip: false,
        });
        self.page = Page::Editor;
    }

    /// Applies an edit: history, save, preview.
    fn edit(&mut self, command: MontageCommand) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        match e.history.apply(&mut e.montage, command, Instant::now()) {
            Ok(()) => Self::after_edit(e),
            Err(err) => vec![Effect::Toast(err.to_string(), true)],
        }
    }

    fn after_edit(e: &mut Editor) -> Vec<Effect> {
        e.preview.set_montage(Arc::new(e.montage.clone()));
        // Selection may point past the end after a removal.
        e.selection = match e.selection {
            Selection::Segment(i) if i >= e.montage.segments.len() => Selection::None,
            Selection::Transition(i) if i >= e.montage.transitions.len() => Selection::None,
            Selection::Title(id) if !e.montage.titles.iter().any(|t| t.id == id) => Selection::None,
            s => s,
        };
        match e.montage.save(&e.path) {
            Ok(()) => Vec::new(),
            Err(err) => vec![Effect::Toast(
                format!("Couldn't save the montage: {err}"),
                true,
            )],
        }
    }

    /// The selected segment, edited by `change` from the segment as it was when the gesture
    /// started (so a drag is one undo step and never drifts).
    fn edit_segment(
        &mut self,
        index: usize,
        change: impl FnOnce(&mut estream_montage::Segment),
    ) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        if e.drag_base.as_ref().is_none_or(|(i, _)| *i != index) {
            let Some(seg) = e.montage.segments.get(index) else {
                return Vec::new();
            };
            e.drag_base = Some((index, seg.clone()));
        }
        let Some((_, base)) = &e.drag_base else {
            return Vec::new();
        };
        let mut seg = base.clone();
        change(&mut seg);
        if Some(&seg) == e.montage.segments.get(index) {
            return Vec::new();
        }
        self.edit(MontageCommand::SetSegment {
            index,
            segment: seg,
        })
    }

    pub fn action(&mut self, ctx: &Ctx<'_>, field: &str, index: i32, value: f32) -> Vec<Effect> {
        let i = index.max(0) as usize;
        let v = value as f64;
        match field {
            // Home
            "home" => {
                if let Some(e) = &self.editor {
                    e.preview.pause();
                }
                if self.editor.as_ref().is_some_and(|e| e.export.is_some()) {
                    return vec![Effect::Toast(
                        "The export keeps going in the background".into(),
                        false,
                    )];
                }
                self.editor = None;
                self.page = Page::Home;
                self.refresh_home(ctx);
            }
            "new" => self.start_wizard(ctx),
            "open" => {
                if let Some((path, ..)) = self.montages.get(i).cloned() {
                    match Montage::load(&path) {
                        Ok(m) => {
                            let missing = m.segments.iter().filter(|s| s.missing).count();
                            self.open_editor(ctx, path, m);
                            if missing > 0 {
                                return vec![Effect::Toast(
                                    format!(
                                        "{missing} clips are missing: they show in red. Relink them by moving them back, or pick their folder from the clip's inspector."
                                    ),
                                    true,
                                )];
                            }
                        }
                        Err(e) => {
                            return vec![Effect::Toast(
                                format!("Couldn't open the montage: {e}"),
                                true,
                            )];
                        }
                    }
                }
            }
            "delete" => {
                if let Some((path, name, _)) = self.montages.get(i).cloned() {
                    let _ = std::fs::remove_file(&path);
                    self.refresh_home(ctx);
                    return vec![Effect::Toast(
                        format!("Deleted “{name}” (its clips are untouched)"),
                        false,
                    )];
                }
            }
            "open-folder" => {
                let _ = std::fs::create_dir_all(&ctx.montages);
                let _ = crate::clips::open_in_player(&ctx.montages);
            }
            // Wizard
            "tab" => self.source_tab = i.min(1),
            "pick-clip" => {
                if let Some(p) = self.clips.get(i).cloned()
                    && !self.picked_clips.remove(&p)
                {
                    self.picked_clips.insert(p);
                }
            }
            "pick-highlight" => {
                if let Some((p, h)) = self.highlights.get(i) {
                    let key = Self::highlight_key(p, h);
                    if !self.picked_highlights.remove(&key) {
                        self.picked_highlights.insert(key);
                    }
                }
            }
            "scan" => {
                if let Some(r) = self.recordings.get_mut(i) {
                    let path = r.path.clone();
                    r.scan = spawn("estream-scan", move |p| {
                        estream_montage::scan::scan(
                            &path,
                            estream_clips::Sensitivity::Medium,
                            &mut |x| p.set(x),
                        )
                        .map_err(|e| e.to_string())
                    });
                }
            }
            "step" => self.step = i.min(1),
            "pick-music" => {
                let picked = rfd::FileDialog::new()
                    .set_title("Music for the montage")
                    .add_filter(
                        "Audio",
                        &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac"],
                    )
                    .pick_file();
                if let Some(path) = picked {
                    if self.page == Page::Editor {
                        if let Some(e) = &mut self.editor {
                            e.music_job =
                                Self::start_music_analysis(path.clone()).map(|j| (path.clone(), j));
                            e.music = None;
                        }
                        let music = Music {
                            source: path,
                            source_rel: None,
                            src_in: 0.0,
                            gain_db: -4.0,
                            fade_in: 0.0,
                            fade_out: 2.0,
                            beats: Vec::new(),
                            missing: false,
                        };
                        if let Some(e) = &mut self.editor {
                            e.selection = Selection::Music;
                        }
                        return self.edit(MontageCommand::SetMusic { music: Some(music) });
                    }
                    self.music_job = Self::start_music_analysis(path.clone());
                    self.music_info = None;
                    self.music_path = Some(path);
                }
            }
            "clear-music" => {
                if self.page == Page::Editor {
                    if let Some(e) = &mut self.editor {
                        e.music = None;
                        e.music_job = None;
                        e.selection = Selection::None;
                    }
                    return self.edit(MontageCommand::SetMusic { music: None });
                }
                self.music_path = None;
                self.music_info = None;
                self.music_job = None;
            }
            "target" => self.target = v.clamp(10.0, 600.0),
            "format" => self.format = i.min(2),
            "order" => self.order = i.min(1),
            "slow-motion" => self.slow_motion = value > 0.5,
            "create" => self.create(ctx),
            _ => return self.editor_action(ctx, field, i, v),
        }
        Vec::new()
    }

    fn editor_action(&mut self, ctx: &Ctx<'_>, field: &str, i: usize, v: f64) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        let zoom = e.zoom;
        let position = e.preview.position();
        match field {
            "seek" => e.preview.seek(v),
            "play" => e.preview.toggle(),
            "undo" => {
                e.drag_base = None;
                return match e.history.undo(&mut e.montage) {
                    Ok(_) => Self::after_edit(e),
                    Err(err) => vec![Effect::Toast(err.to_string(), true)],
                };
            }
            "redo" => {
                e.drag_base = None;
                return match e.history.redo(&mut e.montage) {
                    Ok(_) => Self::after_edit(e),
                    Err(err) => vec![Effect::Toast(err.to_string(), true)],
                };
            }
            "zoom" => e.zoom = (e.zoom * if v > 0.0 { 1.25 } else { 0.8 }).clamp(4.0, 400.0),
            "seek-px" => e.preview.seek((v / zoom).max(0.0)),
            "select-block" => {
                e.selection = Selection::Segment(i);
                e.drag_base = None;
            }
            "drop-block" => {
                // The block's centre after the drag decides its new place.
                let center = v / zoom;
                let layout = e.montage.layout();
                let to = layout
                    .iter()
                    .filter(|p| p.index != i)
                    .filter(|p| (p.start + p.end) / 2.0 < center)
                    .count();
                if to != i {
                    e.selection = Selection::Segment(to);
                    return self.edit(MontageCommand::MoveSegment { from: i, to });
                }
            }
            "trim-in" | "trim-out" => {
                let delta = v / zoom;
                let out = field == "trim-out";
                return self.edit_segment(i, |s| {
                    let src_delta = delta * s.speed as f64;
                    if out {
                        s.src_out = (s.src_out + src_delta).max(s.src_in + 0.3);
                    } else {
                        s.src_in = (s.src_in + src_delta).clamp(0.0, s.src_out - 0.3);
                    }
                });
            }
            "seal" => {
                e.history.seal();
                e.drag_base = None;
            }
            "select-transition" => e.selection = Selection::Transition(i),
            "select-title" => {
                if let Some(t) = e.montage.titles.get(i) {
                    e.selection = Selection::Title(t.id);
                }
            }
            "select-music" => e.selection = Selection::Music,
            "set-format" => {
                let format = format_of(i);
                return self.edit(MontageCommand::SetFormat { format });
            }
            "fade-out" => return self.edit(MontageCommand::SetFadeOut { secs: v as f32 }),
            "add-title" => {
                let id = e.montage.new_id();
                e.selection = Selection::Title(id);
                return self.edit(MontageCommand::AddTitle {
                    title: Title {
                        id,
                        text: "Title".into(),
                        style: TitleStyle::Bold,
                        start: position,
                        dur: 2.0,
                        anchor: Anchor::Center,
                    },
                });
            }
            "speed" | "gain" | "pan" => {
                let Selection::Segment(index) = e.selection else {
                    return Vec::new();
                };
                let format = e.montage.format;
                return match field {
                    "speed" => self.edit_segment(index, |s| s.speed = (v as f32).clamp(0.25, 2.0)),
                    "gain" => {
                        self.edit_segment(index, |s| s.gain_db = (v as f32).clamp(-60.0, 12.0))
                    }
                    _ => {
                        let source = e.montage.segments.get(index).map(|s| s.source.clone());
                        let aspect = source.map_or(16.0 / 9.0, |p| aspect_of(&p));
                        self.edit_segment(index, |s| {
                            s.crop = estream_montage::Crop::fill(aspect, format.aspect(), v as f32);
                        })
                    }
                };
            }
            "split" => return self.split(),
            "delete-selected" => return self.delete_selected(),
            "trans-kind" | "trans-secs" => {
                let Selection::Transition(index) = e.selection else {
                    return Vec::new();
                };
                let current = e
                    .montage
                    .transitions
                    .get(index)
                    .copied()
                    .unwrap_or_default();
                let secs = if field == "trans-secs" {
                    v as f32
                } else {
                    current.secs().max(0.5) as f32
                };
                let kind = if field == "trans-kind" {
                    i
                } else {
                    match current {
                        Transition::Cut => 0,
                        Transition::Crossfade { .. } => 1,
                        Transition::DipToBlack { .. } => 2,
                        Transition::Whip { .. } => 3,
                    }
                };
                let transition = match kind {
                    1 => Transition::Crossfade { secs },
                    2 => Transition::DipToBlack { secs },
                    3 => Transition::Whip {
                        secs: secs.min(0.6),
                    },
                    _ => Transition::Cut,
                };
                return self.edit(MontageCommand::SetTransition { index, transition });
            }
            "title-style" | "title-anchor" | "title-start" | "title-dur" => {
                let Selection::Title(id) = e.selection else {
                    return Vec::new();
                };
                let Some(mut title) = e.montage.titles.iter().find(|t| t.id == id).cloned() else {
                    return Vec::new();
                };
                match field {
                    "title-style" => {
                        title.style = if i == 1 {
                            TitleStyle::Clean
                        } else {
                            TitleStyle::Bold
                        }
                    }
                    "title-anchor" => {
                        title.anchor = match i {
                            0 => Anchor::Top,
                            2 => Anchor::Bottom,
                            _ => Anchor::Center,
                        }
                    }
                    "title-start" => title.start = v.max(0.0),
                    _ => title.dur = v.max(0.3),
                }
                return self.edit(MontageCommand::SetTitle { title });
            }
            "music-gain" | "music-fade-in" | "music-fade-out" => {
                let Some(mut music) = e.montage.music.clone() else {
                    return Vec::new();
                };
                match field {
                    "music-gain" => music.gain_db = v as f32,
                    "music-fade-in" => music.fade_in = v as f32,
                    _ => music.fade_out = v as f32,
                }
                return self.edit(MontageCommand::SetMusic { music: Some(music) });
            }
            "duck" => {
                let mut mix = e.montage.mix;
                mix.duck_db = v as f32;
                return self.edit(MontageCommand::SetMix { mix });
            }
            "export-preset" => self.export_preset = i.min(2),
            "add-to-clips" => self.add_to_clips = v > 0.5,
            "export" => return self.start_export(ctx),
            "export-cancel" => {
                if let Some(job) = &e.export {
                    job.progress.cancel.store(true, Ordering::Release);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn split(&mut self) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        let t = e.preview.position();
        let Some(p) = e
            .montage
            .layout()
            .into_iter()
            .find(|p| t > p.start && t < p.end)
        else {
            return Vec::new();
        };
        match MontageCommand::split(&mut e.montage, p.index, t - p.start) {
            Ok(c) => {
                e.selection = Selection::Segment(p.index + 1);
                self.edit(c)
            }
            Err(err) => vec![Effect::Toast(err.to_string(), false)],
        }
    }

    fn delete_selected(&mut self) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        let command = match e.selection {
            Selection::Segment(index) => MontageCommand::RemoveSegment { index },
            Selection::Title(id) => MontageCommand::RemoveTitle { id },
            Selection::Transition(index) => MontageCommand::SetTransition {
                index,
                transition: Transition::Cut,
            },
            Selection::Music => MontageCommand::SetMusic { music: None },
            Selection::None => return Vec::new(),
        };
        e.selection = Selection::None;
        self.edit(command)
    }

    fn start_export(&mut self, ctx: &Ctx<'_>) -> Vec<Effect> {
        let Some(e) = &mut self.editor else {
            return Vec::new();
        };
        if e.export.is_some() {
            return Vec::new();
        }
        let montage = e.montage.clone();
        if montage.segments.iter().any(|s| s.missing) {
            return vec![Effect::Toast(
                "Some clips are missing: remove or relink them first".into(),
                true,
            )];
        }
        let dir = if self.add_to_clips {
            ctx.library.root().join("Montages")
        } else {
            ctx.montages.clone()
        };
        let _ = std::fs::create_dir_all(&dir);
        let path = unique(&dir, &montage.name, "mp4");
        let preset = match self.export_preset {
            1 => ExportPreset::Vertical,
            2 => ExportPreset::Source,
            _ => ExportPreset::YouTube,
        };
        let options = ExportOptions {
            preset,
            ..ExportOptions::new(&path)
        };
        let gpu = ctx.gpu.clone();
        e.preview.pause();
        e.export_adds_clip = self.add_to_clips;
        e.export = spawn("estream-montage-export", move |p| {
            estream_montage::export::export(&gpu, &montage, &options, &mut |x| p.set(x))
                .map_err(|e| e.to_string())
        });
        Vec::new()
    }

    pub fn text(&mut self, _ctx: &Ctx<'_>, field: &str, text: &str) -> Vec<Effect> {
        match field {
            "title-text" => self.title_text = text.to_owned(),
            "name" if !text.trim().is_empty() => {
                return self.edit(MontageCommand::Rename {
                    name: text.trim().to_owned(),
                });
            }
            "title-edit" => {
                let Some(e) = &self.editor else {
                    return Vec::new();
                };
                let Selection::Title(id) = e.selection else {
                    return Vec::new();
                };
                if let Some(mut title) = e.montage.titles.iter().find(|t| t.id == id).cloned() {
                    title.text = text.to_owned();
                    return self.edit(MontageCommand::SetTitle { title });
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Editor keyboard shortcuts.
    pub fn key(&mut self, ctx: &Ctx<'_>, text: &str, ctrl: bool, shift: bool) -> Vec<Effect> {
        use slint::platform::Key;
        let is = |k: Key| text == SharedString::from(k).as_str();
        let Some(e) = &self.editor else {
            return Vec::new();
        };
        let frame = e.montage.format.frame_secs();
        let position = e.preview.position();
        let lower = text.to_lowercase();
        match () {
            _ if ctrl && lower == "z" && shift => self.action(ctx, "redo", 0, 0.0),
            _ if ctrl && lower == "z" => self.action(ctx, "undo", 0, 0.0),
            _ if ctrl && lower == "y" => self.action(ctx, "redo", 0, 0.0),
            _ if text == " " || lower == "k" && !ctrl => {
                if text == " " {
                    self.action(ctx, "play", 0, 0.0)
                } else {
                    e.preview.pause();
                    Vec::new()
                }
            }
            _ if lower == "l" => {
                e.preview.play();
                Vec::new()
            }
            _ if lower == "j" => {
                e.preview.seek((position - 2.0).max(0.0));
                Vec::new()
            }
            _ if lower == "s" && !ctrl => self.split(),
            _ if is(Key::Delete) || is(Key::Backspace) => self.delete_selected(),
            _ if is(Key::LeftArrow) => {
                e.preview.pause();
                e.preview.seek((position - frame).max(0.0));
                Vec::new()
            }
            _ if is(Key::RightArrow) => {
                e.preview.pause();
                e.preview.seek(position + frame);
                Vec::new()
            }
            _ if is(Key::Home) => {
                e.preview.seek(0.0);
                Vec::new()
            }
            _ if text == "-" => self.action(ctx, "zoom", 0, -1.0),
            _ if text == "=" || text == "+" => self.action(ctx, "zoom", 0, 1.0),
            _ => Vec::new(),
        }
    }

    /// Picks up finished work. Returns whether the form should be refreshed.
    pub fn tick(&mut self, ctx: &Ctx<'_>) -> (bool, Vec<Effect>) {
        let mut changed = false;
        let mut effects = Vec::new();
        // Scans.
        let mut scanned = false;
        for r in &mut self.recordings {
            if let Some(job) = &r.scan {
                changed = true;
                match job.done.try_recv() {
                    Ok(Ok(h)) => {
                        let n = h.len();
                        r.highlights = Some(h);
                        r.scan = None;
                        r.detail = format!("{n} highlights found");
                        scanned = true;
                    }
                    Ok(Err(e)) => {
                        r.scan = None;
                        effects.push(Effect::Toast(format!("Scan failed: {e}"), true));
                    }
                    Err(TryRecvError::Disconnected) => r.scan = None,
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        if scanned {
            self.rebuild_highlights();
            // Pick the strongest highlights by default.
            for (p, h) in &self.highlights {
                if h.score >= 0.6 {
                    self.picked_highlights.insert(Self::highlight_key(p, h));
                }
            }
        }
        // Wizard music.
        if let Some(job) = &self.music_job
            && let Ok(result) = job.done.try_recv()
        {
            self.music_job = None;
            changed = true;
            match result {
                Ok(info) => self.music_info = Some(info),
                Err(e) => {
                    effects.push(Effect::Toast(format!("Couldn't read the music: {e}"), true))
                }
            }
        }
        // Creating.
        if let Some(job) = &self.create_job
            && let Ok(result) = job.done.try_recv()
        {
            self.create_job = None;
            changed = true;
            match result {
                Ok((path, m)) => self.open_editor(ctx, path, m),
                Err(e) => effects.push(Effect::Toast(
                    format!("Couldn't make the montage: {e}"),
                    true,
                )),
            }
        }
        // Thumbnails.
        while let Ok((key, jpg)) = self.thumb_rx.try_recv() {
            let image = Image::load_from_path(&jpg).ok();
            let _ = std::fs::remove_file(&jpg);
            self.thumbs.insert(key, image);
            changed = true;
        }
        // Editor: preview frames, music analysis, export.
        if let Some(e) = &mut self.editor {
            if let Some(frame) = e.preview.latest()
                && frame.sequence != e.preview_sequence
            {
                e.preview_sequence = frame.sequence;
                e.preview_image = Image::try_from(frame.texture).ok();
                changed = true;
            }
            if e.preview.is_playing() {
                changed = true;
            }
            if let Some((path, job)) = &e.music_job
                && let Ok(result) = job.done.try_recv()
            {
                let path = path.clone();
                e.music_job = None;
                changed = true;
                match result {
                    Ok(info) => {
                        // Store the beats in the montage once (for snapping later).
                        let beats = info
                            .beats
                            .as_ref()
                            .map(|b| b.beats.clone())
                            .unwrap_or_default();
                        e.music = Some(info);
                        if let Some(mut music) = e.montage.music.clone()
                            && music.source == path
                            && music.beats.is_empty()
                            && !beats.is_empty()
                        {
                            music.beats = beats;
                            let _ = MontageCommand::SetMusic { music: Some(music) }
                                .apply(&mut e.montage);
                            effects.extend(Self::after_edit(e));
                        }
                    }
                    Err(err) => effects.push(Effect::Toast(
                        format!("Couldn't read the music: {err}"),
                        true,
                    )),
                }
            }
            if let Some(job) = &e.export {
                changed = true;
                match job.done.try_recv() {
                    Ok(Ok(path)) => {
                        e.export = None;
                        let duration = e.montage.duration();
                        effects.push(Effect::Toast(format!("Exported {}", path.display()), false));
                        if e.export_adds_clip {
                            effects.push(Effect::AddClip {
                                path,
                                title: e.montage.name.clone(),
                                duration,
                            });
                        }
                    }
                    Ok(Err(err)) => {
                        e.export = None;
                        if !err.contains("cancelled") {
                            effects.push(Effect::Toast(format!("Export failed: {err}"), true));
                        }
                    }
                    Err(TryRecvError::Disconnected) => e.export = None,
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        (changed || self.create_job.is_some(), effects)
    }

    fn thumb(&mut self, path: &Path, at: f64) -> Option<Image> {
        let key = (path.to_path_buf(), (at * 10.0) as i64);
        if let Some(t) = self.thumbs.get(&key) {
            return t.clone();
        }
        self.thumbs.insert(key.clone(), None);
        let tx = self.thumb_tx.clone();
        let (src, n) = (path.to_path_buf(), self.thumbs.len());
        let _ = std::thread::Builder::new()
            .name("estream-montage-thumb".into())
            .spawn(move || {
                let jpg = std::env::temp_dir()
                    .join(format!("estream-mthumb-{}-{n}.jpg", std::process::id()));
                if estream_media::thumbnail::thumbnail(&src, at, 160, &jpg).is_ok() {
                    let _ = tx.send((key, jpg));
                }
            });
        None
    }

    fn image(&mut self, path: &Path) -> Option<Image> {
        self.images
            .entry(path.to_path_buf())
            .or_insert_with(|| Image::load_from_path(path).ok())
            .clone()
    }

    pub fn form(&mut self, ctx: &Ctx<'_>) -> MontageForm {
        let strings = |v: Vec<MontageRow>| ModelRc::new(VecModel::from(v));
        let mut form = MontageForm {
            page: match self.page {
                Page::Home => "home",
                Page::Wizard => "wizard",
                Page::Editor => "editor",
            }
            .into(),
            montages: strings(
                self.montages
                    .iter()
                    .map(|(_, n, d)| MontageRow {
                        name: n.into(),
                        detail: d.into(),
                    })
                    .collect(),
            ),
            step: self.step as i32,
            source_tab: self.source_tab as i32,
            picked_count: self.picked_count() as i32,
            target: self.target as f32,
            format: self.format as i32,
            order: self.order as i32,
            slow_motion: self.slow_motion,
            title_text: self.title_text.clone().into(),
            export_preset: self.export_preset as i32,
            export_progress: -1.0,
            add_to_clips: self.add_to_clips,
            error: self.error.clone().into(),
            ..MontageForm::default()
        };
        if self.page == Page::Wizard {
            let clips: Vec<PickRow> = self
                .clips
                .clone()
                .iter()
                .filter_map(|p| ctx.library.get(p).cloned())
                .map(|c| {
                    let thumb = c.thumbnail.as_ref().and_then(|t| self.image(t));
                    PickRow {
                        title: c.meta.display_title().into(),
                        detail: format!(
                            "{}{} · {}",
                            c.meta
                                .game
                                .as_deref()
                                .map(|g| format!("{g} · "))
                                .unwrap_or_default(),
                            crate::clips::format_when(c.meta.created),
                            crate::clips::format_length(c.meta.duration)
                        )
                        .into(),
                        has_thumb: thumb.is_some(),
                        thumb: thumb.unwrap_or_default(),
                        picked: self.picked_clips.contains(&c.path),
                    }
                })
                .collect();
            form.clips = ModelRc::new(VecModel::from(clips));
            form.recordings = ModelRc::new(VecModel::from(
                self.recordings
                    .iter()
                    .map(|r| RecordingRow {
                        name: r.name.clone().into(),
                        detail: r.detail.clone().into(),
                        progress: r.scan.as_ref().map_or(-1.0, |j| j.progress.get()),
                        scanned: r.highlights.is_some(),
                    })
                    .collect::<Vec<_>>(),
            ));
            let highlights: Vec<PickRow> = self
                .highlights
                .clone()
                .iter()
                .map(|(p, h)| {
                    let thumb = self.thumb(p, h.anchor);
                    PickRow {
                        title: h.reasons.join(" + ").into(),
                        detail: format!(
                            "{} · at {} · {}",
                            p.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            crate::clips::format_length(h.anchor),
                            crate::clips::format_length(h.length())
                        )
                        .into(),
                        has_thumb: thumb.is_some(),
                        thumb: thumb.unwrap_or_default(),
                        picked: self.picked_highlights.contains(&Self::highlight_key(p, h)),
                    }
                })
                .collect();
            form.highlights = ModelRc::new(VecModel::from(highlights));
            form.music_name = self
                .music_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into();
            form.music_detail = match (&self.music_job, &self.music_info) {
                (Some(_), _) => "Finding the beat…".into(),
                (None, Some(i)) => match &i.beats {
                    Some(b) => format!(
                        "{:.0} BPM · {}",
                        b.bpm,
                        crate::clips::format_length(i.duration)
                    )
                    .into(),
                    None => format!(
                        "no clear beat · {}",
                        crate::clips::format_length(i.duration)
                    )
                    .into(),
                },
                _ => SharedString::new(),
            };
            form.busy = if self.create_job.is_some() {
                "Putting it together…".into()
            } else {
                SharedString::new()
            };
        }
        if self.page == Page::Editor {
            self.editor_form(&mut form);
        }
        form
    }

    fn editor_form(&mut self, form: &mut MontageForm) {
        let Some(e) = &self.editor else { return };
        let m = e.montage.clone();
        let zoom = e.zoom;
        let px = |t: f64| (t * zoom) as f32;
        let layout = m.layout();
        let selection = e.selection;
        let position = e.preview.position();
        let duration = m.duration();
        let music_info = e.music.clone();
        let export_progress = e.export.as_ref().map(|j| j.progress.get());
        let (playing, can_undo, can_redo) = (
            e.preview.is_playing(),
            e.history.can_undo(),
            e.history.can_redo(),
        );
        let preview_image = e.preview_image.clone();
        let error = e.preview.error().unwrap_or_default();

        let blocks: Vec<BlockRow> = layout
            .iter()
            .map(|p| {
                let s = &m.segments[p.index];
                let thumb = if s.missing {
                    None
                } else {
                    self.thumb(&s.source, s.src_in)
                };
                BlockRow {
                    x: px(p.start),
                    w: px(p.end - p.start),
                    label: s
                        .source
                        .file_stem()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .into(),
                    has_thumb: thumb.is_some(),
                    thumb: thumb.unwrap_or_default(),
                    selected: selection == Selection::Segment(p.index),
                    missing: s.missing,
                    slow: s.speed < 0.99,
                    events: ModelRc::new(VecModel::from(
                        s.events
                            .iter()
                            .filter(|e| (s.src_in..=s.src_out).contains(*e))
                            .map(|e| px((e - s.src_in) / s.speed as f64))
                            .collect::<Vec<f32>>(),
                    )),
                }
            })
            .collect();
        let chips: Vec<ChipRow> = layout
            .iter()
            .skip(1)
            .enumerate()
            .map(|(i, p)| ChipRow {
                x: px(p.start + m.transition_length(i) / 2.0),
                label: m.transitions[i].label().into(),
                selected: selection == Selection::Transition(i),
            })
            .collect();
        let titles: Vec<TitleRow> = m
            .titles
            .iter()
            .map(|t| TitleRow {
                x: px(t.start),
                w: px(t.dur),
                text: t.text.clone().into(),
                selected: selection == Selection::Title(t.id),
            })
            .collect();
        form.name = m.name.clone().into();
        form.duration = duration as f32;
        form.position = position as f32;
        form.time_text = format!("{} / {}", clock(position), clock(duration)).into();
        form.playing = playing;
        form.has_preview = preview_image.is_some();
        form.preview = preview_image.unwrap_or_default();
        form.can_undo = can_undo;
        form.can_redo = can_redo;
        form.timeline_width = px(duration);
        form.playhead_x = px(position);
        form.blocks = ModelRc::new(VecModel::from(blocks));
        form.chips = ModelRc::new(VecModel::from(chips));
        form.titles = ModelRc::new(VecModel::from(titles));
        form.format = index_of(m.format);
        form.fade_out = m.fade_out;
        form.duck = m.mix.duck_db;
        form.error = error.into();
        form.export_progress = export_progress.unwrap_or(-1.0);
        if let Some(music) = &m.music {
            form.has_music = true;
            form.music_x = 0.0;
            form.music_w = px(duration);
            form.music_name = music
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into();
            form.music_gain = music.gain_db;
            form.music_fade_in = music.fade_in;
            form.music_fade_out = music.fade_out;
            let bars = (px(duration) / 4.0).ceil() as usize;
            if let Some(info) = &music_info {
                let max = info.peaks.iter().fold(1e-6f32, |a, b| a.max(*b));
                let wave: Vec<f32> = (0..bars)
                    .map(|k| {
                        let t = music.src_in + (k as f64 * 4.0) / zoom;
                        info.peaks
                            .get((t / 0.05) as usize)
                            .map_or(0.0, |p| (p / max).max(0.04))
                    })
                    .collect();
                form.waveform = ModelRc::new(VecModel::from(wave));
            }
            let beats: Vec<f32> = music
                .beats
                .iter()
                .map(|b| b - music.src_in)
                .filter(|t| (0.0..=duration).contains(t))
                .map(px)
                .collect();
            form.beats = ModelRc::new(VecModel::from(beats));
        }
        form.selection = match selection {
            Selection::None => "",
            Selection::Segment(_) => "segment",
            Selection::Transition(_) => "transition",
            Selection::Title(_) => "title",
            Selection::Music => "music",
        }
        .into();
        match selection {
            Selection::Segment(i) => {
                if let Some(s) = m.segments.get(i) {
                    form.seg_speed = s.speed;
                    form.seg_gain = s.gain_db;
                    let free = 1.0 - s.crop.w;
                    form.seg_pan = if free > 1e-3 {
                        s.crop.x / free * 2.0 - 1.0
                    } else {
                        0.0
                    };
                    form.seg_length = format!(
                        "{} on the timeline · {}–{} of the source",
                        crate::clips::format_length(s.length()),
                        clock(s.src_in),
                        clock(s.src_out)
                    )
                    .into();
                    form.seg_source = s.source.display().to_string().into();
                }
            }
            Selection::Transition(i) => {
                if let Some(t) = m.transitions.get(i) {
                    form.trans_kind = match t {
                        Transition::Cut => 0,
                        Transition::Crossfade { .. } => 1,
                        Transition::DipToBlack { .. } => 2,
                        Transition::Whip { .. } => 3,
                    };
                    form.trans_secs = t.secs() as f32;
                }
            }
            Selection::Title(id) => {
                if let Some(t) = m.titles.iter().find(|t| t.id == id) {
                    form.title_edit = t.text.clone().into();
                    form.title_style = (t.style == TitleStyle::Clean) as i32;
                    form.title_anchor = match t.anchor {
                        Anchor::Top => 0,
                        Anchor::Center => 1,
                        Anchor::Bottom => 2,
                    };
                    form.title_start = t.start as f32;
                    form.title_dur = t.dur as f32;
                }
            }
            _ => {}
        }
    }
}

/// The metadata of an exported montage added to the clip library.
pub fn clip_meta(title: &str, duration: f64) -> estream_clips::ClipMeta {
    estream_clips::ClipMeta {
        game: Some("Montages".into()),
        created: chrono::Utc::now(),
        trigger: Trigger::Manual,
        duration,
        title: Some(title.to_owned()),
        ..estream_clips::ClipMeta::default()
    }
}
