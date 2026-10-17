//! What the API needs from the app beyond the engine: studio mode, outputs and players.

use estream_core::{SceneId, SourceId};
use estream_engine::Engine;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Stream,
    Record,
    ReplayBuffer,
    VirtualCam,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputStatus {
    pub active: bool,
    pub reconnecting: bool,
    pub duration: Duration,
    pub bytes: u64,
    pub skipped_frames: u64,
    pub total_frames: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaState {
    None,
    Playing,
    Paused,
    Ended,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaStatus {
    pub state: MediaState,
    pub duration: Option<Duration>,
    pub position: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Stop,
    Restart,
    Next,
    Previous,
}

/// The app side of the API. Implemented by the studio app (and by a fake in tests).
pub trait Studio {
    /// The running engine (`None` while no collection is open).
    fn engine(&self) -> Option<&Engine>;
    fn collection_name(&self) -> String;
    fn record_directory(&self) -> PathBuf;

    fn studio_mode(&self) -> bool;
    fn set_studio_mode(&mut self, enabled: bool);
    /// The Preview scene (studio mode only).
    fn preview_scene(&self) -> Option<SceneId>;
    fn set_preview_scene(&mut self, scene: SceneId) -> Result<(), String>;
    /// Sends the Preview to Program with the current transition.
    fn trigger_studio_transition(&mut self) -> Result<(), String>;

    fn output_status(&self, output: Output) -> OutputStatus;
    /// Starts or stops an output; stopping a recording returns its file.
    fn set_output_active(
        &mut self,
        output: Output,
        active: bool,
    ) -> Result<Option<PathBuf>, String>;
    fn save_replay(&mut self) -> Result<PathBuf, String>;
    fn last_replay(&self) -> Option<PathBuf>;
    /// The file of the most recent finished recording.
    fn last_recording(&self) -> Option<PathBuf>;

    /// Playback state of a media source (`None` if it isn't one).
    fn media_status(&self, source: SourceId) -> Option<MediaStatus>;
    fn media_action(&mut self, source: SourceId, action: MediaAction) -> Result<(), String>;
    fn media_seek(&mut self, source: SourceId, position: Duration) -> Result<(), String>;
    /// A source's action button (e.g. a browser source's refresh).
    fn press_input_button(&mut self, source: SourceId, property: &str) -> Result<(), String>;

    /// Machine load for `GetStats`: (CPU %, memory MB, free disk MB on the recording drive).
    fn system_stats(&self) -> Option<(f64, f64, f64)> {
        None
    }

    /// Macro names (obs-websocket "hotkeys").
    fn macro_names(&self) -> Vec<String> {
        Vec::new()
    }

    /// Runs the macro with this name.
    fn run_macro(&mut self, name: &str) -> Result<(), String> {
        Err(format!("no macro named “{name}”"))
    }
}
