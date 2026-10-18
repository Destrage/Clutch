//! Valve Game State Integration (Counter-Strike 2, Dota 2).
//!
//! The game POSTs its state as JSON to a local HTTP endpoint named in a config file in the
//! game's `cfg` folder. Each payload carries the token from that file in `auth.token`;
//! anything without it is rejected. Events come from comparing a payload with the previous
//! one for the same player, and only the local player (`provider.steamid`) counts, so
//! spectating someone else never clips.

use super::{GameEvent, steam};
use crate::event::EventKind;
use crate::games::Integration;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The port tried first; the next few are tried if it's taken.
pub const DEFAULT_PORT: u16 = 3091;
/// The config file name written into the game's `cfg` folder.
pub const CONFIG_NAME: &str = "gamestate_integration_estream.cfg";

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

fn u64_at(v: &Value, path: &[&str]) -> Option<u64> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_u64()
}

/// The local player's counters in the last payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counters {
    kills: u64,
    deaths: u64,
}

/// Counter-Strike 2 payloads → events.
#[derive(Debug, Default)]
pub struct Cs2Tracker {
    own: Option<Counters>,
    round_phase: Option<String>,
    map_phase: Option<String>,
}

impl Cs2Tracker {
    pub fn update(&mut self, payload: &Value) -> Vec<EventKind> {
        let mut events = Vec::new();
        let me = str_at(payload, &["provider", "steamid"]);
        let player = str_at(payload, &["player", "steamid"]);
        let own = me.is_some() && me == player;
        let team = str_at(payload, &["player", "team"]).unwrap_or_default();

        if own
            && let (Some(kills), Some(deaths)) = (
                u64_at(payload, &["player", "match_stats", "kills"]),
                u64_at(payload, &["player", "match_stats", "deaths"]),
            )
        {
            let now = Counters { kills, deaths };
            if let Some(before) = self.own
                && now.kills >= before.kills
                && now.deaths >= before.deaths
            {
                let round_kills = u64_at(payload, &["player", "state", "round_kills"]).unwrap_or(0);
                for _ in before.kills..now.kills {
                    events.push(match round_kills {
                        n if n >= 5 => EventKind::Ace,
                        n if n >= 2 => EventKind::MultiKill { n: n as u8 },
                        _ => EventKind::Kill,
                    });
                }
                if now.deaths > before.deaths {
                    events.push(EventKind::Death);
                }
            }
            // Counters going down mean a new match: take them as the new baseline.
            self.own = Some(now);
        }

        let round_phase = str_at(payload, &["round", "phase"]).map(str::to_owned);
        if round_phase.as_deref() == Some("over")
            && self.round_phase.as_deref() != Some("over")
            && !team.is_empty()
            && str_at(payload, &["round", "win_team"]) == Some(team)
        {
            events.push(EventKind::RoundWin);
        }
        if round_phase.is_some() {
            self.round_phase = round_phase;
        }

        let map_phase = str_at(payload, &["map", "phase"]).map(str::to_owned);
        if map_phase.as_deref() == Some("gameover") && self.map_phase.as_deref() != Some("gameover")
        {
            let score = |side: &str| u64_at(payload, &["map", side, "score"]).unwrap_or(0);
            let (ct, t) = (score("team_ct"), score("team_t"));
            let won = match team {
                "CT" => ct > t,
                "T" => t > ct,
                _ => false,
            };
            if won {
                events.push(EventKind::MatchWin);
            }
        }
        if map_phase.is_some() {
            self.map_phase = map_phase;
        }
        events
    }
}

/// Dota 2 payloads → events. Kills within 18 s of each other count up a multi-kill.
#[derive(Debug, Default)]
pub struct Dota2Tracker {
    own: Option<Counters>,
    recent_kills: VecDeque<Instant>,
    game_state: Option<String>,
}

const DOTA_MULTI_KILL: Duration = Duration::from_secs(18);

