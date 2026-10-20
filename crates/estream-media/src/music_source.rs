//! The `music` source: Creative Commons music from Jamendo, picked by genre or tags and played
//! one track at a time through the media [`Player`]. A controller thread per source fetches the
//! playlist, starts the next track when one ends, and writes the current credit to a text file
//! (for an on-screen "now playing" line).

use crate::media_source::MediaAudio;
use crate::player::{self, PlayState, Player, PlayerSettings};
use estream_audio::{AudioSource, Block};
use estream_core::{SourceStatus, Timestamp};
use estream_engine::{Property, PropertySchema, SelectOption, SourceRegistry, SourceType};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// Genres and moods offered in the settings: (Jamendo tag, label).
pub const GENRES: &[(&str, &str)] = &[
    ("lofi", "Lo-fi"),
    ("chillout", "Chill"),
    ("electronic", "Electronic"),
    ("ambient", "Ambient"),
    ("rock", "Rock"),
    ("hiphop", "Hip-hop"),
    ("jazz", "Jazz"),
    ("classical", "Classical"),
    ("acoustic", "Acoustic"),
    ("soundtrack", "Cinematic"),
];

/// Tracks fetched per request.
const PAGE: usize = 50;

// ---------------------------------------------------------------- configuration

struct Config {
    client_id: Option<String>,
    credit_dir: PathBuf,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Sets the Jamendo client ID (`None` in builds without one) and the folder for credit files.
/// Call once at startup, before sources are created.
pub fn configure(client_id: Option<&str>, credit_dir: PathBuf) {
    let client_id = client_id.map(str::trim).filter(|s| !s.is_empty());
    let _ = CONFIG.set(Config {
        client_id: client_id.map(str::to_owned),
        credit_dir,
    });
}

fn client_id() -> Option<&'static str> {
    CONFIG.get()?.client_id.as_deref()
}

/// Registers the music source type.
pub fn register(registry: &mut SourceRegistry) {
    registry.register(source_type());
}

pub fn schema() -> PropertySchema {
    PropertySchema::new(vec![
        Property::select(
            "genre",
            "Genre",
            GENRES
                .iter()
                .map(|(tag, label)| SelectOption::new(*tag, *label))
                .collect(),
            GENRES[0].0,
        ),
        Property::text("tags", "Or your own tags", false, "")
            .hint("Used instead of the genre, e.g. piano chillout"),
        Property::bool("safe", "Safe for monetized streams", true).hint(
            "Only licenses that allow commercial use. Off adds NonCommercial tracks, which are \
             for non-monetized streams only",
        ),
        Property::button("skip", "Track", "Skip").hint(&credit_hint()),
    ])
}

/// Tells where the credit goes, so it can be shown on screen.
fn credit_hint() -> String {
    match CONFIG.get() {
        Some(c) => format!(
            "Each track's credit is written to a music-….txt file in {}. To show it on \
             screen, add a Text source with Read text from file and pick that file.",
            c.credit_dir.display()
        ),
        None => String::new(),
    }
}

pub fn source_type() -> SourceType {
    SourceType::audio(
        "music",
        "Music",
        "Creative Commons music from Jamendo by genre or mood, with the credit for each track",
        |_| schema(),
        |settings| Ok(Box::new(MusicAudio::new(settings))),
    )
}

// ---------------------------------------------------------------- Jamendo

/// One track from Jamendo.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    /// Short license name, e.g. "CC BY-SA 3.0".
    pub license: String,
    pub license_url: String,
    /// The MP3 stream.
    pub audio: String,
}

impl Track {
    /// "Title by Artist (CC BY 3.0)": what a Creative Commons license asks to be shown.
    pub fn credit(&self) -> String {
        format!("{} by {} ({})", self.title, self.artist, self.license)
    }
}

/// "CC BY-NC-ND 3.0" from a license URL such as
/// `http://creativecommons.org/licenses/by-nc-nd/3.0/`.
pub fn license_label(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    if url.contains("publicdomain") {
        return "CC0".to_owned();
    }
    match parts.iter().position(|p| *p == "licenses") {
        Some(i) if i + 1 < parts.len() => {
            let kind = parts[i + 1].to_uppercase();
            match parts.get(i + 2) {
                Some(version) => format!("CC {kind} {version}"),
                None => format!("CC {kind}"),
            }
        }
        _ => "Creative Commons".to_owned(),
    }
}

