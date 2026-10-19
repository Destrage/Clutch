//! ScreenCast portal restore tokens, so the screen picker isn't shown at every start.
//!
//! Tokens are single-use: each session start returns the next one. Writing that into the
//! project would mark it unsaved and add an undo step at every start, and a token means
//! nothing on another machine, so they live in `portal-tokens.json` in Clutch's settings
//! folder, keyed by source id.

use estream_core::SourceId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static TOKENS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Where tokens are saved (Clutch's settings folder). Without it they last for the session.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn file() -> Option<PathBuf> {
    DIR.get().map(|d| d.join("portal-tokens.json"))
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
    let mut guard = TOKENS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(|| {
        file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    f(map)
}

pub fn get(source: SourceId) -> Option<String> {
    with(|m| m.get(&source.to_string()).cloned())
}

/// Saves (or with `None`, forgets) the source's token.
pub fn set(source: SourceId, token: Option<String>) {
    let text = with(|m| {
        match token {
            Some(t) => m.insert(source.to_string(), t),
            None => m.remove(&source.to_string()),
        };
        serde_json::to_string_pretty(m).unwrap_or_default()
    });
    if let Some(path) = file()
        && let Err(e) = std::fs::write(&path, text)
    {
        tracing::warn!(error = %e, path = %path.display(), "couldn't save screen-share tokens");
    }
}
