//! Fortnite replays.
//!
//! Fortnite records every match (with "Record replays" on) to
//! `%LOCALAPPDATA%\FortniteGame\Saved\Demos\*.replay`, an Unreal Engine local replay: a file
//! info block, then chunks (header, game data, checkpoints, events). Only the info block, the
//! header and the event chunks are read; the game data (Oodle-compressed) is skipped.
//!
//! Event chunks hold every elimination and knock of the match, plus the recording player's own
//! stats (`AthenaMatchStats`: their elimination count) and placement (`AthenaMatchTeamStats`).
//! In finished replays the events are AES-256 encrypted with a key from the info block, which
//! Fortnite only writes when the match is over, so events are read after the match.
//!
//! Eliminations name players by account id, and nothing outside the game data says which is
//! the recording player. They're found from the account ids in Fortnite's own log (the
//! signed-in player), or else as the one player whose eliminations match their own stats; when
//! neither settles it (no kills, or a tie), only the placement is used.
//!
//! Layout as in the MIT-licensed FortniteReplayDecompressor (github.com/Shiqan).

use super::{ParsedReplay, ReplayError, ReplayEvent, ReplayFormat};
use crate::event::EventKind;
use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const GAME: &str = "Fortnite";

const FILE_MAGIC: u32 = 0x1CA2_E27F;
const NETWORK_MAGIC: u32 = 0x2CF5_A13D;
const CHUNK_HEADER: u32 = 0;
const CHUNK_EVENT: u32 = 3;

/// Where Fortnite writes its replays (Windows only).
pub fn folder() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir().map(|d| d.join("FortniteGame").join("Saved").join("Demos"))
    } else {
        None
    }
}

pub struct Fortnite;

impl ReplayFormat for Fortnite {
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
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("replay"))
            })
            .collect()
    }

    fn parse(&self, replay: &Path) -> Result<ParsedReplay, ReplayError> {
        parse(&std::fs::read(replay)?, &logged_accounts())
    }
}

/// A little-endian reader over a byte slice. Running out of bytes is `NotReady` (a file still
/// being written); nonsense values are `Unsupported`.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], ReplayError> {
        let end = self.pos.checked_add(n).ok_or(ReplayError::NotReady)?;
        let out = self.data.get(self.pos..end).ok_or(ReplayError::NotReady)?;
        self.pos = end;
        Ok(out)
    }

    fn skip(&mut self, n: usize) -> Result<(), ReplayError> {
        self.bytes(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, ReplayError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ReplayError> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, ReplayError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, ReplayError> {
        Ok(self.u32()? as i32)
    }

    fn i64(&mut self) -> Result<i64, ReplayError> {
        let b = self.bytes(8)?;
        let mut a = [0; 8];
        a.copy_from_slice(b);
        Ok(i64::from_le_bytes(a))
    }

    /// An Unreal `FString`: a signed length (negative for UTF-16) including the terminator.
    fn fstring(&mut self) -> Result<String, ReplayError> {
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len.unsigned_abs() > 1 << 20 {
            return Err(ReplayError::Unsupported(format!("string of {len}")));
        }
        let text = if len < 0 {
            let raw = self.bytes(len.unsigned_abs() as usize * 2)?;
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(self.bytes(len as usize)?).into_owned()
        };
        Ok(text.trim_end_matches('\0').to_owned())
    }
}

/// The file info block: when the match started, whether it's still recording, the event key.
struct Info {
    started: SystemTime,
    live: bool,
    key: Option<Vec<u8>>,
}

/// .NET ticks (100 ns since 0001-01-01) of the Unix epoch.
const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;

fn read_info(r: &mut Reader<'_>) -> Result<Info, ReplayError> {
    if r.u32()? != FILE_MAGIC {
        return Err(ReplayError::Unsupported("not an Unreal replay".into()));
    }
    let version = r.u32()?;
    if version >= 7 {
        let custom = r.i32()?.max(0) as usize;
        r.skip(custom * 20)?;
    }
    let _length_ms = r.u32()?;
    let _network_version = r.u32()?;
    let _changelist = r.u32()?;
    let _friendly_name = r.fstring()?;
    let live = r.u32()? != 0;
    let mut started = None;
    if version >= 3 {
        // A .NET `DateTime.ToBinary`: ticks in the low 62 bits (UTC).
        let ticks = r.i64()? & 0x3FFF_FFFF_FFFF_FFFF;
        let since_unix = ticks - UNIX_EPOCH_TICKS;
        if since_unix > 0 {
            started = Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(since_unix as u64 * 100));
        }
    }
    if version >= 2 {
        let _compressed = r.u32()?;
    }
    let mut key = None;
    if version >= 6 {
        let encrypted = r.u32()? != 0;
        let size = r.u32()? as usize;
        let bytes = r.bytes(size)?;
        if encrypted {
            key = Some(bytes.to_vec());
        }
    }
    Ok(Info {
        started: started.ok_or_else(|| ReplayError::Unsupported(format!("info v{version}")))?,
        live,
        key,
    })
}