/// Whether a license allows commercial use (not NonCommercial).
pub fn commercial_ok(license_url: &str) -> bool {
    !license_url.to_lowercase().contains("-nc")
}

/// The search request for `tags` (space or comma separated).
fn search_url(client_id: &str, tags: &str, safe: bool, offset: usize) -> String {
    let tags: Vec<String> = tags
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    let mut url = format!(
        "https://api.jamendo.com/v3.0/tracks/?client_id={client_id}&format=json&limit={PAGE}\
         &offset={offset}&audioformat=mp32&order=popularity_total&tags={}",
        tags.join("+")
    );
    if safe {
        url.push_str("&ccnc=false");
    }
    url
}

/// Tracks from a Jamendo search response (Jamendo reports errors in `headers`).
fn parse_tracks(body: &Value, safe: bool) -> Result<Vec<Track>, String> {
    let headers = &body["headers"];
    if headers["status"].as_str() != Some("success") {
        let message = headers["error_message"].as_str().unwrap_or("unknown error");
        return Err(format!("Jamendo: {message}"));
    }
    let text = |t: &Value, k: &str| t[k].as_str().unwrap_or_default().to_owned();
    Ok(body["results"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let license_url = text(t, "license_ccurl");
            Track {
                id: t["id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| t["id"].as_u64().map(|n| n.to_string()))
                    .unwrap_or_default(),
                title: text(t, "name"),
                artist: text(t, "artist_name"),
                license: license_label(&license_url),
                license_url,
                audio: text(t, "audio"),
            }
        })
        // Belt and braces: never play a NonCommercial track in safe mode.
        .filter(|t| !t.audio.is_empty() && (!safe || commercial_ok(&t.license_url)))
        .collect())
}

fn fetch(tags: &str, safe: bool, offset: usize) -> Result<Vec<Track>, String> {
    let client_id = client_id().ok_or("no Jamendo client ID")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent
        .get(&search_url(client_id, tags, safe, offset))
        .call()
        .map_err(|e| format!("can't reach Jamendo ({e})"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("Jamendo answered {status}"));
    }
    let body: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Jamendo sent something unexpected ({e})"))?;
    parse_tracks(&body, safe)
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Debug, PartialEq)]
struct MusicSettings {
    tags: String,
    safe: bool,
    skip: i64,
}

