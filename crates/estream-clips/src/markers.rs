//! Recording markers: the game events and hype peaks that happened during a recording, saved
//! next to it as `<recording>.markers.json` (times in seconds from the recording's start).
//! The montage editor reads them to find highlights.

use crate::event::EventKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MARKERS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    #[serde(flatten)]
    pub kind: EventKind,
    #[serde(default)]
    pub game: Option<String>,
    /// Seconds from the start of the recording.
    pub at: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Markers {
    pub version: u32,
    pub markers: Vec<Marker>,
}

/// `video.mkv` → `video.markers.json`.
pub fn markers_path(recording: &Path) -> PathBuf {
    recording.with_extension("markers.json")
}

impl Markers {
    pub fn load(recording: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(markers_path(recording)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, recording: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let path = markers_path(recording);
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(temp, path)
    }

    /// Adds a marker, keeping them in time order.
    pub fn push(&mut self, marker: Marker) {
        self.version = MARKERS_VERSION;
        let i = self.markers.partition_point(|m| m.at <= marker.at);
        self.markers.insert(i, marker);
    }

    /// Moves the marker file along with its recording (e.g. after a remux to MP4).
    pub fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
        let (a, b) = (markers_path(from), markers_path(to));
        if a.exists() && a != b {
            std::fs::rename(a, b)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip_in_time_order_and_follow_a_remux() {
        let dir = std::env::temp_dir().join(format!("estream-markers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let mkv = dir.join("match.mkv");
        assert_eq!(markers_path(&mkv), dir.join("match.markers.json"));
        let mut m = Markers::default();
        m.push(Marker {
            kind: EventKind::Ace,
            game: Some("Counter-Strike 2".into()),
            at: 300.5,
        });
        m.push(Marker {
            kind: EventKind::Kill,
            game: Some("Counter-Strike 2".into()),
            at: 12.0,
        });
        assert!(m.save(&mkv).is_ok());
        let mp4 = dir.join("match.mp4");
        assert!(Markers::rename(&mkv, &mp4).is_ok());
        let loaded = Markers::load(&mp4).unwrap_or_default();
        assert_eq!(loaded.markers.len(), 2);
        assert_eq!(loaded.markers[0].kind, EventKind::Kill);
        assert_eq!(loaded.markers[1].at, 300.5);
        // The name doesn't include the video's extension, so a remux keeps the same file.
        assert_eq!(markers_path(&mkv), markers_path(&mp4));
        // A real rename (another name) moves it.
        let renamed = dir.join("final cut.mp4");
        assert!(Markers::rename(&mp4, &renamed).is_ok());
        assert!(Markers::load(&renamed).is_some() && Markers::load(&mp4).is_none());
    }
}
