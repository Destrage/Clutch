//! App-level settings (not part of a scene collection): paths, appearance, window
//! and panel layout. Stored as JSON in the user's config directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Locations of Clutch's files.
#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

/// The app's settings folder name up to 0.2, when it was called eStream.
const LEGACY_NAME: &str = "eStream";

impl Paths {
    pub fn new() -> Self {
        let root = ["CLUTCH_HOME", "ESTREAM_HOME"]
            .into_iter()
            .find_map(std::env::var_os)
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|d| d.join("Clutch")))
            .unwrap_or_else(|| PathBuf::from(".clutch"));
        Self { root }
    }

    /// Clutch's settings folder, unless a home folder is set in the environment.
    pub fn legacy_root() -> Option<PathBuf> {
        if ["CLUTCH_HOME", "ESTREAM_HOME"]
            .into_iter()
            .any(|v| std::env::var_os(v).is_some())
        {
            return None;
        }
        dirs::config_dir().map(|d| d.join(LEGACY_NAME))
    }

    /// Moves the settings folder from `legacy` to `root`, once: only when `legacy` exists and
    /// `root` doesn't yet. Copies instead when the folder can't be moved (a file in it is
    /// open), leaving `legacy` as it is. Returns whether anything was taken over.
    pub fn adopt(&self, legacy: &Path) -> std::io::Result<bool> {
        if !legacy.is_dir() || self.root.exists() {
            return Ok(false);
        }
        if let Some(parent) = self.root.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::rename(legacy, &self.root).is_err() {
            copy_dir(legacy, &self.root)?;
        }
        Ok(true)
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

/// Copies a folder tree (skipping files that can't be read, like a held lock file).
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if let Err(e) = std::fs::copy(entry.path(), &target) {
            tracing::warn!(file = %entry.path().display(), error = %e, "not copied");
        }
    }
    Ok(())
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
    /// Clip mode, the clip hotkey, auto-clip and the clip library.
    pub clips: ClipSettings,
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

/// When to clip a game event automatically, and how much around it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoRule {
    /// The game's name (as in the games table).
    pub game: String,
    pub event: estream_project::GameEventKind,
    pub enabled: bool,
    /// Seconds before and after the event.
    pub pre: u32,
    pub post: u32,
}

impl Default for AutoRule {
    fn default() -> Self {
        Self {
            game: String::new(),
            event: estream_project::GameEventKind::Kill,
            enabled: true,
            pre: 10,
            post: 3,
        }
    }
}

/// The default window around an event kind (from the add-clips design).
pub fn default_window(event: estream_project::GameEventKind) -> (u32, u32) {
    use estream_project::GameEventKind as K;
    match event {
        K::Kill | K::Death => (10, 3),
        K::MultiKill | K::Ace => (15, 4),
        K::RoundWin | K::MatchWin => (15, 5),
        K::Objective => (12, 3),
        K::Hype => (12, 4),
    }
}

/// The event kinds each integrated game reports.
pub fn integration_events(
    integration: estream_clips::Integration,
) -> &'static [estream_project::GameEventKind] {
    use estream_clips::Integration as I;
    use estream_project::GameEventKind as K;
    match integration {
        I::Cs2Gsi => &[
            K::Kill,
            K::MultiKill,
            K::Ace,
            K::RoundWin,
            K::MatchWin,
            K::Death,
        ],
        I::Dota2Gsi => &[K::Kill, K::MultiKill, K::MatchWin, K::Death],
        I::LeagueLiveClient => &[K::Kill, K::MultiKill, K::Ace, K::Objective, K::MatchWin],
    }
}

/// Clips: clip mode, the clip hotkey, auto-clip and the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipSettings {
    /// Start the replay buffer by itself while a game runs.
    pub mode: bool,
    /// Replay buffer length in clip mode (seconds).
    pub buffer_seconds: u32,
    /// The clip hotkey, e.g. "F8" (empty = none).
    pub hotkey: String,
    /// Seconds the clip hotkey saves.
    pub hotkey_seconds: u32,
    /// The longest clip merged events may make.
    pub max_seconds: u32,
    /// The clips folder (`None` = `Clips` in the recording folder).
    pub folder: Option<PathBuf>,
    /// Play a sound when a clip is saved.
    pub sound: bool,
    /// Automatic clips beyond this many GB are deleted, oldest first.
    pub storage_cap_gb: f32,
    /// Integrations the user connected (the listener runs only for these).
    pub connected: Vec<estream_clips::Integration>,
    pub rules: Vec<AutoRule>,
    pub hype_game: bool,
    pub hype_mic: bool,
    pub hype_sensitivity: estream_clips::Sensitivity,
    /// Port the game state integrations post to.
    pub gsi_port: u16,
    pub cs2_token: String,
    pub dota2_token: String,
    pub user_games: Vec<estream_clips::games::UserGame>,
    pub not_games: Vec<String>,
}