impl MusicSettings {
    fn from_json(settings: &Value) -> Self {
        let custom = settings["tags"].as_str().unwrap_or_default().trim();
        let genre = settings["genre"].as_str().unwrap_or(GENRES[0].0);
        Self {
            tags: if custom.is_empty() { genre } else { custom }.to_owned(),
            safe: settings["safe"].as_bool().unwrap_or(true),
            skip: settings["skip"].as_i64().unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------- controller

/// What the user asked the controller to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Request {
    Next,
    Previous,
    /// Play this track of the playlist.
    Play(usize),
}

#[derive(Default)]
struct State {
    settings: Option<MusicSettings>,
    /// The fetched page, shuffled, and the playing track's place in it.
    playlist: Vec<Track>,
    pos: Option<usize>,
    /// Why nothing is playing (network trouble, no results).
    problem: Option<String>,
    request: Option<Request>,
    /// Play the current track again instead of moving on (the Repeat button).
    repeat: bool,
}

/// Fetches and sequences tracks for one music source, on its own thread.
pub struct Controller {
    player: Arc<Player>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl Controller {
    fn new(id: &str, settings: MusicSettings) -> Arc<Self> {
        let player = if id.is_empty() {
            Player::new(PlayerSettings::default())
        } else {
            player::acquire(id, PlayerSettings::default())
        };
        let state = Arc::new(Mutex::new(State {
            settings: Some(settings),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let controller = Arc::new(Self {
            player: player.clone(),
            state: state.clone(),
            stop: stop.clone(),
        });
        let credit_file = credit_file(id);
        let _ = std::thread::Builder::new()
            .name("estream-music".into())
            .spawn(move || run(player, state, stop, credit_file));
        controller
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, settings: MusicSettings) {
        let mut state = self.lock();
        if let Some(old) = &state.settings {
            // The Skip button counts up.
            if settings.skip != old.skip {
                state.request = Some(Request::Next);
            }
        }
        state.settings = Some(settings);
    }

    pub fn skip(&self) {
        self.lock().request = Some(Request::Next);
    }

    pub fn previous(&self) {
        self.lock().request = Some(Request::Previous);
    }

    /// Plays track `index` of the playlist (as [`Controller::window`] numbers them).
    pub fn play(&self, index: usize) {
        self.lock().request = Some(Request::Play(index));
    }

    /// Turns Repeat (play the current track again) on or off.
    pub fn toggle_repeat(&self) {
        let mut state = self.lock();
        state.repeat = !state.repeat;
    }

    pub fn repeat(&self) -> bool {
        self.lock().repeat
    }

    /// The current track's credit, or why nothing plays.
    pub fn now_playing(&self) -> String {
        let state = self.lock();
        let now = state.pos.and_then(|p| state.playlist.get(p));
        match (now, &state.problem) {
            (_, Some(problem)) => problem.clone(),
            (Some(track), None) => track.credit(),
            (None, None) => "Finding music…".to_owned(),
        }
    }

    /// Up to `n` tracks of the playlist around the playing one (up to `before` already played,
    /// then the playing one and what comes next): their credits, the playlist index of the first
    /// (for [`Controller::play`]), and where the playing one is in the list.
    pub fn window(&self, before: usize, n: usize) -> (Vec<String>, usize, Option<usize>) {
        let state = self.lock();
        let start = state.pos.map_or(0, |p| p.saturating_sub(before));
        let tracks = state
            .playlist
            .iter()
            .skip(start)
            .take(n)
            .map(Track::credit)
            .collect();
        (tracks, start, state.pos.map(|p| p - start))
    }

    fn status(&self) -> SourceStatus {
        if client_id().is_none() {
            return SourceStatus::Unavailable(
                "This build of Clutch has no Jamendo client ID, so it can't play music".into(),
            );
        }
        let state = self.lock();
        match (state.pos, &state.problem) {
            (None, Some(problem)) => SourceStatus::Error(problem.clone()),
            _ => SourceStatus::Active,
        }
    }
}

impl Drop for Controller {
    // Sources can be dropped on the real-time threads: only raise the flag; the controller
    // thread removes the credit file as it exits.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Where the current track's credit is written (empty path without a configured folder).
pub fn credit_file(id: &str) -> PathBuf {
    match CONFIG.get() {
        Some(c) if !id.is_empty() => c.credit_dir.join(format!("music-{id}.txt")),
        _ => PathBuf::new(),
    }
}

/// Shuffles in place (xorshift seeded from the clock; good enough for a playlist).
fn shuffle<T>(items: &mut [T]) {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9, |d| d.as_nanos() as u64)
        | 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// Jamendo sometimes answers a good search with no tracks (about one time in three), then
/// answers it properly a moment later: ask again a few times before believing it.
fn fetch_patiently(tags: &str, safe: bool, offset: usize) -> Result<Vec<Track>, String> {
    let mut last = Ok(Vec::new());
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(600));
        }
        last = fetch(tags, safe, offset);
        match &last {
            Ok(tracks) if tracks.is_empty() => {
                tracing::debug!(tags, offset, attempt, "music: Jamendo returned no tracks");
            }
            _ => return last,
        }
    }
    last
}

/// What to play next.
#[derive(Debug, PartialEq)]
enum Next {
    /// This track of the playlist (past its end: the next page).
    Index(usize),
    /// The same track again.
    Replay,
}

/// Decides what the controller does: `pos` is the playing track, `finished` says it ended.
fn next_step(
    request: Option<Request>,
    pos: Option<usize>,
    finished: bool,
    repeat: bool,
) -> Option<Next> {
    match (request, pos) {
        (Some(Request::Play(i)), _) => Some(Next::Index(i)),
        (Some(Request::Next), Some(p)) => Some(Next::Index(p + 1)),
        (Some(Request::Previous), Some(0)) => Some(Next::Replay),
        (Some(Request::Previous), Some(p)) => Some(Next::Index(p - 1)),
        (_, None) => Some(Next::Index(0)),
        (None, Some(_)) if finished && repeat => Some(Next::Replay),
        (None, Some(p)) if finished => Some(Next::Index(p + 1)),
        (None, Some(_)) => None,
    }
}

fn run(player: Arc<Player>, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>, credit_file: PathBuf) {
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let mut offset = 0usize;
    let mut query: Option<(String, bool)> = None; // (tags, safe) of the playlist
    let mut started = Instant::now();
    let mut retry_at: Option<Instant> = None;
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(250));
        if client_id().is_none() {
            continue;
        }
        let (settings, request) = {
            let mut s = lock();
            (s.settings.clone(), s.request.take())
        };
        let Some(settings) = settings else { continue };
        let wanted = (settings.tags.clone(), settings.safe);
        if query.as_ref() != Some(&wanted) {
            // Another genre, tags or license filter: a new playlist, and the old genre's track
            // stops now (it would otherwise play on while the new one loads).
            if query.is_some() {
                player.update(PlayerSettings::default());
            }
            let mut s = lock();
            s.playlist.clear();
            s.pos = None;
            s.problem = None;
            offset = 0;
            query = Some(wanted);
            retry_at = None;
        }
        let status = player.status();
        // A just-started track still reports the previous one's end for a moment.
        let settled = started.elapsed() > Duration::from_millis(1500);
        let finished = settled
            && (status.state == PlayState::Ended
                || (status.state == PlayState::Idle && status.error.is_some()));
        let (pos, repeat) = {
            let s = lock();
            (s.pos, s.repeat)
        };
        let Some(step) = next_step(request, pos, finished, repeat) else {
            continue;
        };
        if step == Next::Replay {
            player.set_paused(false);
            player.restart();
            started = Instant::now();
            continue;
        }
        let Next::Index(mut index) = step else {
            continue;
        };
        let len = lock().playlist.len();
        if index >= len {
            // Past the end (or nothing yet): the next page of tracks.
            if retry_at.is_some_and(|t| Instant::now() < t) {
                continue;
            }
            let page = if len == 0 { 0 } else { offset };
            let mut result = fetch_patiently(&settings.tags, settings.safe, page);
            if matches!(&result, Ok(t) if t.is_empty()) && page > 0 {
                // Played through every page: start over.
                result = fetch_patiently(&settings.tags, settings.safe, 0);
                offset = 0;
            }
            match result {
                Ok(mut tracks) if !tracks.is_empty() => {
                    shuffle(&mut tracks);
                    offset = if page == 0 && offset == 0 {
                        PAGE
                    } else {
                        offset + PAGE
                    };
                    let mut s = lock();
                    s.playlist = tracks;
                    s.pos = None;
                    index = 0;
                    retry_at = None;
                }
                Ok(_) => {
                    tracing::info!(tags = %settings.tags, "music: no tracks found");
                    lock().problem = Some(format!(
                        "No music found for \"{}\". Try another genre or other tags.",
                        settings.tags
                    ));
                    retry_at = Some(Instant::now() + Duration::from_secs(20));
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "music: couldn't load the playlist");
                    lock().problem = Some(format!("Couldn't load music: {e}. Trying again…"));
                    retry_at = Some(Instant::now() + Duration::from_secs(10));
                    continue;
                }
            }
        }
        let Some(track) = lock().playlist.get(index).cloned() else {
            continue;
        };
        tracing::info!(track = %track.credit(), "music: now playing");
        player.set_paused(false);
        player.update(PlayerSettings {
            items: vec![PathBuf::from(&track.audio)],
            looping: false,
            shuffle: false,
            image_seconds: 5.0,
        });
        started = Instant::now();
        if !credit_file.as_os_str().is_empty() {
            if let Some(dir) = credit_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&credit_file, format!("Now playing: {}", track.credit()));
        }
        let mut s = lock();
        s.pos = Some(index);
        s.problem = None;
    }
    if !credit_file.as_os_str().is_empty() {
        let _ = std::fs::remove_file(&credit_file);
    }
}