impl Dota2Tracker {
    pub fn update(&mut self, payload: &Value, now: Instant) -> Vec<EventKind> {
        let mut events = Vec::new();
        let me = str_at(payload, &["provider", "steamid"]);
        let player = str_at(payload, &["player", "steamid"]);
        let own = me.is_some() && me == player;
        if own
            && let (Some(kills), Some(deaths)) = (
                u64_at(payload, &["player", "kills"]),
                u64_at(payload, &["player", "deaths"]),
            )
        {
            let counters = Counters { kills, deaths };
            if let Some(before) = self.own
                && counters.kills >= before.kills
                && counters.deaths >= before.deaths
            {
                for _ in before.kills..counters.kills {
                    while self
                        .recent_kills
                        .front()
                        .is_some_and(|t| now.duration_since(*t) > DOTA_MULTI_KILL)
                    {
                        self.recent_kills.pop_front();
                    }
                    self.recent_kills.push_back(now);
                    events.push(match self.recent_kills.len() {
                        1 => EventKind::Kill,
                        n => EventKind::MultiKill {
                            n: n.min(255) as u8,
                        },
                    });
                }
                if counters.deaths > before.deaths {
                    events.push(EventKind::Death);
                    self.recent_kills.clear();
                }
            }
            self.own = Some(counters);
        }
        let state = str_at(payload, &["map", "game_state"]).map(str::to_owned);
        const POST: &str = "DOTA_GAMERULES_STATE_POST_GAME";
        if state.as_deref() == Some(POST)
            && self.game_state.as_deref() != Some(POST)
            && own
            && str_at(payload, &["map", "win_team"]).is_some()
            && str_at(payload, &["map", "win_team"]) == str_at(payload, &["player", "team_name"])
        {
            events.push(EventKind::MatchWin);
        }
        if state.is_some() {
            self.game_state = state;
        }
        events
    }
}

/// The integration's config file text.
pub fn config_text(integration: Integration, port: u16, token: &str) -> String {
    let (route, data) = match integration {
        Integration::Cs2Gsi => (
            "cs2",
            [
                "provider",
                "map",
                "round",
                "player_id",
                "player_state",
                "player_match_stats",
            ]
            .as_slice(),
        ),
        Integration::Dota2Gsi => ("dota2", ["provider", "map", "player"].as_slice()),
        Integration::LeagueLiveClient => ("", [].as_slice()),
    };
    let data: String = data
        .iter()
        .map(|d| format!("        \"{d}\" \"1\"\n"))
        .collect();
    format!(
        "\"eStream\"\n{{\n    \"uri\" \"http://127.0.0.1:{port}/{route}\"\n    \"timeout\" \"1.1\"\n    \"buffer\" \"0.1\"\n    \"throttle\" \"0.1\"\n    \"heartbeat\" \"10.0\"\n    \"auth\"\n    {{\n        \"token\" \"{token}\"\n    }}\n    \"data\"\n    {{\n{data}    }}\n}}\n"
    )
}

/// The folder the game reads integration configs from, under its install folder.
pub fn config_dir(integration: Integration, install: &Path) -> Option<PathBuf> {
    match integration {
        Integration::Cs2Gsi => Some(install.join("game").join("csgo").join("cfg")),
        Integration::Dota2Gsi => Some(
            install
                .join("game")
                .join("dota")
                .join("cfg")
                .join("gamestate_integration"),
        ),
        Integration::LeagueLiveClient => None,
    }
}

fn steam_app(integration: Integration) -> Option<u32> {
    match integration {
        Integration::Cs2Gsi => Some(730),
        Integration::Dota2Gsi => Some(570),
        Integration::LeagueLiveClient => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("Steam wasn't found on this PC")]
    NoSteam,
    #[error("the game isn't installed through Steam")]
    NotInstalled,
    #[error("this game doesn't use a config file")]
    NoConfig,
    #[error("couldn't write the config file: {0}")]
    Io(String),
}

