//! Game events from replay files that games write to the local disk.
//!
//! Reading files the game has already written never touches the game process, so it is safe
//! with anti-cheat. A [`ReplayWatcher`] polls a game's replay folder on its own thread, parses
//! each new replay with its [`ReplayFormat`] and reports the local player's events, stamped
//! at the moment they happened (usually in the past: games write replays when a match ends).

pub mod pubg;

use super::GameEvent;
use crate::event::EventKind;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const POLL: Duration = Duration::from_secs(2);
/// A replay still unreadable after this many polls is given up.
const MAX_ATTEMPTS: u32 = 30;

/// One event in a replay, on the replay's own clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayEvent {
    pub kind: EventKind,
    /// Time from the start of the replay.
    pub match_time: Duration,
}

/// The local player's events in one replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedReplay {
    /// When the replay started (wall clock).
    pub started: SystemTime,
    pub events: Vec<ReplayEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// The game is still writing it; try again on the next poll.
    #[error("the replay isn't finished yet")]
    NotReady,
    /// A format this parser doesn't know, e.g. after a game update.
    #[error("unsupported replay format ({0})")]
    Unsupported(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// How one game stores its replays.
pub trait ReplayFormat: Send + 'static {
    /// The game's name, as in `games.json`.
    fn game(&self) -> &'static str;
    /// The folder holding the replays, if this system has one.
    fn folder(&self) -> Option<PathBuf>;
    /// The replays (one path per match) currently in `folder`.
    fn replays(&self, folder: &Path) -> Vec<PathBuf>;
    /// Reads the local player's events from a replay.
    fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError>;
}

/// Turns a wall-clock moment into an `Instant` (clamped to now for future times).
pub fn instant_of(wall: SystemTime, now: Instant, now_wall: SystemTime) -> Instant {
    match now_wall.duration_since(wall) {
        Ok(ago) => now.checked_sub(ago).unwrap_or(now),
        Err(_) => now,
    }
}

/// Watches a game's replay folder on its own thread while the game runs. Replays already
/// there when it starts are ignored. Dropping it does a last scan (the game may have just
/// written the final match) and stops the thread.
pub struct ReplayWatcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReplayWatcher {
    /// Starts watching the format's own folder. `None` if this system has no such folder
    /// location at all (e.g. the game doesn't exist on this platform).
    pub fn start(
        format: Box<dyn ReplayFormat>,
        events: Sender<GameEvent>,
    ) -> Option<std::io::Result<Self>> {
        let folder = format.folder()?;
        Some(Self::start_in(format, folder, events))
    }

