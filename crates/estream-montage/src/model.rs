//! The montage document: one video track of segments with transitions between them, text
//! titles on the timeline, an optional music track and the mix.
//!
//! Timeline times are never stored; they follow from the segments: segment `i` lasts
//! `(src_out - src_in) / speed`, and the transition after it overlaps it with the next one.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const VERSION: u32 = 1;
/// The montage file extension.
pub const EXTENSION: &str = "clutch-montage";
/// The extension up to 0.2, when the app was called eStream (still read).
pub const LEGACY_EXTENSION: &str = "estream-montage";

/// Whether `path` is a montage file.
pub fn is_montage(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == EXTENSION || e == LEGACY_EXTENSION)
}

/// Output size and frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Format {
    pub const LANDSCAPE: Format = Format {
        width: 1920,
        height: 1080,
        fps: 60,
    };
    pub const VERTICAL: Format = Format {
        width: 1080,
        height: 1920,
        fps: 60,
    };
    pub const SQUARE: Format = Format {
        width: 1080,
        height: 1080,
        fps: 60,
    };

    pub fn aspect(self) -> f64 {
        self.width as f64 / self.height.max(1) as f64
    }

    pub fn frame_secs(self) -> f64 {
        1.0 / self.fps.max(1) as f64
    }
}

impl Default for Format {
    fn default() -> Self {
        Self::LANDSCAPE
    }
}

/// The part of the source picture to show, in 0..1 of its width and height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Crop {
    pub const FULL: Crop = Crop {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    /// The centered crop of a `source_aspect` picture that fills a `target_aspect` frame,
    /// moved horizontally by `pan` (-1 = left edge, 0 = center, 1 = right edge).
    pub fn fill(source_aspect: f64, target_aspect: f64, pan: f32) -> Crop {
        if source_aspect > target_aspect {
            let w = (target_aspect / source_aspect) as f32;
            let free = 1.0 - w;
            Crop {
                x: free * (pan.clamp(-1.0, 1.0) + 1.0) / 2.0,
                y: 0.0,
                w,
                h: 1.0,
            }
        } else {
            let h = (source_aspect / target_aspect) as f32;
            Crop {
                x: 0.0,
                y: (1.0 - h) / 2.0,
                w: 1.0,
                h,
            }
        }
    }
}

impl Default for Crop {
    fn default() -> Self {
        Self::FULL
    }
}

/// One piece of a source on the video track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Stable within a montage (for selection in the editor).
    pub id: u64,
    /// Absolute path of the clip or recording.
    pub source: PathBuf,
    /// The same file relative to the montage file, for when a folder moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rel: Option<PathBuf>,
    /// Seconds in the source.
    pub src_in: f64,
    pub src_out: f64,
    /// Playback speed, 0.25..=2.
    pub speed: f32,
    #[serde(default)]
    pub crop: Crop,
    #[serde(default)]
    pub gain_db: f32,
    /// Event times in source seconds (for snapping and slow motion).
    #[serde(default)]
    pub events: Vec<f64>,
    /// The source file wasn't found when the montage was opened.
    #[serde(skip)]
    pub missing: bool,
}

pub const MIN_SPEED: f32 = 0.25;
pub const MAX_SPEED: f32 = 2.0;
/// The shortest a segment may be on the timeline.
pub const MIN_SEGMENT: f64 = 0.2;

impl Segment {
    pub fn new(id: u64, source: impl Into<PathBuf>, src_in: f64, src_out: f64) -> Self {
        Self {
            id,
            source: source.into(),
            source_rel: None,
            src_in,
            src_out,
            speed: 1.0,
            crop: Crop::FULL,
            gain_db: 0.0,
            events: Vec::new(),
            missing: false,
        }
    }

    /// Seconds on the timeline.
    pub fn length(&self) -> f64 {
        (self.src_out - self.src_in).max(0.0) / self.speed.clamp(MIN_SPEED, MAX_SPEED) as f64
    }

    /// The source time shown `offset` seconds into the segment.
    pub fn source_time(&self, offset: f64) -> f64 {
        (self.src_in + offset * self.speed.clamp(MIN_SPEED, MAX_SPEED) as f64)
            .clamp(self.src_in, self.src_out)
    }
}

/// What happens between two segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transition {
    #[default]
    Cut,
    Crossfade {
        secs: f32,
    },
    DipToBlack {
        secs: f32,
    },
    /// A fast push with motion blur.
    Whip {
        secs: f32,
    },
}

