//! Knowing when a game is running.
//!
//! [`GameList`] says whether a program is a game: the built-in table of known games, plus the
//! programs the user marked as games or as not games. [`Detector`] watches the foreground
//! window and turns what it sees into "game started" and "game stopped" edges. It is pure
//! logic; the app feeds it the foreground window and a way to check that a process is alive
//! (see [`crate::platform`]).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// A game's local event API that Clutch can listen to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integration {
    /// Counter-Strike 2 Game State Integration.
    Cs2Gsi,
    /// Dota 2 Game State Integration.
    Dota2Gsi,
    /// League of Legends Live Client Data API.
    LeagueLiveClient,
    /// PUBG: Battlegrounds replay files (events arrive when the player's match ends).
    PubgReplay,
    /// Fortnite replay files (events arrive when the match ends).
    FortniteReplay,
    /// Rocket League replay files (saved when the match ends, with auto-save on).
    RocketLeagueReplay,
}

impl Integration {
    /// Whether the game's events come from its replay files (after a match), not live.
    pub fn is_replay(self) -> bool {
        matches!(
            self,
            Self::PubgReplay | Self::FortniteReplay | Self::RocketLeagueReplay
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct KnownGame {
    pub name: String,
    pub exe: Vec<String>,
    #[serde(default)]
    pub steam_app: Option<u32>,
    #[serde(default)]
    pub integration: Option<Integration>,
}

/// A program the user marked as a game.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserGame {
    pub exe: String,
    pub name: String,
}

/// A recognised game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub name: String,
    pub exe: String,
    pub integration: Option<Integration>,
    pub steam_app: Option<u32>,
}

/// Programs that are never offered as games even when fullscreen (video players, browsers,
/// the desktop, Clutch itself).
const NEVER_GAMES: &[&str] = &[
    "clutch.exe",
    "estream.exe",
    "explorer.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "vlc.exe",
    "mpv.exe",
    "mpc-hc64.exe",
    "potplayermini64.exe",
    "applicationframehost.exe",
    "powerpnt.exe",
    "steamwebhelper.exe",
    "discord.exe",
    "obs64.exe",
    "code.exe",
    "lockapp.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
];

/// Which programs are games.
#[derive(Clone, Debug)]
pub struct GameList {
    known: Vec<KnownGame>,
    pub user_games: Vec<UserGame>,
    /// Lower-case exe names the user said are not games.
    pub not_games: Vec<String>,
}

impl Default for GameList {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl GameList {
    pub fn new(user_games: Vec<UserGame>, not_games: Vec<String>) -> Self {
        Self {
            known: known_games(),
            user_games,
            not_games: not_games.into_iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    pub fn known(&self) -> &[KnownGame] {
        &self.known
    }

    /// The game `exe` (a file name, any case) is, if it is one.
    pub fn lookup(&self, exe: &str) -> Option<Game> {
        let lower = exe.to_lowercase();
        if self.not_games.contains(&lower) {
            return None;
        }
        if let Some(u) = self
            .user_games
            .iter()
            .find(|u| u.exe.to_lowercase() == lower)
        {
            return Some(Game {
                name: u.name.clone(),
                exe: exe.to_owned(),
                integration: None,
                steam_app: None,
            });
        }
        self.known
            .iter()
            .find(|g| g.exe.iter().any(|e| e.to_lowercase() == lower))
            .map(|g| Game {
                name: g.name.clone(),
                exe: exe.to_owned(),
                integration: g.integration,
                steam_app: g.steam_app,
            })
    }

    /// Whether a fullscreen `exe` that isn't a known game should be offered as one.
    pub fn may_ask(&self, exe: &str) -> bool {
        let lower = exe.to_lowercase();
        !lower.is_empty()
            && !NEVER_GAMES.contains(&lower.as_str())
            && !self.not_games.contains(&lower)
            && self.lookup(exe).is_none()
    }
}

/// The built-in known-games table.
pub fn known_games() -> Vec<KnownGame> {
    serde_json::from_str(include_str!("games.json")).unwrap_or_else(|e| {
        tracing::error!(error = %e, "the built-in games table is invalid");
        Vec::new()
    })
}

/// The foreground window, as the detector sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    pub pid: u32,
    /// Program file name, e.g. `cs2.exe`.
    pub program: String,
    pub title: String,
    /// The window covers its whole monitor (fullscreen or borderless).
    pub fullscreen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    Started(Game),
    Stopped(Game),
    /// A fullscreen program that isn't on any list: ask the user once whether it's a game.
    AskIsGame {
        exe: String,
        title: String,
    },
}

/// Turns foreground observations into game start/stop edges.
#[derive(Debug)]
pub struct Detector {
    current: Option<Running>,
    asked: HashSet<String>,
    grace: Duration,
}

#[derive(Debug)]
struct Running {
    game: Game,
    pid: u32,
    exited_at: Option<Instant>,
}

impl Default for Detector {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

impl Detector {
    /// `grace`: how long after the game's process exits it counts as stopped.
    pub fn new(grace: Duration) -> Self {
        Self {
            current: None,
            asked: HashSet::new(),
            grace,
        }
    }

