//! Rocket League replays.
//!
//! Rocket League saves a replay when a match ends (with the in-game "Auto save replays" option
//! on) to `Documents\My Games\Rocket League\TAGame\Demos` (Steam) or `DemosEpic` (Epic). Only
//! the header is read, with `boxcars`; it lists every goal with its frame and scorer, the
//! final score and the recording player.

use super::{ParsedReplay, ReplayError, ReplayEvent, ReplayFormat};
use crate::event::EventKind;
use boxcars::HeaderProp;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const GAME: &str = "Rocket League";

/// The folders Rocket League saves replays to: Steam's and Epic's.
const DEMO_FOLDERS: [&str; 2] = ["Demos", "DemosEpic"];

/// Rocket League's save folder (the replays are in its `Demos*` folders).
pub fn folder() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::document_dir().map(|d| d.join("My Games").join("Rocket League").join("TAGame"))
    } else {
        None
    }
}

/// Whether Rocket League has saved any replay folder yet.
pub fn has_replays() -> bool {
    folder().is_some_and(|f| DEMO_FOLDERS.iter().any(|d| f.join(d).is_dir()))
}

pub struct RocketLeague;

impl ReplayFormat for RocketLeague {
    fn game(&self) -> &'static str {
        GAME
    }

    fn folder(&self) -> Option<PathBuf> {
        folder()
    }

    fn replays(&self, folder: &Path) -> Vec<PathBuf> {
        DEMO_FOLDERS
            .iter()
            .filter_map(|d| std::fs::read_dir(folder.join(d)).ok())
            .flat_map(|entries| entries.flatten())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("replay"))
            })
            .collect()
    }

    fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError> {
        let data = std::fs::read(replay)?;
        // The file is written once, at the end of the match; its time is when the match ended.
        let saved = std::fs::metadata(replay)?.modified()?;
        parse(&data, saved)
    }
}

fn prop<'a>(props: &'a [(String, HeaderProp)], key: &str) -> Option<&'a HeaderProp> {
    props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn int(props: &[(String, HeaderProp)], key: &str) -> Option<i64> {
    match prop(props, key)? {
        HeaderProp::Int(n) => Some(i64::from(*n)),
        HeaderProp::QWord(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn float(props: &[(String, HeaderProp)], key: &str) -> Option<f64> {
    match prop(props, key)? {
        HeaderProp::Float(f) => Some(f64::from(*f)),
        HeaderProp::Int(n) => Some(f64::from(*n)),
        _ => None,
    }
}

fn text<'a>(props: &'a [(String, HeaderProp)], key: &str) -> Option<&'a str> {
    match prop(props, key)? {
        HeaderProp::Str(s) | HeaderProp::Name(s) => Some(s),
        _ => None,
    }
}

/// The local player's goals and a win, from a replay's bytes. `saved`: when the file was
/// written (the end of the match).
pub fn parse(data: &[u8], saved: SystemTime) -> Result<ParsedReplay, ReplayError> {
    let replay = boxcars::ParserBuilder::new(data)
        .never_parse_network_data()
        .never_check_crc()
        .parse()
        .map_err(|e| {
            if cut_short(&e) {
                // A file cut short is one still being written.
                ReplayError::NotReady
            } else {
                ReplayError::Unsupported(e.to_string())
            }
        })?;
    let props = &replay.properties;
    let fps = float(props, "RecordFPS")
        .filter(|f| *f > 0.0)
        .unwrap_or(30.0);
    let frames = int(props, "NumFrames").unwrap_or(0).max(0) as f64;
    let length = Duration::from_secs_f64(frames / fps);
    let started = saved.checked_sub(length).unwrap_or(saved);
    let me = text(props, "PlayerName").unwrap_or_default();
    let my_team = player_team(props, me).or_else(|| int(props, "PrimaryPlayerTeam"));

    let mut events = Vec::new();
    if let Some(HeaderProp::Array(goals)) = prop(props, "Goals") {
        for goal in goals {
            if !me.is_empty() && text(goal, "PlayerName") == Some(me) {
                let frame = int(goal, "frame").unwrap_or(0).max(0) as f64;
                events.push(ReplayEvent {
                    kind: EventKind::Goal,
                    match_time: Duration::from_secs_f64(frame / fps),
                });
            }
        }
    }
    let (blue, orange) = (
        int(props, "Team0Score").unwrap_or(0),
        int(props, "Team1Score").unwrap_or(0),
    );
    let won = match my_team {
        Some(0) => blue > orange,
        Some(1) => orange > blue,
        _ => false,
    };
    if won {
        events.push(ReplayEvent {
            kind: EventKind::MatchWin,
            match_time: length,
        });
    }
    events.sort_by_key(|e| e.match_time);
    Ok(ParsedReplay { started, events })
}

/// Whether boxcars ran out of bytes (possibly inside a section's error).
fn cut_short(e: &boxcars::ParseError) -> bool {
    match e {
        boxcars::ParseError::InsufficientData(..) => true,
        boxcars::ParseError::ParseError(_, _, inner)
        | boxcars::ParseError::CorruptReplay(_, inner) => cut_short(inner),
        _ => false,
    }
}

/// The recording player's team (0 blue, 1 orange) from the scoreboard.
fn player_team(props: &[(String, HeaderProp)], me: &str) -> Option<i64> {
    let Some(HeaderProp::Array(players)) = prop(props, "PlayerStats") else {
        return None;
    };
    players
        .iter()
        .find(|p| !me.is_empty() && text(p, "Name") == Some(me))
        .and_then(|p| int(p, "Team"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    const REPLAY: &[u8] = include_bytes!("../../../tests/data/rocket-league.replay");

    #[test]
    fn reads_the_recording_players_goal_and_win() {
        let saved = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let parsed = parse(REPLAY, saved).unwrap();
        let events: Vec<(EventKind, u128)> = parsed
            .events
            .iter()
            .map(|e| (e.kind.clone(), e.match_time.as_millis()))
            .collect();
        // FikusRL (orange) scored at frame 492 of 581, at 30 fps; orange won 1-0.
        assert_eq!(
            events,
            vec![(EventKind::Goal, 16_400), (EventKind::MatchWin, 19_366)]
        );
        let length = Duration::from_secs_f64(581.0 / 30.0);
        assert_eq!(
            parsed.started,
            saved - length,
            "started = saved minus its length"
        );
    }

    #[test]
    fn a_file_cut_short_is_not_ready() {
        let cut = &REPLAY[..REPLAY.len() / 3];
        let r = parse(cut, SystemTime::now());
        assert!(
            matches!(r, Err(ReplayError::NotReady)),
            "{:?}",
            r.map(|p| p.events)
        );
        assert!(matches!(
            parse(b"not a replay at all", SystemTime::now()),
            Err(ReplayError::NotReady | ReplayError::Unsupported(_))
        ));
    }

    /// Prints the header of every replay in `ESTREAM_RL_REPLAYS` (a folder), with what `parse`
    /// makes of it. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_sample_replays() {
        let dir = PathBuf::from(std::env::var("ESTREAM_RL_REPLAYS").unwrap());
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let data = std::fs::read(entry.path()).unwrap();
            let replay = boxcars::ParserBuilder::new(&data)
                .never_parse_network_data()
                .never_check_crc()
                .parse()
                .unwrap();
            println!("== {}", entry.path().display());
            for (k, v) in &replay.properties {
                println!("  {k}: {v:?}");
            }
            println!("  -> {:?}", parse(&data, SystemTime::now()).unwrap().events);
        }
    }
}
