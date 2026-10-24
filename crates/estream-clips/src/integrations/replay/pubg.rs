//! PUBG: Battlegrounds replays.
//!
//! PUBG keeps the replay in memory during a match and writes it to
//! `%LOCALAPPDATA%\TslGame\Saved\Demos\<match>\` the moment the player's match ends (death or
//! win), so events arrive a few seconds after that, never during play. The files used:
//!
//! - `PUBG.replayinfo`: JSON with the start time (`Timestamp`, Unix ms), `bIsLive` and the
//!   recording player's nickname (`RecordUserNickName`).
//! - `events/kill<n>`, `events/DBNO<n>`: JSON with the event id and `time1` (ms into the replay).
//! - `data/<id>`: JSON with `instigatorName`, `victimName` and `bDBNO` for that event.
//! - `data/ReplaySummary<n>`: per-player results (`ranking`), every byte shifted down by one.
//!
//! Every file is a little-endian `u32` length followed by that many bytes (NUL-padded).

use super::{ParsedReplay, ReplayError, ReplayEvent, ReplayFormat};
use crate::event::EventKind;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

pub const GAME: &str = "PUBG: Battlegrounds";

/// Where PUBG writes its replays, on Windows (the only platform it runs on).
pub fn folder() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("TslGame").join("Saved").join("Demos"))
    } else {
        None
    }
}

pub struct Pubg;