/// Writes the integration's config into the game's folder (found through Steam). Only ever
/// called from an explicit user action ("Connect"). Returns the file written.
pub fn install(
    integration: Integration,
    steam_root: Option<&Path>,
    port: u16,
    token: &str,
) -> Result<PathBuf, InstallError> {
    let app = steam_app(integration).ok_or(InstallError::NoConfig)?;
    let root = match steam_root {
        Some(r) => r.to_path_buf(),
        None => steam::steam_root().ok_or(InstallError::NoSteam)?,
    };
    let install = steam::app_install_dir(&root, app).ok_or(InstallError::NotInstalled)?;
    let dir = config_dir(integration, &install).ok_or(InstallError::NoConfig)?;
    std::fs::create_dir_all(&dir).map_err(|e| InstallError::Io(e.to_string()))?;
    let path = dir.join(CONFIG_NAME);
    std::fs::write(&path, config_text(integration, port, token))
        .map_err(|e| InstallError::Io(e.to_string()))?;
    Ok(path)
}

/// A new random token for a config file.
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Not expected on any supported OS; fall back to a time-based value.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = t.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A token per integration; payloads with another token are rejected.
pub type Tokens = HashMap<Integration, String>;

/// The local HTTP listener the games post to.
pub struct GsiServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    last_seen: Arc<Mutex<HashMap<Integration, Instant>>>,
}

impl GsiServer {
    /// Listens on 127.0.0.1, trying `port` and the next 9 ports. Events go to `events`.
    pub fn start(port: u16, tokens: Tokens, events: Sender<GameEvent>) -> std::io::Result<Self> {
        let listener = (port..port.saturating_add(10))
            .find_map(|p| TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], p))).ok())
            .ok_or_else(|| std::io::Error::other("no free port for game integrations"))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let last_seen = Arc::new(Mutex::new(HashMap::new()));
        let thread = {
            let (stop, last_seen) = (stop.clone(), last_seen.clone());
            std::thread::Builder::new()
                .name("estream-gsi".into())
                .spawn(move || serve(listener, tokens, events, stop, last_seen))?
        };
        tracing::info!(port, "game state integration listening");
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
            last_seen,
        })
    }

    /// When the game last posted a valid payload.
    pub fn last_seen(&self, integration: Integration) -> Option<Instant> {
        self.last_seen
            .lock()
            .ok()
            .and_then(|m| m.get(&integration).copied())
    }
}

impl Drop for GsiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(
    listener: TcpListener,
    tokens: Tokens,
    events: Sender<GameEvent>,
    stop: Arc<AtomicBool>,
    last_seen: Arc<Mutex<HashMap<Integration, Instant>>>,
) {
    let mut cs2 = Cs2Tracker::default();
    let mut dota = Dota2Tracker::default();
    while !stop.load(Ordering::Acquire) {
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "game state integration accept failed");
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let Some((path, body)) = read_request(stream) else {
            continue;
        };
        let now = Instant::now();
        let integration = match path.trim_end_matches('/') {
            "/cs2" => Integration::Cs2Gsi,
            "/dota2" => Integration::Dota2Gsi,
            _ => continue,
        };
        let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let token = str_at(&payload, &["auth", "token"]);
        if token.is_none() || token != tokens.get(&integration).map(String::as_str) {
            tracing::warn!(
                ?integration,
                "game state payload with a wrong token ignored"
            );
            continue;
        }
        if let Ok(mut m) = last_seen.lock() {
            m.insert(integration, now);
        }
        let (game, kinds) = match integration {
            Integration::Cs2Gsi => ("Counter-Strike 2", cs2.update(&payload)),
            Integration::Dota2Gsi => ("Dota 2", dota.update(&payload, now)),
            Integration::LeagueLiveClient => continue,
        };
        for kind in kinds {
            let _ = events.send(GameEvent {
                game: game.to_owned(),
                kind,
                at: now,
            });
        }
    }
}