impl Default for ClipSettings {
    fn default() -> Self {
        let mut s = Self {
            mode: false,
            buffer_seconds: 120,
            hotkey: "F8".into(),
            hotkey_seconds: 30,
            max_seconds: 90,
            folder: None,
            sound: true,
            storage_cap_gb: 20.0,
            connected: Vec::new(),
            rules: Vec::new(),
            hype_game: false,
            hype_mic: false,
            hype_sensitivity: estream_clips::Sensitivity::Medium,
            gsi_port: estream_clips::integrations::gsi::DEFAULT_PORT,
            cs2_token: String::new(),
            dota2_token: String::new(),
            user_games: Vec::new(),
            not_games: Vec::new(),
        };
        s.fill_rules();
        s
    }
}

impl ClipSettings {
    /// Adds a rule (with the default window) for every event an integrated game reports
    /// and that has none yet. Deaths start off.
    pub fn fill_rules(&mut self) {
        use estream_clips::Integration as I;
        for (integration, game) in [
            (I::Cs2Gsi, "Counter-Strike 2"),
            (I::Dota2Gsi, "Dota 2"),
            (I::LeagueLiveClient, "League of Legends"),
        ] {
            for &event in integration_events(integration) {
                if !self
                    .rules
                    .iter()
                    .any(|r| r.game == game && r.event == event)
                {
                    let (pre, post) = default_window(event);
                    self.rules.push(AutoRule {
                        game: game.into(),
                        event,
                        enabled: event != estream_project::GameEventKind::Death,
                        pre,
                        post,
                    });
                }
            }
        }
    }

    /// The rule for `event` in `game`, if it is enabled.
    pub fn rule(&self, game: &str, event: estream_project::GameEventKind) -> Option<&AutoRule> {
        self.rules
            .iter()
            .find(|r| r.enabled && r.event == event && r.game.eq_ignore_ascii_case(game))
    }

    pub fn folder(&self, recording: &estream_media::RecordingSettings) -> PathBuf {
        self.folder
            .clone()
            .unwrap_or_else(|| recording.folder.join("Clips"))
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
            clips: ClipSettings::default(),
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

    /// Points saved paths inside the old settings folder at the new one (after
    /// [`Paths::adopt`]). Returns whether anything changed.
    pub fn rebase(&mut self, from: &Path, to: &Path) -> bool {
        let mut changed = false;
        for path in [&mut self.last_collection, &mut self.clips.folder]
            .into_iter()
            .flatten()
        {
            if let Ok(rest) = path.strip_prefix(from) {
                *path = to.join(rest);
                changed = true;
            }
        }
        changed
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
///
/// Read with the registry API, not `reg.exe`: this runs every few seconds on the UI thread,
/// and launching a console program from the windowed app flashed a console each time.
pub fn system_prefers_dark() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
        use windows::core::w;
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: `value` and `size` outlive the call; `size` is the buffer length in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut value).cast()),
                Some(&mut size),
            )
        };
        if status.is_ok() {
            return value == 0;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry read agrees with what `reg.exe` reports.
    #[cfg(windows)]
    #[test]
    fn system_theme_matches_the_registry() {
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        if text.contains("AppsUseLightTheme") {
            assert_eq!(system_prefers_dark(), !text.contains("0x1"));
        }
    }

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
    fn estream_folder_is_adopted_once() {
        let dir = std::env::temp_dir().join(format!("clutch-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let legacy = dir.join("eStream");
        std::fs::create_dir_all(legacy.join("scenes")).unwrap();
        std::fs::write(legacy.join("scenes/Stream.json"), "{}").unwrap();
        let s = Settings {
            last_collection: Some(legacy.join("scenes/Stream.json")),
            ..Settings::default()
        };
        s.save(&legacy.join("settings.json"));

        let paths = Paths {
            root: dir.join("Clutch"),
        };
        assert!(paths.adopt(&legacy).unwrap());
        assert!(!legacy.exists());
        assert!(paths.scenes().join("Stream.json").exists());
        let mut loaded = Settings::load(&paths.settings());
        assert!(loaded.rebase(&legacy, &paths.root));
        assert_eq!(
            loaded.last_collection,
            Some(paths.scenes().join("Stream.json"))
        );

        // Never again once the new folder exists.
        std::fs::create_dir_all(&legacy).unwrap();
        assert!(!paths.adopt(&legacy).unwrap());
        assert!(legacy.exists());
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
