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
    /// Where Go Live streams to.
    pub destinations: Vec<DestinationConfig>,
    /// The obs-websocket compatible remote control server.
    pub remote: RemoteSettings,
    /// Canvas shown by the virtual camera (`None` = the first canvas).
    pub vcam_canvas: Option<estream_core::CanvasId>,
    /// Single-stream settings from before multistream; read once, migrated, never written.
    #[serde(rename = "stream", skip_serializing)]
    legacy_stream: Option<estream_media::StreamSettings>,
    #[serde(rename = "stream_key_protected", skip_serializing)]
    legacy_key: Option<String>,
}

/// One streaming destination.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DestinationConfig {
    pub name: String,
    pub enabled: bool,
    /// `stream.key` holds the key in memory only; it is saved encrypted in `key_protected`.
    pub stream: estream_media::StreamSettings,
    pub key_protected: String,
}

impl Default for DestinationConfig {
    fn default() -> Self {
        Self {
            name: "New destination".into(),
            enabled: true,
            stream: estream_media::StreamSettings::default(),
            key_protected: String::new(),
        }
    }
}

impl DestinationConfig {
    /// Sets the stream key (kept in memory, saved encrypted).
    pub fn set_key(&mut self, key: &str) {
        self.stream.key = key.trim().to_owned();
        self.key_protected = crate::secret::protect(&self.stream.key);
    }

    pub fn to_destination(&self) -> estream_media::Destination {
        estream_media::Destination {
            name: self.name.clone(),
            settings: self.stream.clone(),
        }
    }
}

/// Remote control (obs-websocket v5) server settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    /// Require the password (recommended; anyone on the network could connect otherwise).
    pub auth: bool,
    /// In memory only; saved encrypted in `password_protected`.
    #[serde(skip)]
    pub password: String,
    pub password_protected: String,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 4455,
            auth: true,
            password: String::new(),
            password_protected: String::new(),
        }
    }
}

impl RemoteSettings {
    pub fn set_password(&mut self, password: &str) {
        self.password = password.trim().to_owned();
        self.password_protected = crate::secret::protect(&self.password);
    }

    /// A random 16-character password (letters and digits, no look-alikes).
    pub fn generate_password(&mut self) {
        const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            bytes = std::array::from_fn(|_| fastrand::u8(..));
        }
        let password: String = bytes
            .iter()
            .map(|b| CHARS[*b as usize % CHARS.len()] as char)
            .collect();
        self.set_password(&password);
    }
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
            destinations: Vec::new(),
            remote: RemoteSettings::default(),
            vcam_canvas: None,
            legacy_stream: None,
            legacy_key: None,
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults (and logging) if missing or invalid.
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "invalid settings file; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        settings.migrate();
        for d in &mut settings.destinations {
            d.stream.key = crate::secret::unprotect(&d.key_protected);
        }
        settings.remote.password = crate::secret::unprotect(&settings.remote.password_protected);
        settings
    }

    /// Turns pre-multistream single-stream settings into the first destination.
    fn migrate(&mut self) {
        let legacy = self.legacy_stream.take();
        let key = self.legacy_key.take().unwrap_or_default();
        let Some(stream) = legacy else { return };
        if !self.destinations.is_empty() || stream.server.trim().is_empty() {
            return;
        }
        let name = match stream.service {
            estream_media::Service::Custom => "Stream".to_owned(),
            service => service.label().to_owned(),
        };
        tracing::info!(%name, "migrated stream settings to a destination");
        self.destinations.push(DestinationConfig {
            name,
            enabled: true,
            stream,
            key_protected: key,
        });
    }

    /// The enabled destinations, in order, with their settings indices.
    pub fn enabled_destinations(&self) -> Vec<(usize, estream_media::Destination)> {
        self.destinations
            .iter()
            .enumerate()
            .filter(|(_, d)| d.enabled)
            .map(|(i, d)| (i, d.to_destination()))
            .collect()
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

    #[test]
    fn stream_key_is_encrypted_on_disk() {
        let dir = std::env::temp_dir().join(format!("estream-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        let mut d = DestinationConfig::default();
        d.stream.server = "rtmp://example.com/app".into();
        d.set_key("live_42_secret");
        s.destinations.push(d);
        s.save(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("live_42_secret"), "{text}");
        let loaded = Settings::load(&path);
        if cfg!(windows) {
            assert_eq!(loaded.destinations[0].stream.key, "live_42_secret");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_stream_settings_migrate_to_a_destination() {
        let dir = std::env::temp_dir().join(format!("estream-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let protected = crate::secret::protect("old-key");
        let old = serde_json::json!({
            "theme": "dark",
            "stream": { "service": "twitch", "server": "rtmp://live.twitch.tv/app", "bitrate_kbps": 4500 },
            "stream_key_protected": protected,
        });
        std::fs::write(&path, old.to_string()).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.destinations.len(), 1);
        let d = &s.destinations[0];
        assert_eq!(d.name, "Twitch");
        assert!(d.enabled);
        assert_eq!(d.stream.server, "rtmp://live.twitch.tv/app");
        assert_eq!(d.stream.bitrate_kbps, 4500);
        if cfg!(windows) {
            assert_eq!(d.stream.key, "old-key");
        }
        // Saved in the new format only.
        s.save(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"destinations\""));
        assert!(!text.contains("\"stream_key_protected\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
