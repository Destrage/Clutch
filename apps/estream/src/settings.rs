//! App-level settings (not part of a scene collection): paths, appearance, window
//! and panel layout. Stored as JSON in the user's config directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Locations of eStream's files.
#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let root = std::env::var_os("ESTREAM_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|d| d.join("eStream")))
            .unwrap_or_else(|| PathBuf::from(".estream"));
        Self { root }
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn scenes(&self) -> PathBuf {
        self.root.join("scenes")
    }

    pub fn default_collection(&self) -> PathBuf {
        self.scenes().join("Untitled.json")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn instance_lock(&self) -> PathBuf {
        self.root.join("instance.lock")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelLayout {
    pub scenes_width: f32,
    pub inspector_width: f32,
    pub bottom_height: f32,
    pub sources_width: f32,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            scenes_width: 220.0,
            inspector_width: 300.0,
            bottom_height: 230.0,
            sources_width: 420.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    /// Accent color as `#rrggbb`.
    pub accent: String,
    pub window: Option<WindowState>,
    pub layout: PanelLayout,
    pub snapping: bool,
    pub last_collection: Option<PathBuf>,
    pub recording: estream_media::RecordingSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            accent: "#6c8cff".into(),
            window: None,
            layout: PanelLayout::default(),
            snapping: true,
            last_collection: None,
            recording: estream_media::RecordingSettings::default(),
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults (and logging) if missing or invalid.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "invalid settings file; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        let result = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                estream_project::persist::write_atomic(path, &bytes).map_err(std::io::Error::other)
            });
        if let Err(e) = result {
            tracing::error!(error = %e, "failed to save settings");
        }
    }
}

/// Parses `#rrggbb` into a Slint color.
pub fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// True if the OS is in dark mode (Windows registry); defaults to dark elsewhere.
pub fn system_prefers_dark() -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output();
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout);
            return !text.contains("0x1");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let s = Settings {
            theme: ThemeMode::Light,
            window: Some(WindowState {
                x: 10,
                y: 20,
                width: 1600,
                height: 900,
                maximized: false,
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        // Missing fields fall back to defaults.
        let partial: Settings = serde_json::from_str("{\"theme\":\"system\"}").unwrap();
        assert_eq!(partial.theme, ThemeMode::System);
        assert!(partial.snapping);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#6c8cff"), Some((0x6c, 0x8c, 0xff)));
        assert_eq!(parse_hex("zzz"), None);
    }
}