impl Transition {
    pub fn secs(self) -> f64 {
        match self {
            Transition::Cut => 0.0,
            Transition::Crossfade { secs }
            | Transition::DipToBlack { secs }
            | Transition::Whip { secs } => secs.max(0.0) as f64,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Transition::Cut => "Cut",
            Transition::Crossfade { .. } => "Crossfade",
            Transition::DipToBlack { .. } => "Dip to black",
            Transition::Whip { .. } => "Whip",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleStyle {
    /// Big, heavy, with an outline: for intros.
    #[default]
    Bold,
    /// Smaller, clean, with a soft shadow: for captions.
    Clean,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Top,
    #[default]
    Center,
    Bottom,
}

/// Text over the video, at timeline times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Title {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub style: TitleStyle,
    pub start: f64,
    pub dur: f64,
    #[serde(default)]
    pub anchor: Anchor,
}

/// The music track: starts at timeline 0, from `src_in` in the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Music {
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rel: Option<PathBuf>,
    #[serde(default)]
    pub src_in: f64,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default = "default_fade_out")]
    pub fade_out: f32,
    /// Beat times in the music file (seconds), from beat detection.
    #[serde(default)]
    pub beats: Vec<f64>,
    #[serde(skip)]
    pub missing: bool,
}

fn default_fade_out() -> f32 {
    2.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mix {
    /// Gain on all clip audio.
    pub clip_gain_db: f32,
    /// How far the music drops while clips have sound.
    pub duck_db: f32,
}

impl Default for Mix {
    fn default() -> Self {
        Self {
            clip_gain_db: 0.0,
            duck_db: 8.0,
        }
    }
}

/// A segment's place on the timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Montage {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub segments: Vec<Segment>,
    /// `transitions[i]` sits between `segments[i]` and `segments[i + 1]`; always one fewer
    /// than the segments.
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub titles: Vec<Title>,
    #[serde(default)]
    pub music: Option<Music>,
    #[serde(default)]
    pub mix: Mix,
    /// Seconds of fade to black (picture and sound) at the very end.
    #[serde(default)]
    pub fade_out: f32,
    /// The next free id for segments and titles.
    #[serde(default)]
    pub next_id: u64,
}

impl Default for Montage {
    fn default() -> Self {
        Self::new("Montage", Format::default())
    }
}

impl Montage {
    pub fn new(name: impl Into<String>, format: Format) -> Self {
        Self {
            version: VERSION,
            name: name.into(),
            format,
            segments: Vec::new(),
            transitions: Vec::new(),
            titles: Vec::new(),
            music: None,
            mix: Mix::default(),
            fade_out: 0.0,
            next_id: 1,
        }
    }

