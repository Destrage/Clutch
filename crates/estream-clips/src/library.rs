//! The clip library: clip files in a folder, each with a JSON sidecar and a thumbnail.
//!
//! ```text
//! <root>/<Game>/<date> <time> <reason>.mp4
//!                                  .json   metadata (game, trigger, events, favorite, title)
//!                                  .jpg    thumbnail
//! ```
//!
//! The index is rebuilt by scanning the folder ([`Library::open`], [`Library::refresh`]), so
//! clips added, removed or renamed outside Clutch show up. A clip file without a sidecar is
//! listed as a manual clip with no game.

use crate::event::EventKind;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv"];
pub const META_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Manual,
    Auto,
}

/// An event inside a clip, `at` seconds from the clip's start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipMark {
    #[serde(flatten)]
    pub kind: EventKind,
    pub at: f64,
}

/// A clip's sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipMeta {
    pub version: u32,
    pub game: Option<String>,
    pub created: DateTime<Utc>,
    pub trigger: Trigger,
    pub events: Vec<ClipMark>,
    pub duration: f64,
    pub favorite: bool,
    pub title: Option<String>,
}

impl Default for ClipMeta {
    fn default() -> Self {
        Self {
            version: META_VERSION,
            game: None,
            created: DateTime::<Utc>::UNIX_EPOCH,
            trigger: Trigger::Manual,
            events: Vec::new(),
            duration: 0.0,
            favorite: false,
            title: None,
        }
    }
}

