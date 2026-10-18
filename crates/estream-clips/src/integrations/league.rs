//! League of Legends Live Client Data API.
//!
//! While a game is running, the client serves `https://127.0.0.1:2999/liveclientdata/…` with
//! a self-signed certificate. The poller asks for the active player's name and the event
//! list every 500 ms and reports new events where the active player is the killer (or their
//! team took the objective, or they won). TLS checks are skipped for this loopback address
//! only.

use super::GameEvent;
use crate::event::EventKind;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const GAME: &str = "League of Legends";
const BASE: &str = "https://127.0.0.1:2999/liveclientdata";
const POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct LeagueEvent {
    #[serde(rename = "EventID")]
    pub event_id: u64,
    pub event_name: String,
    #[serde(default)]
    pub killer_name: Option<String>,
    #[serde(default)]
    pub acer: Option<String>,
    #[serde(default)]
    pub kill_streak: Option<u8>,
    #[serde(default)]
    pub dragon_type: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub stolen: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EventList {
    events: Vec<LeagueEvent>,
}

/// Turns the event list into new clip events for `me` (the active player's name).
#[derive(Debug, Default)]
pub struct LeagueTracker {
    /// Events up to this id have been handled.
    seen: Option<u64>,
}

impl LeagueTracker {
    /// Events already in the list on the first call (Clutch opened mid-game) are not
    /// clipped, as they're no longer in the buffer.
    pub fn update(&mut self, me: &str, events: &[LeagueEvent]) -> Vec<EventKind> {
        let newest = events.iter().map(|e| e.event_id).max();
        let Some(seen) = self.seen else {
            self.seen = Some(newest.unwrap_or(0));
            // A fresh game starts with GameStart (id 0); anything later already happened.
            return if newest.is_some_and(|n| n > 0) {
                Vec::new()
            } else {
                events.iter().filter_map(|e| kind_of(me, e)).collect()
            };
        };
        if newest.is_some_and(|n| n < seen) {
            // A new game: the ids started over.
            self.seen = None;
            return self.update(me, events);
        }
        let mut out = Vec::new();
        for e in events.iter().filter(|e| e.event_id > seen) {
            out.extend(kind_of(me, e));
        }
        if let Some(n) = newest {
            self.seen = Some(seen.max(n));
        }
        out
    }
}

fn is(me: &str, name: &Option<String>) -> bool {
    name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(me))
}

fn kind_of(me: &str, e: &LeagueEvent) -> Option<EventKind> {
    match e.event_name.as_str() {
        "ChampionKill" if is(me, &e.killer_name) => Some(EventKind::Kill),
        "Multikill" if is(me, &e.killer_name) => Some(EventKind::MultiKill {
            n: e.kill_streak.unwrap_or(2),
        }),
        "Ace" if is(me, &e.acer) => Some(EventKind::Ace),
        "DragonKill" if is(me, &e.killer_name) => Some(EventKind::Objective {
            name: "dragon".into(),
        }),
        "BaronKill" if is(me, &e.killer_name) => Some(EventKind::Objective {
            name: "baron".into(),
        }),
        "HeraldKill" if is(me, &e.killer_name) => Some(EventKind::Objective {
            name: "herald".into(),
        }),
        "GameEnd" if e.result.as_deref() == Some("Win") => Some(EventKind::MatchWin),
        _ => None,
    }
}

/// Polls the Live Client Data API on its own thread while League is running.
pub struct LeaguePoller {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LeaguePoller {
    pub fn start(events: Sender<GameEvent>) -> std::io::Result<Self> {
        Self::start_at(BASE.to_owned(), events)
    }