impl ReplayFormat for Pubg {
    fn game(&self) -> &'static str {
        GAME
    }

    fn folder(&self) -> Option<PathBuf> {
        folder()
    }

    fn replays(&self, folder: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("match."))
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect()
    }

    fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError> {
        parse(replay)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReplayInfo {
    #[serde(rename = "LengthInMS")]
    length_in_ms: u64,
    timestamp: u64,
    #[serde(rename = "bIsLive", default)]
    is_live: bool,
    record_user_nick_name: String,
    #[serde(default)]
    game_version: String,
}

#[derive(Deserialize)]
struct EventEntry {
    id: String,
    group: String,
    time1: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KillData {
    #[serde(default)]
    instigator_name: String,
    #[serde(default)]
    victim_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    #[serde(default)]
    player_state_summaries: Vec<PlayerSummary>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerSummary {
    player_name: String,
    #[serde(default)]
    ranking: u32,
}

/// The payload of a length-prefixed replay file.
fn read_record(path: &Path) -> Result<Vec<u8>, ReplayError> {
    let bytes = std::fs::read(path)?;
    let Some(len) = bytes.get(..4) else {
        return Err(ReplayError::NotReady);
    };
    let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
    let mut payload = bytes[4..].to_vec();
    if payload.len() < len {
        return Err(ReplayError::NotReady);
    }
    payload.truncate(len);
    while payload.last() == Some(&0) {
        payload.pop();
    }
    Ok(payload)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, what: &str) -> Result<T, ReplayError> {
    let payload = read_record(path)?;
    serde_json::from_slice(&payload).map_err(|e| ReplayError::Unsupported(format!("{what}: {e}")))
}

pub fn parse(replay: &Path) -> Result<ParsedReplay, ReplayError> {
    let info_path = replay.join("PUBG.replayinfo");
    if !info_path.exists() {
        // Written last, once the rest of the replay is on disk.
        return Err(ReplayError::NotReady);
    }
    let info: ReplayInfo = read_json(&info_path, "replayinfo")?;
    if info.is_live {
        return Err(ReplayError::NotReady);
    }
    let me = info.record_user_nick_name.as_str();
    let mut events = Vec::new();
    let event_files = std::fs::read_dir(replay.join("events")).map_err(|_| {
        ReplayError::Unsupported(format!("no events folder ({})", info.game_version))
    })?;
    for file in event_files.flatten() {
        let name = file.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("kill") || name.starts_with("DBNO")) {
            continue;
        }
        let entry: EventEntry = read_json(&file.path(), "event")?;
        let data: KillData = read_json(&replay.join("data").join(&entry.id), "event data")?;
        let (by_me, on_me) = (
            is_me(&data.instigator_name, me),
            is_me(&data.victim_name, me),
        );
        let kind = match entry.group.as_str() {
            "kill" if by_me && !on_me => EventKind::Kill,
            "kill" if on_me => EventKind::Death,
            "DBNO" if by_me && !on_me => EventKind::Knock,
            _ => continue,
        };
        events.push(ReplayEvent {
            kind,
            match_time: Duration::from_millis(entry.time1),
        });
    }
    if placement(replay, me) == Some(1) {
        events.push(ReplayEvent {
            kind: EventKind::MatchWin,
            match_time: Duration::from_millis(info.length_in_ms),
        });
    }
    events.sort_by_key(|e| e.match_time);
    Ok(ParsedReplay {
        started: UNIX_EPOCH + Duration::from_millis(info.timestamp),
        events,
    })
}

/// Whether a kill-feed name is the recording player: the feed shows clan tags
/// (`[MYR] Name`), the replay info doesn't.
fn is_me(name: &str, me: &str) -> bool {
    let bare = match name.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((_, rest)) => rest,
        None => name,
    };
    !me.is_empty() && bare == me
}

/// The player's final placement, from the last replay summary.
fn placement(replay: &Path, me: &str) -> Option<u32> {
    let last = std::fs::read_dir(replay.join("data"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("ReplaySummary")?.parse::<u32>().ok()
        })
        .max()?;
    let mut payload =
        read_record(&replay.join("data").join(format!("ReplaySummary{last}"))).ok()?;
    payload.iter_mut().for_each(|b| *b = b.wrapping_add(1));
    let summary: Summary = serde_json::from_slice(&payload).ok()?;
    summary
        .player_state_summaries
        .iter()
        .find(|p| p.player_name == me)
        .map(|p| p.ranking)
        .filter(|&r| r > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(payload: &str) -> Vec<u8> {
        let mut out = (payload.len() as u32 + 1).to_le_bytes().to_vec();
        out.extend_from_slice(payload.as_bytes());
        out.push(0);
        out
    }

    fn shifted(payload: &str) -> String {
        payload.bytes().map(|b| (b - 1) as char).collect()
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str, live: bool) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("estream-pubg-{name}-{}", std::process::id()))
                .join("match.bro.official.test");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("events")).unwrap();
            std::fs::create_dir_all(dir.join("data")).unwrap();
            let info = format!(
                r#"{{"LengthInMS": 1662414, "Timestamp": 1792138192623, "bIsLive": {live},
                "RecordUserNickName": "Me", "GameVersion": "2610.1.1.65"}}"#
            );
            std::fs::write(dir.join("PUBG.replayinfo"), record(&info)).unwrap();
            Self(dir)
        }

        fn event(&self, id: &str, group: &str, ms: u64, by: &str, victim: &str) {
            let entry = format!(
                "{{\r\n\t\"id\": \"{id}\",\r\n\t\"group\": \"{group}\",\r\n\t\"time1\": {ms},\r\n\t\"time2\": {ms}\r\n}}"
            );
            std::fs::write(self.0.join("events").join(id), record(&entry)).unwrap();
            let data = format!(
                r#"{{"instigatorName":"{by}","victimName":"{victim}","bDBNO":{}}}"#,
                group == "DBNO"
            );
            std::fs::write(self.0.join("data").join(id), record(&data)).unwrap();
        }

        fn summary(&self, n: u32, ranking: u32) {
            let s = format!(
                r#"{{"playerStateSummaries":[{{"playerName":"Other","ranking":2}},{{"playerName":"Me","ranking":{ranking}}}]}}"#
            );
            std::fs::write(
                self.0.join("data").join(format!("ReplaySummary{n}")),
                record(&shifted(&s)),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    #[test]
    fn reads_the_local_players_kills_knocks_death_and_win() {
        let f = Fixture::new("events", false);
        f.event("DBNO0", "DBNO", 100_000, "Me", "A");
        f.event("kill0", "kill", 101_000, "[TAG] Me", "A"); // clan tag in the kill feed
        f.event("kill1", "kill", 150_000, "B", "C"); // someone else's kill
        f.event("DBNO1", "DBNO", 160_000, "B", "Me"); // knocked: not an event
        f.event("kill2", "kill", 170_000, "B", "Me");
        f.summary(0, 0);
        f.summary(1, 1);

        let parsed = parse(&f.0).unwrap();
        let kinds: Vec<(EventKind, u64)> = parsed
            .events
            .iter()
            .map(|e| (e.kind.clone(), e.match_time.as_millis() as u64))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (EventKind::Knock, 100_000),
                (EventKind::Kill, 101_000),
                (EventKind::Death, 170_000),
                (EventKind::MatchWin, 1_662_414),
            ]
        );
        assert_eq!(
            parsed.started,
            UNIX_EPOCH + Duration::from_millis(1_792_138_192_623)
        );
    }

    #[test]
    fn matches_names_with_or_without_a_clan_tag() {
        assert!(is_me("Me", "Me"));
        assert!(is_me("[MYR] Me", "Me"));
        assert!(!is_me("[MYR] Me2", "Me"));
        assert!(!is_me("NotMe", "Me"));
        assert!(!is_me("", ""));
    }

    #[test]
    fn no_win_without_first_place() {
        let f = Fixture::new("placement", false);
        f.summary(0, 5);
        assert!(parse(&f.0).unwrap().events.is_empty());
    }

    #[test]
    fn live_or_missing_info_is_not_ready() {
        let f = Fixture::new("live", true);
        assert!(matches!(parse(&f.0), Err(ReplayError::NotReady)));
        std::fs::remove_file(f.0.join("PUBG.replayinfo")).unwrap();
        assert!(matches!(parse(&f.0), Err(ReplayError::NotReady)));
    }

    #[test]
    fn unknown_layouts_are_unsupported() {
        let f = Fixture::new("unknown", false);
        std::fs::write(f.0.join("PUBG.replayinfo"), record("not json")).unwrap();
        assert!(matches!(parse(&f.0), Err(ReplayError::Unsupported(_))));
    }

    #[test]
    fn truncated_records_are_not_ready() {
        let f = Fixture::new("truncated", false);
        let mut bytes = record(r#"{"LengthInMS": 1}"#);
        bytes.truncate(10);
        std::fs::write(f.0.join("PUBG.replayinfo"), bytes).unwrap();
        assert!(matches!(parse(&f.0), Err(ReplayError::NotReady)));
    }

    /// Reads the newest real replay on this machine (run with `--ignored` where PUBG is installed).
    #[test]
    #[ignore]
    fn reads_the_newest_local_replay() {
        let folder = folder().unwrap();
        let newest = Pubg
            .replays(&folder)
            .into_iter()
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .unwrap();
        let parsed = parse(&newest).unwrap();
        for e in &parsed.events {
            println!("{:>8.1}s {}", e.match_time.as_secs_f64(), e.kind.label());
        }
    }
}