    pub fn new_id(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    /// Keeps `transitions` one shorter than `segments` (extra ones dropped, cuts added).
    pub fn normalize(&mut self) {
        let want = self.segments.len().saturating_sub(1);
        self.transitions.resize(want, Transition::Cut);
        let max = self
            .segments
            .iter()
            .map(|s| s.id)
            .chain(self.titles.iter().map(|t| t.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
    }

    /// How long transition `i` really lasts: at most half of either segment around it.
    pub fn transition_length(&self, i: usize) -> f64 {
        let (Some(t), Some(a), Some(b)) = (
            self.transitions.get(i),
            self.segments.get(i),
            self.segments.get(i + 1),
        ) else {
            return 0.0;
        };
        t.secs().min(a.length() / 2.0).min(b.length() / 2.0)
    }

    /// Every segment's start and end on the timeline.
    pub fn layout(&self) -> Vec<Placed> {
        let mut out = Vec::with_capacity(self.segments.len());
        let mut start = 0.0;
        for (i, s) in self.segments.iter().enumerate() {
            let end = start + s.length();
            out.push(Placed {
                index: i,
                start,
                end,
            });
            start = end - self.transition_length(i);
        }
        out
    }

    /// Timeline length in seconds.
    pub fn duration(&self) -> f64 {
        self.layout().last().map_or(0.0, |p| p.end)
    }

    /// What to show at timeline time `t`.
    pub fn frame_at(&self, t: f64) -> FrameAt {
        let layout = self.layout();
        for (k, p) in layout.iter().enumerate() {
            if t < p.start || t >= p.end {
                continue;
            }
            let seg = &self.segments[p.index];
            let a = (p.index, seg.source_time(t - p.start));
            // Inside the overlap with the next segment: a transition.
            if let Some(next) = layout.get(k + 1)
                && t >= next.start
            {
                let len = self.transition_length(p.index);
                let progress = if len > 0.0 {
                    ((t - next.start) / len).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let b_seg = &self.segments[next.index];
                return FrameAt::Transition {
                    from: a,
                    to: (next.index, b_seg.source_time(t - next.start)),
                    transition: self.transitions[p.index],
                    progress: progress as f32,
                };
            }
            return FrameAt::Segment(a.0, a.1);
        }
        FrameAt::Black
    }

    /// Titles visible at `t`.
    pub fn titles_at(&self, t: f64) -> impl Iterator<Item = &Title> {
        self.titles
            .iter()
            .filter(move |title| t >= title.start && t < title.start + title.dur)
    }

    // ------------------------------------------------------------ files

    /// Saves to `path` (JSON), recording sources also relative to the montage's folder.
    pub fn save(&self, path: &Path) -> Result<(), MontageError> {
        let mut copy = self.clone();
        copy.version = VERSION;
        let dir = path.parent().unwrap_or(Path::new("."));
        for s in &mut copy.segments {
            s.source_rel = relative(dir, &s.source);
        }
        if let Some(m) = &mut copy.music {
            m.source_rel = relative(dir, &m.source);
        }
        let json = serde_json::to_vec_pretty(&copy)?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// Opens a montage. Sources are looked for at their absolute path, then relative to the
    /// montage's folder; segments whose file is in neither place are flagged `missing`.
    pub fn load(path: &Path) -> Result<Self, MontageError> {
        let text = std::fs::read_to_string(path)?;
        let mut montage: Montage = serde_json::from_str(&text)?;
        if montage.version > VERSION {
            return Err(MontageError::TooNew(montage.version));
        }
        montage.version = VERSION;
        montage.normalize();
        let dir = path.parent().unwrap_or(Path::new("."));
        for s in &mut montage.segments {
            s.missing = !locate(dir, &mut s.source, s.source_rel.as_deref());
        }
        if let Some(m) = &mut montage.music {
            m.missing = !locate(dir, &mut m.source, m.source_rel.as_deref());
        }
        Ok(montage)
    }

    /// Points missing segments at `folder`: any missing source whose file name is there is
    /// relinked. Returns how many were found.
    pub fn relink(&mut self, folder: &Path) -> usize {
        let mut found = 0;
        let mut fix = |source: &mut PathBuf, missing: &mut bool| {
            if !*missing {
                return;
            }
            if let Some(name) = source.file_name() {
                let candidate = folder.join(name);
                if candidate.is_file() {
                    *source = candidate;
                    *missing = false;
                    found += 1;
                }
            }
        };
        for s in &mut self.segments {
            fix(&mut s.source, &mut s.missing);
        }
        if let Some(m) = &mut self.music {
            fix(&mut m.source, &mut m.missing);
        }
        found
    }
}

/// What the renderer shows at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrameAt {
    Black,
    /// Segment index and source time.
    Segment(usize, f64),
    Transition {
        from: (usize, f64),
        to: (usize, f64),
        transition: Transition,
        progress: f32,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MontageError {
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("this montage file is damaged: {0}")]
    Json(#[from] serde_json::Error),
    #[error("this montage was made by a newer Clutch (format {0})")]
    TooNew(u32),
}

/// `target` relative to `dir` (with `..` as needed), if both are on the same drive.
fn relative(dir: &Path, target: &Path) -> Option<PathBuf> {
    let dir: Vec<Component> = dir.components().collect();
    let target: Vec<Component> = target.components().collect();
    // Different roots (drives) can't be related.
    if dir.first() != target.first() {
        return None;
    }
    let common = dir.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..dir.len() {
        out.push("..");
    }
    for c in &target[common..] {
        out.push(c.as_os_str());
    }
    Some(out)
}

/// Finds a source: its absolute path, else relative to `dir`. Updates `source` when found
/// the second way.
fn locate(dir: &Path, source: &mut PathBuf, rel: Option<&Path>) -> bool {
    if source.is_file() {
        return true;
    }
    if let Some(rel) = rel {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            *source = candidate;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: u64, len: f64) -> Segment {
        Segment::new(id, format!("C:/clips/{id}.mp4"), 10.0, 10.0 + len)
    }

    fn montage(lens: &[f64], transitions: &[Transition]) -> Montage {
        let mut m = Montage {
            segments: lens
                .iter()
                .enumerate()
                .map(|(i, l)| seg(i as u64 + 1, *l))
                .collect(),
            transitions: transitions.to_vec(),
            ..Montage::default()
        };
        m.normalize();
        m
    }

    #[test]
    fn durations_follow_speed_and_transitions() {
        let mut m = montage(&[4.0, 6.0, 2.0], &[Transition::Cut]);
        assert_eq!(m.transitions.len(), 2, "normalized to segments - 1");
        assert!((m.duration() - 12.0).abs() < 1e-9);
        m.segments[1].speed = 0.5; // 6 s of source take 12 s
        assert!((m.duration() - 18.0).abs() < 1e-9);
        m.transitions[0] = Transition::Crossfade { secs: 1.0 };
        assert!((m.duration() - 17.0).abs() < 1e-9);
        let layout = m.layout();
        assert!((layout[1].start - 3.0).abs() < 1e-9 && (layout[1].end - 15.0).abs() < 1e-9);
        // A transition is at most half of the shorter neighbour.
        m.transitions[1] = Transition::DipToBlack { secs: 5.0 };
        assert!((m.transition_length(1) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn frames_inside_and_between_segments() {
        let m = montage(&[4.0, 4.0], &[Transition::Crossfade { secs: 1.0 }]);
        assert_eq!(m.frame_at(1.0), FrameAt::Segment(0, 11.0));
        match m.frame_at(3.5) {
            FrameAt::Transition {
                from, to, progress, ..
            } => {
                assert_eq!(from, (0, 13.5));
                assert_eq!(to, (1, 10.5));
                assert!((progress - 0.5).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.frame_at(5.0), FrameAt::Segment(1, 12.0));
        assert_eq!(m.frame_at(7.0), FrameAt::Black, "past the end");
    }

    #[test]
    fn slow_motion_maps_timeline_to_source() {
        let mut s = seg(1, 2.0);
        s.speed = 0.5;
        assert!((s.length() - 4.0).abs() < 1e-9);
        assert!((s.source_time(2.0) - 11.0).abs() < 1e-9);
        assert!(
            (s.source_time(99.0) - 12.0).abs() < 1e-9,
            "clamped to the out point"
        );
    }

    #[test]
    fn crop_fills_a_vertical_frame_from_landscape() {
        let c = Crop::fill(16.0 / 9.0, 9.0 / 16.0, 0.0);
        assert!((c.w - 0.31640625).abs() < 1e-4, "{c:?}");
        assert!((c.x - (1.0 - c.w) / 2.0).abs() < 1e-6, "centered");
        let left = Crop::fill(16.0 / 9.0, 9.0 / 16.0, -1.0);
        assert_eq!(left.x, 0.0);
        let full = Crop::fill(16.0 / 9.0, 16.0 / 9.0, 0.0);
        assert!((full.w - 1.0).abs() < 1e-6 && (full.h - 1.0).abs() < 1e-6);
    }

    #[test]
    fn save_load_and_relink_moved_files() {
        let root =
            std::env::temp_dir().join(format!("estream-montage-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let clips = root.join("Clips");
        let _ = std::fs::create_dir_all(&clips);
        for name in ["a.mp4", "b.mp4"] {
            let _ = std::fs::write(clips.join(name), b"x");
        }
        let mut m = Montage::new("Test", Format::VERTICAL);
        let id = m.new_id();
        m.segments
            .push(Segment::new(id, clips.join("a.mp4"), 0.0, 3.0));
        let id = m.new_id();
        m.segments
            .push(Segment::new(id, clips.join("b.mp4"), 1.0, 4.0));
        m.normalize();
        let file = root.join("My montage.estream-montage");
        assert!(m.save(&file).is_ok());

        // Moving the whole folder keeps it working through the relative paths.
        let moved =
            std::env::temp_dir().join(format!("estream-montage-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&moved);
        assert!(std::fs::rename(&root, &moved).is_ok());
        let loaded = Montage::load(&moved.join("My montage.estream-montage"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(loaded.segments.iter().all(|s| !s.missing));
        assert_eq!(loaded.segments[0].source, moved.join("Clips").join("a.mp4"));
        assert_eq!(loaded.format, Format::VERTICAL);

        // Moving only the clips leaves them missing until relinked.
        let elsewhere = moved.join("Elsewhere");
        let _ = std::fs::create_dir_all(&elsewhere);
        for name in ["a.mp4", "b.mp4"] {
            let _ = std::fs::rename(moved.join("Clips").join(name), elsewhere.join(name));
        }
        let mut again = Montage::load(&moved.join("My montage.estream-montage"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(again.segments.iter().all(|s| s.missing));
        assert_eq!(again.relink(&elsewhere), 2);
        assert!(again.segments.iter().all(|s| !s.missing));
    }

    #[test]
    fn newer_files_are_refused() {
        let path =
            std::env::temp_dir().join(format!("estream-montage-new-{}.json", std::process::id()));
        let _ = std::fs::write(&path, r#"{ "version": 99, "name": "x" }"#);
        assert!(matches!(
            Montage::load(&path),
            Err(MontageError::TooNew(99))
        ));
    }
}