/// What the header chunk says that event parsing needs.
#[derive(Default)]
struct Header {
    engine_network_version: u32,
    /// Fortnite's version from the branch (`++Fortnite+Release-4.2`).
    major: u32,
    minor: u32,
}

fn read_header(r: &mut Reader<'_>) -> Result<Header, ReplayError> {
    if r.u32()? != NETWORK_MAGIC {
        return Err(ReplayError::Unsupported("bad header magic".into()));
    }
    let network_version = r.u32()?;
    if network_version >= 19 {
        let custom = r.i32()?.max(0) as usize;
        r.skip(custom * 20)?;
    }
    let _checksum = r.u32()?;
    let engine_network_version = r.u32()?;
    let _game_protocol = r.u32()?;
    if network_version >= 12 {
        r.skip(16)?;
    }
    let mut header = Header {
        engine_network_version,
        ..Default::default()
    };
    if network_version >= 11 {
        let (_major, _minor, _patch) = (r.u16()?, r.u16()?, r.u16()?);
        let _changelist = r.u32()?;
        let branch = r.fstring()?;
        if let Some(v) = branch.split("Release-").nth(1) {
            let mut parts = v.split('.');
            header.major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            header.minor = parts
                .next()
                .map(|p| {
                    p.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                })
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
        }
    }
    Ok(header)
}

/// One event chunk.
struct Event {
    group: String,
    metadata: String,
    start_ms: u32,
    data: Vec<u8>,
}

fn decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>, ReplayError> {
    let cipher = aes::Aes256::new_from_slice(key)
        .map_err(|_| ReplayError::Unsupported(format!("key of {} bytes", key.len())))?;
    if !data.len().is_multiple_of(16) {
        return Err(ReplayError::Unsupported(
            "encrypted event not in blocks".into(),
        ));
    }
    let mut out = data.to_vec();
    for block in out.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    // PKCS#7 padding.
    if let Some(&pad) = out.last()
        && (1..=16).contains(&pad)
        && out.len() >= pad as usize
    {
        out.truncate(out.len() - pad as usize);
    }
    Ok(out)
}

fn read_event(r: &mut Reader<'_>, key: Option<&[u8]>) -> Result<Event, ReplayError> {
    let _id = r.fstring()?;
    let group = r.fstring()?;
    let metadata = r.fstring()?;
    let start_ms = r.u32()?;
    let _end_ms = r.u32()?;
    let size = r.i32()?.max(0) as usize;
    let raw = r.bytes(size)?;
    let data = match key {
        Some(key) => decrypt(key, raw)?,
        None => raw.to_vec(),
    };
    Ok(Event {
        group,
        metadata,
        start_ms,
        data,
    })
}

/// A player in an elimination: an account id (hex), a named bot, or an unnamed bot (`None`).
fn read_player(r: &mut Reader<'_>, version: i32) -> Result<Option<String>, ReplayError> {
    if version < 6 {
        return Ok(Some(r.fstring()?));
    }
    Ok(match r.u8()? {
        // Player: a sized id.
        17 => {
            let size = r.u8()? as usize;
            Some(r.bytes(size)?.iter().map(|b| format!("{b:02X}")).collect())
        }
        // Named bot.
        16 => Some(r.fstring()?),
        // Bot.
        _ => None,
    })
}