    /// The game running now, if any.
    pub fn current(&self) -> Option<&Game> {
        self.current.as_ref().map(|r| &r.game)
    }

    /// Call every few hundred milliseconds. `alive(pid)` says whether a process still runs.
    pub fn update(
        &mut self,
        list: &GameList,
        now: Instant,
        foreground: Option<&Foreground>,
        alive: &dyn Fn(u32) -> bool,
    ) -> Vec<Detection> {
        let mut out = Vec::new();
        if let Some(run) = &mut self.current {
            if alive(run.pid) {
                run.exited_at = None;
            } else {
                let exited = *run.exited_at.get_or_insert(now);
                // A different game in front ends this one at once; otherwise wait out the
                // grace period (some games restart their process on launch).
                let other_game = foreground
                    .and_then(|f| list.lookup(&f.program))
                    .is_some_and(|g| g.name != run.game.name);
                if other_game || now.duration_since(exited) >= self.grace {
                    out.push(Detection::Stopped(run.game.clone()));
                    self.current = None;
                }
            }
        }
        let Some(fg) = foreground else {
            return out;
        };
        match list.lookup(&fg.program) {
            Some(game) => match &mut self.current {
                None => {
                    self.current = Some(Running {
                        game: game.clone(),
                        pid: fg.pid,
                        exited_at: None,
                    });
                    out.push(Detection::Started(game));
                }
                // The same game came back with a new process (launcher restart): keep going.
                Some(run) if run.game.name == game.name => {
                    run.pid = fg.pid;
                    run.exited_at = None;
                }
                // Another game while one is still running: the first one stays current.
                Some(_) => {}
            },
            None => {
                if fg.fullscreen
                    && list.may_ask(&fg.program)
                    && self.asked.insert(fg.program.to_lowercase())
                {
                    out.push(Detection::AskIsGame {
                        exe: fg.program.clone(),
                        title: fg.title.clone(),
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;

    fn fg(pid: u32, program: &str, fullscreen: bool) -> Foreground {
        Foreground {
            pid,
            program: program.into(),
            title: program.trim_end_matches(".exe").into(),
            fullscreen,
        }
    }

    struct World {
        alive: RefCell<HashSet<u32>>,
    }

    impl World {
        fn new(pids: &[u32]) -> Self {
            Self {
                alive: RefCell::new(pids.iter().copied().collect()),
            }
        }
        fn alive(&self) -> impl Fn(u32) -> bool + '_ {
            |pid| self.alive.borrow().contains(&pid)
        }
        fn kill(&self, pid: u32) {
            self.alive.borrow_mut().remove(&pid);
        }
    }

    #[test]
    fn the_table_loads_and_has_the_integrated_games() {
        let list = GameList::default();
        assert!(list.known().len() > 80, "{}", list.known().len());
        let cs = list.lookup("CS2.EXE").unwrap_or_else(|| panic!("cs2"));
        assert_eq!(cs.name, "Counter-Strike 2");
        assert_eq!(cs.integration, Some(Integration::Cs2Gsi));
        assert_eq!(cs.steam_app, Some(730));
        assert_eq!(
            list.lookup("League of Legends.exe").map(|g| g.integration),
            Some(Some(Integration::LeagueLiveClient))
        );
        // No exe appears twice.
        let mut seen = HashSet::new();
        for g in list.known() {
            for e in &g.exe {
                assert!(seen.insert(e.to_lowercase()), "{e} listed twice");
            }
        }
    }

    #[test]
    fn user_lists_override_the_table() {
        let list = GameList::new(
            vec![UserGame {
                exe: "MyIndie.exe".into(),
                name: "My Indie".into(),
            }],
            vec!["Cs2.exe".into()],
        );
        assert_eq!(
            list.lookup("myindie.exe").map(|g| g.name),
            Some("My Indie".into())
        );
        assert!(list.lookup("cs2.exe").is_none(), "marked not a game");
        assert!(!list.may_ask("cs2.exe"));
        assert!(!list.may_ask("chrome.exe"));
        assert!(list.may_ask("unknown.exe"));
    }

    #[test]
    fn starts_and_stops_after_the_grace_period() {
        let list = GameList::default();
        let world = World::new(&[10]);
        let mut d = Detector::new(Duration::from_secs(10));
        let t0 = Instant::now();
        let out = d.update(&list, t0, Some(&fg(10, "cs2.exe", true)), &world.alive());
        assert!(matches!(&out[..], [Detection::Started(g)] if g.name == "Counter-Strike 2"));
        // Alt-tab away: still running.
        let desk = fg(1, "explorer.exe", false);
        assert!(
            d.update(
                &list,
                t0 + Duration::from_secs(5),
                Some(&desk),
                &world.alive()
            )
            .is_empty()
        );
        assert!(d.current().is_some());
        world.kill(10);
        assert!(
            d.update(
                &list,
                t0 + Duration::from_secs(6),
                Some(&desk),
                &world.alive()
            )
            .is_empty()
        );
        assert!(
            d.update(
                &list,
                t0 + Duration::from_secs(15),
                Some(&desk),
                &world.alive()
            )
            .is_empty()
        );
        let out = d.update(
            &list,
            t0 + Duration::from_secs(16),
            Some(&desk),
            &world.alive(),
        );
        assert!(matches!(&out[..], [Detection::Stopped(_)]));
        assert!(d.current().is_none());
    }

    #[test]
    fn a_restarted_process_keeps_the_game_running() {
        let list = GameList::default();
        let world = World::new(&[10]);
        let mut d = Detector::default();
        let t0 = Instant::now();
        d.update(&list, t0, Some(&fg(10, "cs2.exe", true)), &world.alive());
        world.kill(10);
        world.alive.borrow_mut().insert(11);
        let out = d.update(
            &list,
            t0 + Duration::from_secs(2),
            Some(&fg(11, "cs2.exe", true)),
            &world.alive(),
        );
        assert!(out.is_empty(), "{out:?}");
        let out = d.update(&list, t0 + Duration::from_secs(30), None, &world.alive());
        assert!(out.is_empty(), "the new process is alive: {out:?}");
    }

    #[test]
    fn another_game_after_exit_switches_at_once() {
        let list = GameList::default();
        let world = World::new(&[10, 20]);
        let mut d = Detector::default();
        let t0 = Instant::now();
        d.update(&list, t0, Some(&fg(10, "cs2.exe", true)), &world.alive());
        world.kill(10);
        let out = d.update(
            &list,
            t0 + Duration::from_secs(1),
            Some(&fg(20, "dota2.exe", true)),
            &world.alive(),
        );
        assert!(
            matches!(&out[..], [Detection::Stopped(a), Detection::Started(b)] if a.name == "Counter-Strike 2" && b.name == "Dota 2"),
            "{out:?}"
        );
    }

    #[test]
    fn unknown_fullscreen_programs_are_asked_about_once() {
        let list = GameList::default();
        let world = World::new(&[30, 40]);
        let mut d = Detector::default();
        let t0 = Instant::now();
        let out = d.update(&list, t0, Some(&fg(30, "Indie.exe", true)), &world.alive());
        assert!(matches!(&out[..], [Detection::AskIsGame { exe, .. }] if exe == "Indie.exe"));
        assert!(
            d.update(&list, t0, Some(&fg(30, "indie.exe", true)), &world.alive())
                .is_empty()
        );
        // Windowed, or a fullscreen browser: never asked.
        assert!(
            d.update(&list, t0, Some(&fg(40, "Tool.exe", false)), &world.alive())
                .is_empty()
        );
        assert!(
            d.update(&list, t0, Some(&fg(40, "chrome.exe", true)), &world.alive())
                .is_empty()
        );
        assert!(d.current().is_none());
    }
}