/// Reads one HTTP request and answers 200. Returns the path and body.
fn read_request(mut stream: TcpStream) -> Option<(String, Vec<u8>)> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let mut request = lines.next()?.split_whitespace();
    let (method, path) = (request.next()?, request.next()?.to_owned());
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if length > 4 * 1024 * 1024 {
        return None;
    }
    while buf.len() < header_end + length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    if method != "POST" {
        return None;
    }
    let end = (header_end + length).min(buf.len());
    Some((path, buf[header_end..end].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "76561198000000001";
    const OTHER: &str = "76561198000000002";

    fn cs2(steamid: &str, kills: u64, round_kills: u64, deaths: u64) -> Value {
        json!({
            "provider": { "name": "Counter-Strike 2", "appid": 730, "steamid": ME },
            "map": { "phase": "live", "team_ct": { "score": 3 }, "team_t": { "score": 2 } },
            "round": { "phase": "live" },
            "player": {
                "steamid": steamid, "team": "CT",
                "state": { "health": 100, "round_kills": round_kills },
                "match_stats": { "kills": kills, "deaths": deaths }
            }
        })
    }

    #[test]
    fn cs2_kills_multi_kills_and_aces() {
        let mut t = Cs2Tracker::default();
        assert!(
            t.update(&cs2(ME, 3, 0, 1)).is_empty(),
            "first payload is the baseline"
        );
        assert_eq!(t.update(&cs2(ME, 4, 1, 1)), [EventKind::Kill]);
        assert_eq!(t.update(&cs2(ME, 5, 2, 1)), [EventKind::MultiKill { n: 2 }]);
        assert_eq!(
            t.update(&cs2(ME, 7, 4, 1)),
            [EventKind::MultiKill { n: 4 }, EventKind::MultiKill { n: 4 }]
        );
        assert_eq!(t.update(&cs2(ME, 8, 5, 1)), [EventKind::Ace]);
        assert_eq!(t.update(&cs2(ME, 8, 0, 2)), [EventKind::Death]);
    }

    #[test]
    fn cs2_spectating_never_clips() {
        let mut t = Cs2Tracker::default();
        t.update(&cs2(ME, 3, 0, 1));
        // Dead, spectating a teammate who gets kills.
        assert!(t.update(&cs2(OTHER, 10, 1, 0)).is_empty());
        assert!(t.update(&cs2(OTHER, 11, 2, 0)).is_empty());
        // Back on our own player with unchanged counters.
        assert!(t.update(&cs2(ME, 3, 0, 1)).is_empty());
    }

    #[test]
    fn cs2_new_match_resets_the_baseline() {
        let mut t = Cs2Tracker::default();
        t.update(&cs2(ME, 20, 0, 15));
        assert!(t.update(&cs2(ME, 0, 0, 0)).is_empty());
        assert_eq!(t.update(&cs2(ME, 1, 1, 0)), [EventKind::Kill]);
    }

    #[test]
    fn cs2_round_and_match_wins_fire_once() {
        let mut t = Cs2Tracker::default();
        let mut p = cs2(ME, 3, 0, 1);
        t.update(&p);
        p["round"] = json!({ "phase": "over", "win_team": "CT" });
        assert_eq!(t.update(&p), [EventKind::RoundWin]);
        assert!(t.update(&p).is_empty(), "same phase again");
        p["round"] = json!({ "phase": "over", "win_team": "T" });
        p["round"]["phase"] = json!("live");
        t.update(&p);
        p["round"] = json!({ "phase": "over", "win_team": "T" });
        assert!(t.update(&p).is_empty(), "the other team won");
        p["map"] =
            json!({ "phase": "gameover", "team_ct": { "score": 13 }, "team_t": { "score": 9 } });
        assert_eq!(t.update(&p), [EventKind::MatchWin]);
        assert!(t.update(&p).is_empty());
    }

    fn dota(kills: u64, deaths: u64) -> Value {
        json!({
            "provider": { "name": "Dota 2", "appid": 570, "steamid": ME },
            "map": { "game_state": "DOTA_GAMERULES_STATE_GAME_IN_PROGRESS", "win_team": "none" },
            "player": { "steamid": ME, "kills": kills, "deaths": deaths, "team_name": "radiant" }
        })
    }

    #[test]
    fn dota_multi_kills_count_within_the_window() {
        let mut t = Dota2Tracker::default();
        let t0 = Instant::now();
        t.update(&dota(0, 0), t0);
        assert_eq!(
            t.update(&dota(1, 0), t0 + Duration::from_secs(1)),
            [EventKind::Kill]
        );
        assert_eq!(
            t.update(&dota(2, 0), t0 + Duration::from_secs(8)),
            [EventKind::MultiKill { n: 2 }]
        );
        assert_eq!(
            t.update(&dota(3, 0), t0 + Duration::from_secs(60)),
            [EventKind::Kill],
            "too long after"
        );
        assert_eq!(
            t.update(&dota(3, 1), t0 + Duration::from_secs(61)),
            [EventKind::Death]
        );
        let mut win = dota(3, 1);
        win["map"] =
            json!({ "game_state": "DOTA_GAMERULES_STATE_POST_GAME", "win_team": "radiant" });
        assert_eq!(
            t.update(&win, t0 + Duration::from_secs(90)),
            [EventKind::MatchWin]
        );
    }

    #[test]
    fn config_files_name_the_endpoint_and_token() {
        let text = config_text(Integration::Cs2Gsi, 3091, "abc123");
        assert!(
            text.contains("\"uri\" \"http://127.0.0.1:3091/cs2\""),
            "{text}"
        );
        assert!(text.contains("\"token\" \"abc123\""));
        assert!(text.contains("\"player_match_stats\" \"1\""));
        let dir = config_dir(Integration::Dota2Gsi, Path::new(r"C:\dota 2 beta"));
        assert_eq!(
            dir,
            Some(PathBuf::from(
                r"C:\dota 2 beta\game\dota\cfg\gamestate_integration"
            ))
        );
        assert_eq!(new_token().len(), 32);
        assert_ne!(new_token(), new_token());
    }

    fn post(port: u16, path: &str, body: &Value) -> String {
        let body = body.to_string();
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| panic!("{e}"));
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = s.write_all(req.as_bytes());
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply);
        reply
    }

    #[test]
    fn server_checks_the_token_and_reports_events() {
        let (tx, rx) = std::sync::mpsc::channel();
        let tokens = Tokens::from([(Integration::Cs2Gsi, "good".to_owned())]);
        let server = GsiServer::start(39_091, tokens, tx).unwrap_or_else(|e| panic!("start: {e}"));
        let with_token = |mut v: Value, token: &str| {
            v["auth"] = json!({ "token": token });
            v
        };
        let reply = post(server.port, "/cs2", &with_token(cs2(ME, 3, 0, 1), "good"));
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        post(server.port, "/cs2", &with_token(cs2(ME, 4, 1, 1), "wrong"));
        post(server.port, "/cs2", &cs2(ME, 4, 1, 1)); // no token at all
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "rejected"
        );
        post(server.port, "/cs2", &with_token(cs2(ME, 4, 1, 1), "good"));
        let event = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|e| panic!("no event: {e}"));
        assert_eq!(event.kind, EventKind::Kill);
        assert_eq!(event.game, "Counter-Strike 2");
        assert!(server.last_seen(Integration::Cs2Gsi).is_some());
        assert!(server.last_seen(Integration::Dota2Gsi).is_none());
    }

    #[test]
    fn install_writes_into_the_game_folder() {
        let root = std::env::temp_dir().join(format!("estream-gsi-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(
            root.join("steamapps/common/Counter-Strike Global Offensive/game/csgo/cfg"),
        );
        let _ = std::fs::write(
            root.join("steamapps/appmanifest_730.acf"),
            "\"AppState\"\n{\n\t\"installdir\"\t\t\"Counter-Strike Global Offensive\"\n}\n",
        );
        let path = install(Integration::Cs2Gsi, Some(&root), 3091, "tok")
            .unwrap_or_else(|e| panic!("install: {e}"));
        assert!(
            path.ends_with(r"game\csgo\cfg\gamestate_integration_estream.cfg")
                || path.ends_with("game/csgo/cfg/gamestate_integration_estream.cfg")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(text.contains("\"tok\""));
        assert!(matches!(
            install(Integration::Dota2Gsi, Some(&root), 3091, "tok"),
            Err(InstallError::NotInstalled)
        ));
    }
}