    /// Watches `folder` (for tests and custom locations).
    pub fn start_in(
        format: Box<dyn ReplayFormat>,
        folder: PathBuf,
        events: Sender<GameEvent>,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("estream-replay-watch".into())
                .spawn(move || Watch::new(format, folder, events).run(&stop))?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ReplayWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Watch {
    format: Box<dyn ReplayFormat>,
    folder: PathBuf,
    events: Sender<GameEvent>,
    done: HashSet<PathBuf>,
    attempts: HashMap<PathBuf, u32>,
    unsupported: HashSet<String>,
}

impl Watch {
    fn new(format: Box<dyn ReplayFormat>, folder: PathBuf, events: Sender<GameEvent>) -> Self {
        let done = format.replays(&folder).into_iter().collect();
        Self {
            format,
            folder,
            events,
            done,
            attempts: HashMap::new(),
            unsupported: HashSet::new(),
        }
    }

    fn run(mut self, stop: &AtomicBool) {
        loop {
            let stopping = stop.load(Ordering::Acquire);
            self.scan();
            if stopping {
                return;
            }
            let until = Instant::now() + POLL;
            while Instant::now() < until && !stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    fn scan(&mut self) {
        let game = self.format.game();
        for replay in self.format.replays(&self.folder) {
            if self.done.contains(&replay) {
                continue;
            }
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.format.parse(&replay)
            }));
            match parsed {
                Ok(Ok(parsed)) => {
                    tracing::info!(game, events = parsed.events.len(), replay = %replay.display(), "replay read");
                    self.send(&parsed);
                    self.done.insert(replay);
                }
                Ok(Err(ReplayError::NotReady | ReplayError::Io(_))) => {
                    let n = self.attempts.entry(replay.clone()).or_default();
                    *n += 1;
                    if *n >= MAX_ATTEMPTS {
                        tracing::warn!(game, replay = %replay.display(), "replay never became readable");
                        self.done.insert(replay);
                    }
                }
                Ok(Err(ReplayError::Unsupported(what))) => {
                    if self.unsupported.insert(what.clone()) {
                        tracing::warn!(game, format = %what, "replay format not supported");
                    }
                    self.done.insert(replay);
                }
                Err(_) => {
                    tracing::error!(game, replay = %replay.display(), "replay parser panicked");
                    self.done.insert(replay);
                }
            }
        }
    }

    fn send(&self, parsed: &ParsedReplay) {
        let (now, now_wall) = (Instant::now(), SystemTime::now());
        for e in &parsed.events {
            let at = instant_of(parsed.started + e.match_time, now, now_wall);
            let _ = self.events.send(GameEvent {
                game: self.format.game().to_owned(),
                kind: e.kind.clone(),
                at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::channel;

    /// Replays are the files in the folder; each parses to one kill 5 s into the match.
    struct FakeFormat {
        parsed: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl ReplayFormat for FakeFormat {
        fn game(&self) -> &'static str {
            "Fake"
        }
        fn folder(&self) -> Option<PathBuf> {
            None
        }
        fn replays(&self, folder: &Path) -> Vec<PathBuf> {
            std::fs::read_dir(folder)
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default()
        }
        fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError> {
            self.parsed.lock().unwrap().push(replay.to_owned());
            match std::fs::read_to_string(replay)?.as_str() {
                "partial" => Err(ReplayError::NotReady),
                "v99" => Err(ReplayError::Unsupported("v99".into())),
                "panic" => panic!("bad replay"),
                _ => Ok(ParsedReplay {
                    started: SystemTime::now() - Duration::from_secs(60),
                    events: vec![ReplayEvent {
                        kind: EventKind::Kill,
                        match_time: Duration::from_secs(5),
                    }],
                }),
            }
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("estream-replay-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reports_new_replays_only_and_skips_bad_ones() {
        let dir = folder("watch");
        std::fs::write(dir.join("old"), "ok").unwrap();
        let parsed = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel();
        let mut watch = Watch::new(
            Box::new(FakeFormat {
                parsed: parsed.clone(),
            }),
            dir.clone(),
            tx,
        );
        std::fs::write(dir.join("new"), "ok").unwrap();
        std::fs::write(dir.join("future"), "v99").unwrap();
        std::fs::write(dir.join("broken"), "panic").unwrap();
        std::fs::write(dir.join("growing"), "partial").unwrap();
        watch.scan();
        watch.scan();

        let events: Vec<GameEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 1, "one kill from the new replay");
        assert_eq!(events[0].kind, EventKind::Kill);
        // The kill happened 55 s ago (started 60 s ago, 5 s in).
        let ago = Instant::now().duration_since(events[0].at);
        assert!(
            ago >= Duration::from_secs(54) && ago <= Duration::from_secs(57),
            "{ago:?}"
        );

        let parsed = parsed.lock().unwrap();
        let count = |name: &str| parsed.iter().filter(|p| p.ends_with(name)).count();
        assert_eq!(
            count("old"),
            0,
            "replays there before the watcher are ignored"
        );
        assert_eq!(count("new"), 1);
        assert_eq!(count("future"), 1, "unsupported replays are not retried");
        assert_eq!(count("broken"), 1, "a panicking parser is not retried");
        assert_eq!(count("growing"), 2, "unfinished replays are retried");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_times_clamp_to_now() {
        let (now, wall) = (Instant::now(), SystemTime::now());
        assert_eq!(instant_of(wall + Duration::from_secs(5), now, wall), now);
        assert_eq!(
            instant_of(wall - Duration::from_secs(5), now, wall),
            now - Duration::from_secs(5)
        );
    }
}