struct Elimination {
    eliminated: Option<String>,
    eliminator: Option<String>,
    knocked: bool,
    at_ms: u32,
}

fn read_elimination(event: &Event, header: &Header) -> Result<Elimination, ReplayError> {
    let mut r = Reader::new(&event.data);
    let version = r.i32()?;
    if version >= 3 {
        r.skip(1)?;
        if version >= 6 {
            // The eliminated player's transform (rotation, location, scale).
            r.skip(40)?;
        }
        // The eliminator's transform.
        r.skip(40)?;
    } else if header.major <= 4 && header.minor < 2 {
        r.skip(8)?;
    } else if header.major == 4 && header.minor <= 2 {
        r.skip(36)?;
    }
    if header.engine_network_version >= 34 {
        r.skip(80)?;
    }
    let eliminated = read_player(&mut r, version)?;
    let eliminator = read_player(&mut r, version)?;
    let _gun = r.u8()?;
    let knocked = r.u32()? != 0;
    Ok(Elimination {
        eliminated,
        eliminator,
        knocked,
        at_ms: event.start_ms,
    })
}

/// The recording player's eliminations, knocks, death and a win, from a replay's bytes.
pub fn parse(data: &[u8], known: &HashSet<String>) -> Result<ParsedReplay, ReplayError> {
    let mut r = Reader::new(data);
    let info = read_info(&mut r)?;
    if info.live {
        return Err(ReplayError::NotReady);
    }
    let mut header = Header::default();
    let mut eliminations = Vec::new();
    let mut my_kills = None;
    let mut placement = None;
    let mut length_ms = 0;
    while r.pos < data.len() {
        let kind = r.u32()?;
        let size = r.i32()?;
        if size <= 0 {
            return Err(ReplayError::Unsupported(format!("chunk of {size} bytes")));
        }
        let mut chunk = Reader::new(r.bytes(size as usize)?);
        match kind {
            CHUNK_HEADER => header = read_header(&mut chunk)?,
            CHUNK_EVENT => {
                let event = read_event(&mut chunk, info.key.as_deref())?;
                length_ms = length_ms.max(event.start_ms);
                if event.group == "playerElim" {
                    // An event this parser can't read is skipped, not the whole replay.
                    if let Ok(e) = read_elimination(&event, &header) {
                        eliminations.push(e);
                    }
                } else if event.metadata == "AthenaMatchStats" {
                    let mut s = Reader::new(&event.data);
                    // unknown, accuracy, assists, eliminations
                    s.skip(12)?;
                    my_kills = Some(s.u32()?);
                } else if event.metadata == "AthenaMatchTeamStats" {
                    let mut s = Reader::new(&event.data);
                    s.skip(4)?;
                    placement = Some(s.u32()?);
                }
            }
            _ => {}
        }
    }
    let mut events = owner(&eliminations, my_kills, known)
        .map(|me| player_events(&eliminations, &me))
        .unwrap_or_default();
    if placement == Some(1) {
        events.push(ReplayEvent {
            kind: EventKind::MatchWin,
            match_time: Duration::from_millis(u64::from(length_ms)),
        });
    }
    events.sort_by_key(|e| e.match_time);
    Ok(ParsedReplay {
        started: info.started,
        events,
    })
}

/// `me`'s knocks, eliminations and final elimination (being knocked isn't an event).
fn player_events(eliminations: &[Elimination], me: &str) -> Vec<ReplayEvent> {
    eliminations
        .iter()
        .filter_map(|e| {
            let by_me = e.eliminator.as_deref() == Some(me);
            let on_me = e.eliminated.as_deref() == Some(me);
            let kind = match (by_me, on_me) {
                (true, false) if e.knocked => EventKind::Knock,
                (true, false) => EventKind::Kill,
                (_, true) if !e.knocked => EventKind::Death,
                _ => return None,
            };
            Some(ReplayEvent {
                kind,
                match_time: Duration::from_millis(u64::from(e.at_ms)),
            })
        })
        .collect()
}