fn registry() -> &'static Mutex<HashMap<String, Weak<Controller>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<Controller>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn acquire(id: &str, settings: MusicSettings) -> Arc<Controller> {
    if id.is_empty() {
        return Controller::new(id, settings);
    }
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, w| w.strong_count() > 0);
    if let Some(c) = map.get(id).and_then(Weak::upgrade) {
        c.update(settings);
        return c;
    }
    let c = Controller::new(id, settings);
    map.insert(id.to_owned(), Arc::downgrade(&c));
    c
}

/// The controller of music source `id`, while it exists.
pub fn get(id: &str) -> Option<Arc<Controller>> {
    registry().lock().ok()?.get(id)?.upgrade()
}

// ---------------------------------------------------------------- audio side

pub struct MusicAudio {
    controller: Arc<Controller>,
    audio: MediaAudio,
}

impl MusicAudio {
    fn new(settings: &Value) -> Self {
        let id = settings["_source_id"].as_str().unwrap_or_default();
        let controller = acquire(id, MusicSettings::from_json(settings));
        let audio = MediaAudio::from_player(controller.player.clone());
        Self { controller, audio }
    }
}

impl AudioSource for MusicAudio {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        self.controller.update(MusicSettings::from_json(settings));
        Ok(())
    }

    fn read(&mut self, block: &mut Block, at: Timestamp) -> usize {
        self.audio.read(block, at)
    }

    fn status(&self) -> SourceStatus {
        self.controller.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn license_labels() {
        assert_eq!(
            license_label("http://creativecommons.org/licenses/by-nc-nd/3.0/"),
            "CC BY-NC-ND 3.0"
        );
        assert_eq!(
            license_label("http://creativecommons.org/licenses/by/3.0/"),
            "CC BY 3.0"
        );
        assert_eq!(
            license_label("https://creativecommons.org/publicdomain/zero/1.0/"),
            "CC0"
        );
        assert!(commercial_ok(
            "http://creativecommons.org/licenses/by-sa/3.0/"
        ));
        assert!(!commercial_ok(
            "http://creativecommons.org/licenses/by-nc-sa/3.0/"
        ));
    }

    #[test]
    fn search_url_has_tags_and_filter() {
        let url = search_url("ID", "Lofi  piano,chill", true, 50);
        assert!(url.contains("client_id=ID"), "{url}");
        assert!(url.contains("tags=lofi+piano+chill"), "{url}");
        assert!(url.contains("offset=50"), "{url}");
        assert!(url.contains("ccnc=false"), "{url}");
        assert!(!search_url("ID", "rock", false, 0).contains("ccnc"));
    }

    // A trimmed response recorded from the Jamendo API.
    fn response() -> Value {
        json!({
            "headers": {"status": "success", "code": 0, "error_message": "", "results_count": 2},
            "results": [
                {"id": "1593988", "name": "Lofi Chillout Hip Hop Beat", "artist_name": "Joystock",
                 "license_ccurl": "http://creativecommons.org/licenses/by-nc-nd/3.0/",
                 "audio": "https://prod-1.storage.jamendo.com/?trackid=1593988&format=mp32"},
                {"id": "1", "name": "Beach Sunset", "artist_name": "Loufo",
                 "license_ccurl": "http://creativecommons.org/licenses/by-sa/3.0/",
                 "audio": "https://prod-1.storage.jamendo.com/?trackid=1&format=mp32"}
            ]
        })
    }

    #[test]
    fn parses_tracks_and_drops_noncommercial_when_safe() {
        let all = parse_tracks(&response(), false).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].credit(), "Beach Sunset by Loufo (CC BY-SA 3.0)");
        let safe = parse_tracks(&response(), true).unwrap();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].title, "Beach Sunset");
    }

    #[test]
    fn reports_jamendo_errors() {
        let body = json!({"headers": {"status": "failed", "error_message": "Invalid client_id"}});
        assert_eq!(
            parse_tracks(&body, true).unwrap_err(),
            "Jamendo: Invalid client_id"
        );
    }

    #[test]
    fn next_track_decisions() {
        use Next::*;
        // Nothing playing yet: the first track.
        assert_eq!(next_step(None, None, false, false), Some(Index(0)));
        // Playing and not finished: nothing to do.
        assert_eq!(next_step(None, Some(3), false, false), None);
        // Finished: the next one, or the same again with Repeat.
        assert_eq!(next_step(None, Some(3), true, false), Some(Index(4)));
        assert_eq!(next_step(None, Some(3), true, true), Some(Replay));
        // Skip moves on even with Repeat; Previous goes back (or restarts the first).
        assert_eq!(
            next_step(Some(Request::Next), Some(3), false, true),
            Some(Index(4))
        );
        assert_eq!(
            next_step(Some(Request::Previous), Some(3), false, false),
            Some(Index(2))
        );
        assert_eq!(
            next_step(Some(Request::Previous), Some(0), false, false),
            Some(Replay)
        );
        // Picking a track from the list plays it.
        assert_eq!(
            next_step(Some(Request::Play(7)), Some(3), false, false),
            Some(Index(7))
        );
    }

    #[test]
    fn custom_tags_win_over_the_genre() {
        let s =
            MusicSettings::from_json(&json!({"genre": "jazz", "tags": " piano ", "safe": false}));
        assert_eq!(s.tags, "piano");
        assert!(!s.safe);
        let s = MusicSettings::from_json(&json!({"genre": "jazz"}));
        assert_eq!(s.tags, "jazz");
        assert!(s.safe);
    }
}