    /// For tests: poll another base URL.
    pub fn start_at(base: String, events: Sender<GameEvent>) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("estream-league".into())
                .spawn(move || poll(&base, &events, &stop))?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for LeaguePoller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn agent() -> ureq::Agent {
    // The client's certificate is self-signed and only ever served on 127.0.0.1:2999.
    let tls = ureq::tls::TlsConfig::builder()
        .disable_verification(true)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(2)))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn get_json<T: serde::de::DeserializeOwned>(agent: &ureq::Agent, url: &str) -> Option<T> {
    let mut response = agent.get(url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.body_mut().read_json().ok()
}

fn poll(base: &str, events: &Sender<GameEvent>, stop: &AtomicBool) {
    let agent = agent();
    let mut tracker = LeagueTracker::default();
    let mut me: Option<String> = None;
    while !stop.load(Ordering::Acquire) {
        let started = Instant::now();
        if me.is_none() {
            me = get_json::<String>(&agent, &format!("{base}/activeplayername"));
        }
        if let Some(name) = &me {
            match get_json::<EventList>(&agent, &format!("{base}/eventdata")) {
                Some(list) => {
                    let at = Instant::now();
                    // Riot IDs come back as "Name#TAG"; events use just the name.
                    let short = name.split('#').next().unwrap_or(name);
                    for kind in tracker.update(short, &list.events) {
                        let _ = events.send(GameEvent {
                            game: GAME.to_owned(),
                            kind,
                            at,
                        });
                    }
                }
                None => {
                    // The game ended or isn't loaded yet: start over next time.
                    me = None;
                    tracker = LeagueTracker::default();
                }
            }
        }
        let wait = POLL.saturating_sub(started.elapsed());
        let until = Instant::now() + wait;
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    fn ev(id: u64, name: &str, extra: serde_json::Value) -> LeagueEvent {
        let mut v = json!({ "EventID": id, "EventName": name, "EventTime": id as f64 * 10.0 });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn only_the_active_players_events_count() {
        let mut t = LeagueTracker::default();
        let mut list = vec![ev(0, "GameStart", json!({}))];
        assert!(t.update("Me", &list).is_empty());
        list.push(ev(
            1,
            "ChampionKill",
            json!({ "KillerName": "Me", "VictimName": "X" }),
        ));
        list.push(ev(
            2,
            "ChampionKill",
            json!({ "KillerName": "Ally", "VictimName": "Y" }),
        ));
        list.push(ev(
            3,
            "Multikill",
            json!({ "KillerName": "me", "KillStreak": 3 }),
        ));
        assert_eq!(
            t.update("Me", &list),
            [EventKind::Kill, EventKind::MultiKill { n: 3 }]
        );
        assert!(t.update("Me", &list).is_empty(), "already seen");
        list.push(ev(4, "Ace", json!({ "Acer": "Me", "AcingTeam": "ORDER" })));
        list.push(ev(
            5,
            "BaronKill",
            json!({ "KillerName": "Me", "Stolen": "False" }),
        ));
        list.push(ev(
            6,
            "DragonKill",
            json!({ "KillerName": "Ally", "DragonType": "Fire" }),
        ));
        list.push(ev(7, "GameEnd", json!({ "Result": "Win" })));
        assert_eq!(
            t.update("Me", &list),
            [
                EventKind::Ace,
                EventKind::Objective {
                    name: "baron".into()
                },
                EventKind::MatchWin
            ]
        );
    }

    #[test]
    fn joining_mid_game_skips_old_events_and_new_games_start_over() {
        let mut t = LeagueTracker::default();
        let old = vec![
            ev(0, "GameStart", json!({})),
            ev(1, "ChampionKill", json!({ "KillerName": "Me" })),
        ];
        assert!(t.update("Me", &old).is_empty(), "already happened");
        let new_game = vec![ev(0, "GameStart", json!({}))];
        assert!(t.update("Me", &new_game).is_empty());
        let mut next = new_game.clone();
        next.push(ev(1, "ChampionKill", json!({ "KillerName": "Me" })));
        assert_eq!(t.update("Me", &next), [EventKind::Kill]);
    }

    /// A plain-HTTP stand-in for the client: answers from a shared event list.
    fn stub(events: Arc<Mutex<serde_json::Value>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let body = if req.starts_with("GET /liveclientdata/activeplayername") {
                    json!("Me#EUW").to_string()
                } else if req.starts_with("GET /liveclientdata/eventdata") {
                    events.lock().map(|e| e.to_string()).unwrap_or_default()
                } else {
                    String::new()
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    #[test]
    fn poller_reports_new_kills_from_the_client() {
        let data = Arc::new(Mutex::new(json!({ "Events": [
            { "EventID": 0, "EventName": "GameStart", "EventTime": 0.0 }
        ] })));
        let port = stub(data.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let poller = LeaguePoller::start_at(format!("http://127.0.0.1:{port}/liveclientdata"), tx)
            .unwrap_or_else(|e| panic!("{e}"));
        std::thread::sleep(Duration::from_millis(800));
        if let Ok(mut d) = data.lock()
            && let Some(list) = d["Events"].as_array_mut()
        {
            list.push(json!({ "EventID": 1, "EventName": "ChampionKill", "EventTime": 90.0, "KillerName": "Me" }));
        }
        let event = rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|e| panic!("no event: {e}"));
        assert_eq!(event.kind, EventKind::Kill);
        assert_eq!(event.game, GAME);
        drop(poller);
    }
}