/// The recording player. `kills`: their own elimination count; `known`: account ids from this
/// PC's Fortnite log (the signed-in player, maybe party members).
///
/// A player seen both in the log and in the match is taken when they're the only one, or the
/// only one whose eliminations fit the stats. Otherwise it's the only player whose final
/// eliminations fit: the stats count `kills`, and can count one more than the feed credits (a
/// knock a teammate finished).
fn owner(
    eliminations: &[Elimination],
    kills: Option<u32>,
    known: &HashSet<String>,
) -> Option<String> {
    let mut finals: HashMap<&String, u32> = HashMap::new();
    for e in eliminations {
        for id in [&e.eliminator, &e.eliminated].into_iter().flatten() {
            finals.entry(id).or_default();
        }
        if let Some(by) = &e.eliminator
            && !e.knocked
            && e.eliminated.as_ref() != Some(by)
        {
            *finals.entry(by).or_default() += 1;
        }
    }
    let in_log: Vec<&String> = finals
        .keys()
        .copied()
        .filter(|id| known.contains(*id))
        .collect();
    if let [id] = in_log.as_slice() {
        return Some((*id).clone());
    }
    let pool = if in_log.is_empty() {
        finals.keys().copied().collect()
    } else {
        in_log
    };
    let kills = kills.filter(|k| *k > 0)?;
    // An exact match first; only without one, one fewer in the feed than in the stats.
    [kills, kills - 1].into_iter().find_map(|n| {
        match pool
            .iter()
            .filter(|id| finals[**id] == n)
            .collect::<Vec<_>>()
            .as_slice()
        {
            [id] => Some((**id).clone()),
            _ => None,
        }
    })
}