impl ClipMeta {
    /// The title to show: the user's, or the strongest event, or "Clip".
    pub fn display_title(&self) -> String {
        if let Some(t) = self.title.as_deref().filter(|t| !t.trim().is_empty()) {
            return t.to_owned();
        }
        self.events
            .iter()
            .max_by(|a, b| a.kind.weight().total_cmp(&b.kind.weight()))
            .map_or_else(|| "Clip".to_owned(), |m| m.kind.label())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub path: PathBuf,
    pub meta: ClipMeta,
    /// Bytes of the clip file.
    pub size: u64,
    pub thumbnail: Option<PathBuf>,
}

pub fn sidecar_path(clip: &Path) -> PathBuf {
    clip.with_extension("json")
}

pub fn thumbnail_path(clip: &Path) -> PathBuf {
    clip.with_extension("jpg")
}

/// Removes characters Windows doesn't allow in file names.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = trimmed.trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() {
        "Clip".to_owned()
    } else {
        trimmed
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad clip metadata: {0}")]
    Json(#[from] serde_json::Error),
    #[error("that clip isn't in the library")]
    NotFound,
}

#[derive(Debug)]
pub struct Library {
    root: PathBuf,
    /// Newest first.
    clips: Vec<Clip>,
}

impl Library {
    /// Opens (and scans) the library at `root`; the folder needn't exist yet.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let mut lib = Self {
            root: root.into(),
            clips: Vec::new(),
        };
        lib.refresh();
        lib
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Newest first.
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    pub fn get(&self, path: &Path) -> Option<&Clip> {
        self.clips.iter().find(|c| c.path == path)
    }

    /// Games that have clips, sorted.
    pub fn games(&self) -> Vec<String> {
        let mut games: Vec<String> = self
            .clips
            .iter()
            .filter_map(|c| c.meta.game.clone())
            .collect();
        games.sort_by_key(|g| g.to_lowercase());
        games.dedup();
        games
    }

    /// Total bytes of clips matching `filter`.
    pub fn total_size(&self, filter: impl Fn(&Clip) -> bool) -> u64 {
        self.clips
            .iter()
            .filter(|c| filter(c))
            .map(|c| c.size)
            .sum()
    }

    /// Rescans the folder.
    pub fn refresh(&mut self) {
        let mut files = Vec::new();
        collect_videos(&self.root, 0, &mut files);
        let mut clips: Vec<Clip> = files.into_iter().map(|p| load_clip(&p)).collect();
        clips.sort_by(|a, b| {
            b.meta
                .created
                .cmp(&a.meta.created)
                .then_with(|| a.path.cmp(&b.path))
        });
        self.clips = clips;
    }

    /// Where a new clip goes: `<root>/<Game>/<local date time> <reason>.<ext>`, made unique.
    pub fn new_clip_path(
        &self,
        game: Option<&str>,
        created: DateTime<Utc>,
        reason: &str,
        ext: &str,
    ) -> PathBuf {
        let folder = self.root.join(sanitize(game.unwrap_or("Other")));
        let stamp = created.with_timezone(&Local).format("%Y-%m-%d %H-%M-%S");
        let base = format!("{stamp} {}", sanitize(reason));
        let mut path = folder.join(format!("{base}.{ext}"));
        let mut n = 2;
        while path.exists() || self.clips.iter().any(|c| c.path == path) {
            path = folder.join(format!("{base} ({n}).{ext}"));
            n += 1;
        }
        path
    }

    /// Adds a clip whose file has been written: saves its sidecar and lists it.
    pub fn add(&mut self, path: &Path, meta: ClipMeta) -> Result<&Clip, LibraryError> {
        write_meta(path, &meta)?;
        let clip = load_clip(path);
        self.clips.retain(|c| c.path != path);
        let i = self
            .clips
            .partition_point(|c| c.meta.created > clip.meta.created);
        self.clips.insert(i, clip);
        Ok(&self.clips[i])
    }

    /// Changes a clip's metadata (title, favorite…) and saves it.
    pub fn update(
        &mut self,
        path: &Path,
        change: impl FnOnce(&mut ClipMeta),
    ) -> Result<(), LibraryError> {
        let clip = self
            .clips
            .iter_mut()
            .find(|c| c.path == path)
            .ok_or(LibraryError::NotFound)?;
        change(&mut clip.meta);
        write_meta(path, &clip.meta)
    }

    /// Re-reads one clip's file size and thumbnail (e.g. after a trim replaced it).
    pub fn reload(&mut self, path: &Path) {
        if let Some(c) = self.clips.iter_mut().find(|c| c.path == path) {
            *c = load_clip(path);
        }
    }

    /// Deletes a clip with its sidecar and thumbnail.
    pub fn delete(&mut self, path: &Path) -> Result<(), LibraryError> {
        let i = self
            .clips
            .iter()
            .position(|c| c.path == path)
            .ok_or(LibraryError::NotFound)?;
        remove_if_exists(path)?;
        remove_if_exists(&sidecar_path(path))?;
        remove_if_exists(&thumbnail_path(path))?;
        self.clips.remove(i);
        Ok(())
    }

    /// Deletes the oldest automatic clips that aren't favorites until automatic clips use at
    /// most `cap` bytes. Manual clips and favorites are never deleted. Returns what was deleted.
    pub fn enforce_cap(&mut self, cap: u64) -> Vec<PathBuf> {
        let deletable = |c: &Clip| c.meta.trigger == Trigger::Auto && !c.meta.favorite;
        let mut total = self.total_size(|c| c.meta.trigger == Trigger::Auto);
        let mut oldest_first: Vec<PathBuf> = self
            .clips
            .iter()
            .rev()
            .filter(|c| deletable(c))
            .map(|c| c.path.clone())
            .collect();
        oldest_first.reverse();
        let mut deleted = Vec::new();
        while total > cap {
            let Some(path) = oldest_first.pop() else {
                break; // only favorites left
            };
            let size = self.get(&path).map_or(0, |c| c.size);
            match self.delete(&path) {
                Ok(()) => {
                    total = total.saturating_sub(size);
                    deleted.push(path);
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "couldn't delete an old clip")
                }
            }
        }
        deleted
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn write_meta(clip: &Path, meta: &ClipMeta) -> Result<(), LibraryError> {
    let json = serde_json::to_string_pretty(meta)?;
    let path = sidecar_path(clip);
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

/// Video files under `dir` (game folders one level down, plus loose clips).
fn collect_videos(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth < 2 {
            collect_videos(&path, depth + 1, out);
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            && !path.to_string_lossy().contains(".part.")
        {
            out.push(path);
        }
    }
}

fn load_clip(path: &Path) -> Clip {
    let file = std::fs::metadata(path).ok();
    let meta = std::fs::read_to_string(sidecar_path(path))
        .ok()
        .and_then(|s| serde_json::from_str::<ClipMeta>(&s).ok())
        .unwrap_or_else(|| ClipMeta {
            // No sidecar: a manual clip dated by the file.
            created: file
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map_or(DateTime::<Utc>::UNIX_EPOCH, |t: SystemTime| t.into()),
            ..ClipMeta::default()
        });
    let thumb = thumbnail_path(path);
    Clip {
        path: path.to_path_buf(),
        size: file.map_or(0, |m| m.len()),
        thumbnail: thumb.is_file().then_some(thumb),
        meta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("estream-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 18, 20, minute, 0)
            .single()
            .unwrap_or_default()
    }

    /// Writes a fake clip of `size` bytes and adds it.
    fn add(lib: &mut Library, game: &str, minute: u32, trigger: Trigger, size: usize) -> PathBuf {
        let path = lib.new_clip_path(Some(game), at(minute), "Kill", "mp4");
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&path, vec![0u8; size]);
        let _ = std::fs::write(thumbnail_path(&path), b"jpg");
        let meta = ClipMeta {
            game: Some(game.into()),
            created: at(minute),
            trigger,
            events: vec![ClipMark {
                kind: EventKind::Kill,
                at: 10.0,
            }],
            duration: 13.0,
            ..ClipMeta::default()
        };
        let _ = lib.add(&path, meta);
        path
    }

    #[test]
    fn sidecars_round_trip_and_the_index_is_newest_first() {
        let root = temp("index");
        let mut lib = Library::open(&root);
        let a = add(&mut lib, "Counter-Strike 2", 1, Trigger::Auto, 10);
        let b = add(&mut lib, "Dota 2", 5, Trigger::Manual, 10);
        let c = add(&mut lib, "Counter-Strike 2", 3, Trigger::Auto, 10);
        assert_eq!(
            lib.clips()
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>(),
            [b.clone(), c.clone(), a.clone()]
        );
        // A fresh scan finds the same clips with their metadata and thumbnails.
        let again = Library::open(&root);
        assert_eq!(again.clips().len(), 3);
        let clip = again.get(&a).unwrap_or_else(|| panic!("a"));
        assert_eq!(clip.meta.game.as_deref(), Some("Counter-Strike 2"));
        assert_eq!(clip.meta.events[0].kind, EventKind::Kill);
        assert!(clip.thumbnail.is_some());
        assert_eq!(again.games(), ["Counter-Strike 2", "Dota 2"]);
        let json = std::fs::read_to_string(sidecar_path(&a)).unwrap_or_default();
        assert!(json.contains("\"kind\": \"kill\""), "{json}");
    }

    #[test]
    fn a_clip_without_a_sidecar_is_a_manual_clip_with_no_game() {
        let root = temp("bare");
        let _ = std::fs::write(root.join("dropped in.mp4"), b"video");
        let _ = std::fs::write(root.join("notes.txt"), b"not a clip");
        let _ = std::fs::write(root.join("half.part.mp4"), b"being written");
        let lib = Library::open(&root);
        assert_eq!(lib.clips().len(), 1);
        let clip = &lib.clips()[0];
        assert_eq!(clip.meta.trigger, Trigger::Manual);
        assert_eq!(clip.meta.game, None);
        assert!(
            clip.meta.created > DateTime::<Utc>::UNIX_EPOCH,
            "dated by the file"
        );
        assert_eq!(clip.meta.display_title(), "Clip");
    }

    #[test]
    fn update_and_delete() {
        let root = temp("edit");
        let mut lib = Library::open(&root);
        let a = add(&mut lib, "Valorant", 1, Trigger::Auto, 10);
        assert!(
            lib.update(&a, |m| {
                m.favorite = true;
                m.title = Some("Clutch".into());
            })
            .is_ok()
        );
        let again = Library::open(&root);
        let clip = again.get(&a).unwrap_or_else(|| panic!("a"));
        assert!(clip.meta.favorite);
        assert_eq!(clip.meta.display_title(), "Clutch");
        assert!(lib.delete(&a).is_ok());
        assert!(!a.exists() && !sidecar_path(&a).exists() && !thumbnail_path(&a).exists());
        assert!(lib.clips().is_empty());
        assert!(matches!(lib.delete(&a), Err(LibraryError::NotFound)));
    }

    #[test]
    fn storage_cap_deletes_oldest_auto_clips_only() {
        let root = temp("cap");
        let mut lib = Library::open(&root);
        let old_auto = add(&mut lib, "CS2", 1, Trigger::Auto, 100);
        let old_fav = add(&mut lib, "CS2", 2, Trigger::Auto, 100);
        let manual = add(&mut lib, "CS2", 3, Trigger::Manual, 500);
        let mid_auto = add(&mut lib, "CS2", 4, Trigger::Auto, 100);
        let new_auto = add(&mut lib, "CS2", 5, Trigger::Auto, 100);
        let _ = lib.update(&old_fav, |m| m.favorite = true);
        // Auto clips: 400 bytes. Cap 250 → delete the two oldest non-favorites.
        let deleted = lib.enforce_cap(250);
        assert_eq!(deleted, [old_auto.clone(), mid_auto.clone()]);
        assert!(
            lib.get(&old_fav).is_some()
                && lib.get(&manual).is_some()
                && lib.get(&new_auto).is_some()
        );
        // Only the favorite and the manual clip could remain past this point.
        let deleted = lib.enforce_cap(0);
        assert_eq!(deleted, [new_auto]);
        assert_eq!(lib.clips().len(), 2);
    }

    #[test]
    fn clip_paths_are_safe_and_unique() {
        let root = temp("names");
        let lib = Library::open(&root);
        let p = lib.new_clip_path(
            Some("Tom Clancy's: R6 / Siege?"),
            at(7),
            "Double kill",
            "mp4",
        );
        let folder = p
            .parent()
            .and_then(|f| f.file_name())
            .map(|f| f.to_string_lossy().into_owned());
        assert_eq!(folder.as_deref(), Some("Tom Clancy's R6 Siege"));
        let name = p
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(name.ends_with(" Double kill.mp4"), "{name}");
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&p, b"x");
        let q = lib.new_clip_path(
            Some("Tom Clancy's: R6 / Siege?"),
            at(7),
            "Double kill",
            "mp4",
        );
        assert_ne!(p, q);
        assert!(q.to_string_lossy().ends_with(" Double kill (2).mp4"));
        assert_eq!(sanitize("  ..  "), "Clip");
    }
}