/// Account ids (32 hex digits) in Fortnite's current log, upper-cased like the replay's.
fn logged_accounts() -> HashSet<String> {
    let Some(log) = dirs::data_local_dir().map(|d| {
        d.join("FortniteGame")
            .join("Saved")
            .join("Logs")
            .join("FortniteGame.log")
    }) else {
        return HashSet::new();
    };
    std::fs::read(log)
        .map(|bytes| account_ids(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// Runs of exactly 32 hex digits in `text`.
fn account_ids(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|w| w.len() == 32)
        .map(str::to_ascii_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elim(by: &str, on: &str, knocked: bool, at_ms: u32) -> Elimination {
        let id = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        Elimination {
            eliminated: id(on),
            eliminator: id(by),
            knocked,
            at_ms,
        }
    }

    /// Final eliminations: ME 2 (plus a knock), X 3 (including ME), Y 1, a bot 1.
    fn match_feed() -> Vec<Elimination> {
        vec![
            elim("ME", "A", true, 1_000),
            elim("ME", "A", false, 2_000),
            elim("ME", "B", false, 3_000),
            elim("X", "C", false, 4_000),
            elim("X", "D", false, 5_000),
            elim("Y", "E", false, 5_500),
            elim("", "F", false, 6_000),
            elim("X", "ME", false, 7_000),
        ]
    }

    #[test]
    fn the_owner_is_the_player_whose_kills_match_their_stats() {
        let none = HashSet::new();
        let feed = match_feed();
        assert_eq!(owner(&feed, Some(2), &none).as_deref(), Some("ME"));
        assert_eq!(owner(&feed, Some(3), &none).as_deref(), Some("X"));
        // Nobody has 4: the one with 3 (a knock a teammate finished counts in the stats).
        assert_eq!(owner(&feed, Some(4), &none).as_deref(), Some("X"));
        assert_eq!(
            owner(&feed, Some(0), &none),
            None,
            "no kills: nothing to match"
        );
        assert_eq!(owner(&feed, None, &none), None);
        let mut tied = feed;
        tied.push(elim("Y", "G", false, 8_000));
        assert_eq!(owner(&tied, Some(2), &none), None, "ME and Y both have 2");
    }

    #[test]
    fn the_recording_players_knocks_kills_and_death() {
        let events: Vec<(EventKind, u128)> = player_events(&match_feed(), "ME")
            .into_iter()
            .map(|e| (e.kind, e.match_time.as_millis()))
            .collect();
        assert_eq!(
            events,
            vec![
                (EventKind::Knock, 1_000),
                (EventKind::Kill, 2_000),
                (EventKind::Kill, 3_000),
                (EventKind::Death, 7_000),
            ]
        );
    }

    #[test]
    fn short_or_foreign_files_are_not_ready_or_unsupported() {
        let none = HashSet::new();
        assert!(matches!(
            parse(&[0x7f, 0xe2], &none),
            Err(ReplayError::NotReady)
        ));
        assert!(matches!(
            parse(&[1, 2, 3, 4, 5, 6, 7, 8], &none),
            Err(ReplayError::Unsupported(_))
        ));
    }

    #[test]
    fn an_account_in_the_log_settles_it() {
        let me = "AB000000000000000000000000000ABC";
        let log = account_ids(&format!(
            "LogOnline: login ok AccountId={} x",
            me.to_lowercase()
        ));
        assert!(log.contains(me));
        let feed = vec![elim("X", "Y", false, 1), elim("Z", me, false, 2)];
        assert_eq!(
            owner(&feed, Some(0), &log).as_deref(),
            Some(me),
            "even without kills"
        );
        // A party member in the log too: the stats pick between them.
        let mut log = log;
        log.insert("X".into());
        assert_eq!(owner(&feed, Some(1), &log).as_deref(), Some("X"));
        assert_eq!(owner(&feed, Some(0), &log), None);
    }

    /// Prints every event chunk's group and metadata, and the eliminations that fail to read.
    fn inspect(data: &[u8]) {
        let mut r = Reader::new(data);
        let info = read_info(&mut r).unwrap();
        println!(
            "  live {} key {:?}",
            info.live,
            info.key.as_ref().map(Vec::len)
        );
        let mut header = Header::default();
        let mut groups: HashMap<String, u32> = HashMap::new();
        let (mut ok, mut bad) = (0, 0);
        let mut per: HashMap<String, (u32, u32)> = HashMap::new();
        while r.pos < data.len() {
            let kind = r.u32().unwrap();
            let size = r.i32().unwrap();
            let mut chunk = Reader::new(r.bytes(size as usize).unwrap());
            match kind {
                CHUNK_HEADER => {
                    header = read_header(&mut chunk).unwrap();
                    println!(
                        "  engine net {} version {}.{}",
                        header.engine_network_version, header.major, header.minor
                    );
                }
                CHUNK_EVENT => {
                    let e = read_event(&mut chunk, info.key.as_deref()).unwrap();
                    *groups
                        .entry(format!("{} / {}", e.group, e.metadata))
                        .or_default() += 1;
                    if e.group == "playerElim" {
                        match read_elimination(&e, &header) {
                            Ok(el) => {
                                ok += 1;
                                let by = el.eliminator.clone().unwrap_or_else(|| "bot".into());
                                let c = per.entry(by).or_default();
                                if el.knocked { c.1 += 1 } else { c.0 += 1 }
                            }
                            Err(err) => {
                                bad += 1;
                                if bad == 1 {
                                    println!(
                                        "  elim error {err}: {:02x?}",
                                        &e.data[..e.data.len().min(160)]
                                    );
                                }
                            }
                        }
                    }
                    if e.metadata.starts_with("AthenaMatch") {
                        println!("  {} {:02x?}", e.metadata, e.data);
                    }
                }
                _ => {}
            }
        }
        println!("  groups {groups:?}");
        println!("  eliminations read {ok}, failed {bad}");
        let mut per: Vec<_> = per.into_iter().filter(|(_, (k, d))| k + d >= 3).collect();
        per.sort_by_key(|(_, (k, _))| std::cmp::Reverse(*k));
        println!("  top eliminators (final, knocks): {per:?}");
    }

    /// Prints what `parse` reads from every replay in `ESTREAM_FN_REPLAYS` (a folder). Run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_sample_replays() {
        let dir = PathBuf::from(std::env::var("ESTREAM_FN_REPLAYS").unwrap());
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let data = std::fs::read(entry.path()).unwrap();
            if data.is_empty() {
                continue;
            }
            println!("== {}", entry.path().display());
            inspect(&data);
            match parse(&data, &HashSet::new()) {
                Ok(p) => {
                    println!("  started {:?}", p.started);
                    for e in &p.events {
                        println!("  {:>8.1}s {}", e.match_time.as_secs_f64(), e.kind.label());
                    }
                }
                Err(e) => println!("  error: {e}"),
            }
        }
    }
}
