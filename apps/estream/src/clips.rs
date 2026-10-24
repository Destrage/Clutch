//! Clips in the studio (the `add-clips` change): clip mode, the clip hotkey, auto-clip from
//! game integrations and hype, and the clip library.
//!
//! [`Clipper::tick`] runs from the app's UI tick and never blocks: saving, thumbnails and the
//! integrations all work on their own threads and report back through channels.

use crate::settings::ClipSettings;
use estream_clips::integrations::GameEvent;
use estream_clips::integrations::gsi::{self, GsiServer};
use estream_clips::integrations::league::LeaguePoller;
use estream_clips::integrations::replay::ReplayWatcher;
use estream_clips::integrations::replay::pubg::Pubg;
use estream_clips::library::{ClipMark, ClipMeta, Library, Trigger};
use estream_clips::{
    ClipEvent, ClipRequest, Detection, Detector, EventKind, GameList, HypeConfig, HypeDetector,
    Integration, Scheduler, Window,
};
use estream_core::{EngineClock, SourceId, Timestamp};
use estream_engine::Engine;
use estream_gpu::GpuContext;
use estream_media::{Container, MediaError, RecordingSettings, ReplayBuffer, SavedClip};
use estream_project::GameEventKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

/// How often the foreground window is checked for a game.
const DETECT_EVERY: Duration = Duration::from_millis(250);
/// Give up on a save after this long.
const SAVE_TIMEOUT: Duration = Duration::from_secs(60);
/// After the game exits, pending clips get this long to save before the buffer stops.
const STOP_GRACE: Duration = Duration::from_secs(15);

/// Something the app should show or act on.
#[derive(Debug)]
pub enum Notice {
    Saved {
        path: PathBuf,
        title: String,
    },
    Failed(String),
    GameStarted(String),
    GameStopped(String),
    /// An unknown fullscreen program: offer to treat it as a game.
    AskIsGame {
        exe: String,
        title: String,
    },
    /// A game event or hype peak, for macros and recording markers.
    Event {
        game: Option<String>,
        kind: EventKind,
        at: Instant,
    },
    /// The clip hotkey is taken by another program.
    #[cfg_attr(not(windows), allow(dead_code))]
    HotkeyConflict(String),
    /// A clip's thumbnail is ready (the library entry has it now).
    Thumbnail,
}

/// Where clip mode's replay buffer runs.
enum Buffer {
    /// On the studio's engine: a program scene already shows the game.
    Studio(ReplayBuffer),
    /// On a small engine of its own that captures the game and audio directly, so clips work
    /// without any scene set up. Never saved.
    Own {
        engine: Engine,
        replay: ReplayBuffer,
    },
}

/// What a save is waiting on.
struct Saving {
    rx: crossbeam_channel::Receiver<Result<SavedClip, MediaError>>,
    window: Window,
    /// The clock of the engine the clip comes from (its times are on it).
    buffer_clock: EngineClock,
    started: Instant,
}

/// Everything the clipper needs from the app for one tick.
pub struct Ctx<'a> {
    pub studio: Option<&'a Engine>,
    /// The replay buffer the user started by hand, if any.
    pub studio_replay: Option<&'a ReplayBuffer>,
    pub settings: &'a ClipSettings,
    pub recording: &'a RecordingSettings,
    pub gpu: &'a GpuContext,
}

pub struct Clipper {
    pub library: Library,
    /// One clock for every pending window; converted to the buffer's engine clock on save.
    clock: EngineClock,
    scheduler: Scheduler,
    games: GameList,
    detector: Detector,
    detect_at: Instant,
    buffer: Option<Buffer>,
    /// When the game stopped (clip mode stops its buffer once pending clips are saved).
    stopping: Option<Instant>,
    hype: HashMap<SourceId, HypeDetector>,
    /// The last hype clip, from any source.
    last_hype: Option<Instant>,
    events_tx: Sender<GameEvent>,
    events_rx: Receiver<GameEvent>,
    gsi: Option<GsiServer>,
    /// What the GSI server was started with (restart when it changes).
    gsi_config: Option<(u16, String, String)>,
    league: Option<LeaguePoller>,
    /// Reads PUBG's replays while it runs (its events arrive when the player's match ends).
    pubg: Option<ReplayWatcher>,
    hotkey: Option<Box<dyn estream_automation::hotkeys::GlobalHotkeys>>,
    hotkey_chord: Option<estream_project::Chord>,
    /// Whether the last hotkey registration's outcome was reported (some systems answer
    /// after asking the user).
    hotkey_reported: bool,
    /// Clip hotkey presses Clutch's own window saw (where there are no system hotkeys).
    in_app_presses: u32,
    saving: Vec<Saving>,
    thumbs_tx: Sender<PathBuf>,
    thumbs_rx: Receiver<PathBuf>,
    /// Clips saved since Clutch started.
    pub session_clips: u32,
    /// The settings the clipper was last configured with.
    applied: Option<ClipSettings>,
}

/// Maps a clip event to the trigger kind macros and rules use.
pub fn trigger_kind(kind: &EventKind) -> Option<GameEventKind> {
    Some(match kind {
        EventKind::Manual => return None,
        EventKind::Kill => GameEventKind::Kill,
        EventKind::Knock => GameEventKind::Knock,
        EventKind::MultiKill { .. } => GameEventKind::MultiKill,
        EventKind::Ace => GameEventKind::Ace,
        EventKind::Death => GameEventKind::Death,
        EventKind::RoundWin => GameEventKind::RoundWin,
        EventKind::MatchWin => GameEventKind::MatchWin,
        EventKind::Goal => GameEventKind::Goal,
        EventKind::Objective { .. } => GameEventKind::Objective,
        EventKind::Hype => GameEventKind::Hype,
    })
}

/// Captures whose own audio is the game's sound (preferred for hype), and Desktop audio, used
/// when no capture has its audio on. Application audio (voice chat, music) never counts.
const CAPTURE_AUDIO: &[&str] = &["game_capture", "window_capture"];
const DESKTOP_AUDIO: &str = "audio_output";
const MIC_AUDIO: &[&str] = &["audio_input"];
/// Hype clips from any source are at least this far apart.
const HYPE_COOLDOWN: Duration = Duration::from_secs(45);
/// Source kinds that show a game on screen.
const GAME_VIDEO: &[&str] = &["game_capture", "display_capture", "window_capture"];

impl Clipper {
    pub fn new(settings: &ClipSettings, recording: &RecordingSettings) -> Self {
        let (events_tx, events_rx) = channel();
        let (thumbs_tx, thumbs_rx) = channel();
        let mut clipper = Self {
            library: Library::open(settings.folder(recording)),
            clock: EngineClock::new(),
            scheduler: Scheduler::new(Duration::from_secs(settings.max_seconds as u64)),
            games: GameList::new(settings.user_games.clone(), settings.not_games.clone()),
            detector: Detector::default(),
            detect_at: Instant::now(),
            buffer: None,
            stopping: None,
            hype: HashMap::new(),
            last_hype: None,
            events_tx,
            events_rx,
            gsi: None,
            gsi_config: None,
            league: None,
            pubg: None,
            hotkey: estream_automation::hotkeys::start(),
            hotkey_chord: None,
            hotkey_reported: true,
            in_app_presses: 0,
            saving: Vec::new(),
            thumbs_tx,
            thumbs_rx,
            session_clips: 0,
            applied: None,
        };
        clipper.library.refresh();
        clipper
    }

    /// The game being played, if any.
    pub fn current_game(&self) -> Option<&str> {
        self.detector.current().map(|g| g.name.as_str())
    }

    /// Makes `clip`'s thumbnail from the frame at `at` seconds, in the background; the library
    /// picks it up (and a [`Notice::Thumbnail`] goes out) on a later tick.
    pub fn make_thumbnail(&self, clip: PathBuf, at: f64) {
        spawn_thumbnail(clip, at, self.thumbs_tx.clone());
    }

    /// Whether clip mode's own buffer is running.
    pub fn buffering(&self) -> bool {
        self.buffer.is_some()
    }

    /// When an integration last heard from its game (GSI only).
    pub fn last_seen(&self, integration: Integration) -> Option<Instant> {
        self.gsi.as_ref()?.last_seen(integration)
    }

    pub fn gsi_port(&self) -> Option<u16> {
        self.gsi.as_ref().map(|g| g.port)
    }

    /// A key press in Clutch's window: the clip hotkey when the system has no global
    /// hotkeys. `true` if it was the clip hotkey.
    pub fn press_in_app(&mut self, chord: &estream_project::Chord) -> bool {
        let hit = self.hotkey.is_none() && self.hotkey_chord.as_ref() == Some(chord);
        if hit {
            self.in_app_presses += 1;
        }
        hit
    }

    /// Picks up settings changes (folder, hotkey, integrations, game lists).
    fn configure(
        &mut self,
        s: &ClipSettings,
        recording: &RecordingSettings,
        out: &mut Vec<Notice>,
    ) {
        if self.applied.as_ref() == Some(s) {
            return;
        }
        let folder = s.folder(recording);
        if folder != self.library.root() {
            self.library = Library::open(folder);
        }
        self.scheduler
            .set_max(Duration::from_secs(s.max_seconds.max(10) as u64));
        self.games = GameList::new(s.user_games.clone(), s.not_games.clone());

        let chord = estream_project::Chord::parse(&s.hotkey);
        if chord != self.hotkey_chord {
            self.hotkey_chord = chord.clone();
            if let Some(listener) = &self.hotkey {
                listener.register(
                    chord
                        .iter()
                        .map(|c| estream_automation::hotkeys::Hotkey {
                            id: 1,
                            chord: c.clone(),
                            label: "Save a clip".into(),
                        })
                        .collect(),
                );
                self.hotkey_reported = false;
            }
        }

        // The GSI listener runs while CS2 or Dota 2 is connected.
        let wants_gsi = s
            .connected
            .iter()
            .any(|i| matches!(i, Integration::Cs2Gsi | Integration::Dota2Gsi));
        let config = (s.gsi_port, s.cs2_token.clone(), s.dota2_token.clone());
        if !wants_gsi {
            self.gsi = None;
            self.gsi_config = None;
        } else if self.gsi_config.as_ref() != Some(&config) {
            self.gsi = None;
            let tokens = gsi::Tokens::from([
                (Integration::Cs2Gsi, s.cs2_token.clone()),
                (Integration::Dota2Gsi, s.dota2_token.clone()),
            ]);
            match GsiServer::start(s.gsi_port, tokens, self.events_tx.clone()) {
                Ok(server) => self.gsi = Some(server),
                Err(e) => out.push(Notice::Failed(format!("Game integrations: {e}"))),
            }
            self.gsi_config = Some(config);
        }
        if !s.connected.contains(&Integration::LeagueLiveClient) {
            self.league = None;
        }
        if !s.connected.contains(&Integration::PubgReplay) {
            self.pubg = None;
        }
        if !(s.hype_game || s.hype_mic) {
            self.hype.clear();
        }
        self.applied = Some(s.clone());
    }

    /// Asks for a clip of `pre` before now and `post` after, for `kind` (e.g. the hotkey).
    /// Fails when no replay buffer is running.
    pub fn request(
        &mut self,
        kind: EventKind,
        game: Option<String>,
        at: Instant,
        pre: Duration,
        post: Duration,
        studio_replay: bool,
    ) -> Result<(), String> {
        if self.buffer.is_none() && !studio_replay {
            return Err(
                "Clips need the replay buffer: turn on clip mode or start the replay buffer".into(),
            );
        }
        self.scheduler.request(ClipRequest {
            event: ClipEvent {
                kind,
                at: self.clock.timestamp_of(at),
                game: game.or_else(|| self.current_game().map(str::to_owned)),
            },
            pre,
            post,
        });
        Ok(())
    }

    pub fn tick(&mut self, ctx: Ctx<'_>) -> Vec<Notice> {
        let mut out = Vec::new();
        let now = Instant::now();
        let s = ctx.settings;
        self.configure(s, ctx.recording, &mut out);

        if now.duration_since(self.detect_at) >= DETECT_EVERY {
            self.detect_at = now;
            self.detect(&ctx, now, &mut out);
        }

        // The clip hotkey.
        let mut presses = std::mem::take(&mut self.in_app_presses);
        if let Some(listener) = &self.hotkey {
            while listener.try_press().is_some() {
                presses += 1;
            }
            if !self.hotkey_reported && !listener.failed().is_empty() {
                self.hotkey_reported = true;
                out.push(Notice::HotkeyConflict(s.hotkey.clone()));
            }
        }
        for _ in 0..presses {
            let pre = Duration::from_secs(s.hotkey_seconds.clamp(5, 600) as u64);
            if let Err(e) = self.request(
                EventKind::Manual,
                None,
                now,
                pre,
                Duration::ZERO,
                ctx.studio_replay.is_some(),
            ) {
                out.push(Notice::Failed(e));
            }
        }

        // Game events from the integrations.
        while let Ok(event) = self.events_rx.try_recv() {
            self.on_event(&ctx, event.kind, Some(event.game), event.at, &mut out);
        }
        self.tick_hype(&ctx, now, &mut out);

        // Save what's due, on whichever buffer is running.
        let due_at = self.clock.timestamp_of(now);
        for window in self.scheduler.due(due_at) {
            self.start_save(&ctx, window, &mut out);
        }
        self.poll_saves(now, &mut out);
        while let Ok(path) = self.thumbs_rx.try_recv() {
            self.library.reload(&path);
            out.push(Notice::Thumbnail);
        }

        // Clip mode's buffer stops once the game has gone and pending clips are saved.
        if let Some(since) = self.stopping
            && ((self.scheduler.pending().is_empty() && self.saving.is_empty())
                || now.duration_since(since) >= STOP_GRACE)
        {
            self.stopping = None;
            self.stop_buffer(ctx.studio);
        }
        out
    }

    fn detect(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<Notice>) {
        let fg = estream_clips::platform::foreground();
        for d in self.detector.update(
            &self.games,
            now,
            fg.as_ref(),
            &estream_clips::platform::is_alive,
        ) {
            match d {
                Detection::Started(game) => {
                    tracing::info!(game = %game.name, "game started");
                    self.stopping = None;
                    if ctx.settings.mode && self.buffer.is_none() {
                        match self.start_buffer(ctx) {
                            Ok(()) => {}
                            Err(e) => out.push(Notice::Failed(format!("Clip mode: {e}"))),
                        }
                    }
                    if game.integration == Some(Integration::LeagueLiveClient)
                        && ctx
                            .settings
                            .connected
                            .contains(&Integration::LeagueLiveClient)
                        && self.league.is_none()
                    {
                        match LeaguePoller::start(self.events_tx.clone()) {
                            Ok(p) => self.league = Some(p),
                            Err(e) => tracing::warn!(error = %e, "League poller didn't start"),
                        }
                    }
                    out.push(Notice::GameStarted(game.name));
                }
                Detection::Stopped(game) => {
                    tracing::info!(game = %game.name, "game stopped");
                    if game.integration == Some(Integration::LeagueLiveClient) {
                        self.league = None;
                    }
                    if game.integration == Some(Integration::PubgReplay) {
                        // Dropping it reads a replay written just before the game closed.
                        self.pubg = None;
                    }
                    if self.buffer.is_some() {
                        self.stopping = Some(now);
                    }
                    self.hype.clear();
                    out.push(Notice::GameStopped(game.name));
                }
                Detection::AskIsGame { exe, title } => out.push(Notice::AskIsGame { exe, title }),
            }
        }
        // PUBG's replay watcher runs while PUBG does, including when it was connected with the
        // game already open.
        if self.pubg.is_none()
            && ctx.settings.connected.contains(&Integration::PubgReplay)
            && self
                .detector
                .current()
                .is_some_and(|g| g.integration == Some(Integration::PubgReplay))
        {
            match ReplayWatcher::start(Box::new(Pubg), self.events_tx.clone()) {
                Some(Ok(w)) => {
                    tracing::info!("PUBG replay watcher started");
                    self.pubg = Some(w);
                }
                Some(Err(e)) => tracing::warn!(error = %e, "PUBG replay watcher didn't start"),
                None => {}
            }
        }
        // Clip mode switched on while a game is already running.
        if ctx.settings.mode
            && self.buffer.is_none()
            && self.stopping.is_none()
            && self.detector.current().is_some()
            && let Err(e) = self.start_buffer(ctx)
        {
            out.push(Notice::Failed(format!("Clip mode: {e}")));
        }
        if !ctx.settings.mode && self.buffer.is_some() && self.stopping.is_none() {
            self.stopping = Some(now);
        }
    }

    fn on_event(
        &mut self,
        ctx: &Ctx<'_>,
        kind: EventKind,
        game: Option<String>,
        at: Instant,
        out: &mut Vec<Notice>,
    ) {
        out.push(Notice::Event {
            game: game.clone(),
            kind: kind.clone(),
            at,
        });
        let Some(trigger) = trigger_kind(&kind) else {
            return;
        };
        let window = match (&game, trigger) {
            (_, GameEventKind::Hype) => Some(crate::settings::default_window(GameEventKind::Hype)),
            (Some(g), t) => ctx.settings.rule(g, t).map(|r| (r.pre, r.post)),
            (None, _) => None,
        };
        let Some((pre, post)) = window else { return };
        if self.buffer.is_none() && ctx.studio_replay.is_none() {
            return; // nothing to clip from
        }
        // Replay-file events arrive after the match: clip only what clip mode still holds
        // (the recording gets a marker either way).
        let age = Instant::now().saturating_duration_since(at) + Duration::from_secs(pre as u64);
        if self.buffer.is_some() && age > Duration::from_secs(ctx.settings.buffer_seconds as u64) {
            tracing::debug!(kind = %kind.label(), "event is older than the clip buffer");
            return;
        }
        let _ = self.request(
            kind,
            game,
            at,
            Duration::from_secs(pre as u64),
            Duration::from_secs(post as u64),
            ctx.studio_replay.is_some(),
        );
    }

    /// Feeds the meters of the buffer's audio sources to the hype detectors.
    fn tick_hype(&mut self, ctx: &Ctx<'_>, now: Instant, out: &mut Vec<Notice>) {
        let s = ctx.settings;
        if !(s.hype_game || s.hype_mic) {
            return;
        }
        let engine = match &self.buffer {
            Some(Buffer::Own { engine, .. }) => Some(engine),
            Some(Buffer::Studio(_)) => ctx.studio,
            None if ctx.studio_replay.is_some() => ctx.studio,
            None => None,
        };
        let Some(engine) = engine else { return };
        let snapshot = engine.snapshot();
        let meters = engine.audio_meters();
        let t = self.clock.timestamp_of(now).as_secs_f64();
        // The game's own sound when a capture has its audio on; otherwise Desktop audio.
        let capture_audio = meters.sources.iter().any(|(id, _)| {
            snapshot.source(*id).is_some_and(|d| {
                CAPTURE_AUDIO.contains(&d.kind.as_str())
                    && d.settings.get("audio").and_then(|v| v.as_bool()) != Some(false)
            })
        });
        // A game that reports its events live (and is connected) clips those; its sound doesn't
        // also make hype clips (the mic still can). PUBG's replay events arrive only after the
        // match, and some modes (Team Deathmatch) write no replay, so hype stays on there.
        let game_reports_events = self
            .detector
            .current()
            .and_then(|g| g.integration)
            .is_some_and(|i| i != Integration::PubgReplay && s.connected.contains(&i));
        let mut fired = None;
        for (id, meter) in &meters.sources {
            let Some(def) = snapshot.source(*id) else {
                continue;
            };
            let kind = def.kind.as_str();
            let game_sound = if capture_audio {
                CAPTURE_AUDIO.contains(&kind)
            } else {
                kind == DESKTOP_AUDIO
            };
            let cfg = if s.hype_game && game_sound && !game_reports_events {
                HypeConfig::game(s.hype_sensitivity)
            } else if s.hype_mic && MIC_AUDIO.contains(&kind) {
                HypeConfig::mic(s.hype_sensitivity)
            } else {
                continue;
            };
            let detector = self
                .hype
                .entry(*id)
                .or_insert_with(|| HypeDetector::new(cfg));
            if *detector.config() != cfg {
                detector.set_config(cfg);
            }
            let reading = meter.read();
            let rms = reading.rms[0].max(reading.rms[1]);
            // Every detector keeps listening (so its baseline stays current); one clip at most.
            if let Some(peak) = detector.push(t, rms)
                && fired.is_none()
            {
                fired = Some((peak, def.name.clone(), detector.baseline()));
            }
        }
        let Some((peak, source, baseline)) = fired else {
            return;
        };
        if self
            .last_hype
            .is_some_and(|last| now.duration_since(last) < HYPE_COOLDOWN)
        {
            return;
        }
        self.last_hype = Some(now);
        tracing::info!(
            source,
            over_db = peak.over_db,
            baseline_db = baseline.unwrap_or_default(),
            "hype"
        );
        let at = self
            .clock
            .instant_of(Timestamp((peak.at.max(0.0) * 1e9) as u64));
        let game = self.current_game().map(str::to_owned);
        self.on_event(ctx, EventKind::Hype, game, at, out);
    }

    /// Starts clip mode's buffer: on the studio engine when a program scene shows the game,
    /// otherwise on a small engine of its own.
    fn start_buffer(&mut self, ctx: &Ctx<'_>) -> Result<(), String> {
        let settings = RecordingSettings {
            replay_seconds: ctx
                .settings
                .buffer_seconds
                .clamp(15, estream_media::MAX_REPLAY_SECONDS),
            canvas: None,
            ..ctx.recording.clone()
        };
        if ctx.studio_replay.is_some() {
            return Ok(()); // the user's own replay buffer is already running: use it
        }
        if let Some(studio) = ctx.studio
            && shows_game(studio)
        {
            let replay = ReplayBuffer::start(studio, &settings).map_err(|e| e.to_string())?;
            self.buffer = Some(Buffer::Studio(replay));
            tracing::info!("clip mode buffering the studio canvas");
            return Ok(());
        }
        let engine = clip_engine(ctx.gpu)?;
        let replay = ReplayBuffer::start(&engine, &settings).map_err(|e| e.to_string())?;
        self.buffer = Some(Buffer::Own { engine, replay });
        tracing::info!("clip mode buffering its own game capture");
        Ok(())
    }

    fn stop_buffer(&mut self, studio: Option<&Engine>) {
        self.scheduler.clear();
        self.hype.clear();
        match self.buffer.take() {
            Some(Buffer::Studio(replay)) => {
                if let Some(engine) = studio {
                    let _ = replay.stop(engine);
                }
            }
            Some(Buffer::Own { engine, replay }) => {
                let _ = replay.stop(&engine);
                engine.shutdown();
            }
            None => {}
        }
    }

    /// Stops everything (app exit, or a collection switch with the studio engine).
    pub fn shutdown(&mut self, studio: Option<&Engine>) {
        self.stopping = None;
        self.stop_buffer(studio);
        self.gsi = None;
        self.league = None;
    }

    /// The studio engine is about to be replaced: a buffer on it must stop first.
    pub fn studio_engine_closing(&mut self, studio: &Engine) {
        if matches!(self.buffer, Some(Buffer::Studio(_))) {
            self.stop_buffer(Some(studio));
        }
    }

    fn start_save(&mut self, ctx: &Ctx<'_>, window: Window, out: &mut Vec<Notice>) {
        let (replay, buffer_clock) = match &self.buffer {
            Some(Buffer::Studio(r)) => (r, ctx.studio.map(Engine::clock)),
            Some(Buffer::Own { engine, replay }) => (replay, Some(engine.clock())),
            None => match ctx.studio_replay {
                Some(r) => (r, ctx.studio.map(Engine::clock)),
                None => {
                    out.push(Notice::Failed(
                        "The replay buffer stopped before the clip was saved".into(),
                    ));
                    return;
                }
            },
        };
        let Some(buffer_clock) = buffer_clock else {
            return;
        };
        let convert = |t: Timestamp| buffer_clock.timestamp_of(self.clock.instant_of(t));
        let game = window.events.iter().find_map(|e| e.game.clone());
        let title = title_of(&window);
        let path = self.library.new_clip_path(
            game.as_deref(),
            chrono::Utc::now(),
            &title,
            Container::Mp4.extension(),
        );
        let rx = replay.save_range(
            convert(window.start),
            convert(window.end),
            path,
            Container::Mp4,
        );
        self.saving.push(Saving {
            rx,
            window,
            buffer_clock,
            started: Instant::now(),
        });
    }

    fn poll_saves(&mut self, now: Instant, out: &mut Vec<Notice>) {
        let mut i = 0;
        while i < self.saving.len() {
            let result = match self.saving[i].rx.try_recv() {
                Ok(r) => r,
                Err(crossbeam_channel::TryRecvError::Empty)
                    if now.duration_since(self.saving[i].started) < SAVE_TIMEOUT =>
                {
                    i += 1;
                    continue;
                }
                Err(_) => Err(MediaError::Engine("saving the clip timed out".into())),
            };
            let saving = self.saving.remove(i);
            match result {
                Ok(clip) => self.saved(clip, saving.window, saving.buffer_clock, out),
                Err(e) => out.push(Notice::Failed(format!("Couldn't save the clip: {e}"))),
            }
        }
    }

    /// A clip file is written: add it to the library, make its thumbnail, apply the cap.
    fn saved(
        &mut self,
        clip: SavedClip,
        window: Window,
        buffer_clock: EngineClock,
        out: &mut Vec<Notice>,
    ) {
        let auto = window.events.iter().any(|e| e.kind != EventKind::Manual);
        // Event times inside the clip: the event's moment minus the clip's first frame. The
        // two are on different clocks, so compare them as Instants.
        let first_frame = buffer_clock.instant_of(clip.start);
        let events: Vec<ClipMark> = window
            .events
            .iter()
            .filter(|e| e.kind != EventKind::Manual)
            .map(|e| ClipMark {
                kind: e.kind.clone(),
                at: self
                    .clock
                    .instant_of(e.at)
                    .saturating_duration_since(first_frame)
                    .as_secs_f64(),
            })
            .collect();
        let duration = clip.end.as_secs_f64() - clip.start.as_secs_f64();
        let meta = ClipMeta {
            game: window.events.iter().find_map(|e| e.game.clone()),
            created: chrono::Utc::now(),
            trigger: if auto { Trigger::Auto } else { Trigger::Manual },
            events,
            duration,
            ..ClipMeta::default()
        };
        let title = meta.display_title();
        let thumb_at = meta.events.first().map_or(duration / 2.0, |m| m.at);
        if let Err(e) = self.library.add(&clip.path, meta) {
            out.push(Notice::Failed(format!(
                "Couldn't add the clip to the library: {e}"
            )));
            return;
        }
        self.session_clips += 1;
        spawn_thumbnail(clip.path.clone(), thumb_at, self.thumbs_tx.clone());
        if auto && let Some(s) = &self.applied {
            let cap = (s.storage_cap_gb.max(0.5) as f64 * 1e9) as u64;
            for path in self.library.enforce_cap(cap) {
                tracing::info!(path = %path.display(), "old automatic clip deleted (storage cap)");
            }
        }
        out.push(Notice::Saved {
            path: clip.path,
            title,
        });
    }
}

/// The clip's name: its strongest game event ("Clip" when it's only a manual save).
fn title_of(window: &Window) -> String {
    window
        .events
        .iter()
        .filter(|e| e.kind != EventKind::Manual)
        .max_by(|a, b| a.kind.weight().total_cmp(&b.kind.weight()))
        .map_or_else(|| "Clip".to_owned(), |e| e.kind.label())
}

/// Whether a program scene of the studio shows a game, window or display capture.
fn shows_game(engine: &Engine) -> bool {
    let p = engine.snapshot();
    p.canvases.iter().any(|c| {
        let Some(scene) = c.active_scene.and_then(|s| p.scene(s)) else {
            return false;
        };
        let mut found = false;
        scene.walk(&mut |item| {
            if item.visible
                && let Some(def) = item.source_id().and_then(|id| p.source(id))
                && GAME_VIDEO.contains(&def.kind.as_str())
            {
                found = true;
            }
        });
        found
    })
}

/// A small engine that captures the fullscreen game with its sound, desktop audio (which then
/// leaves the game out) and the default mic at 1080p60. It has no collection file, so nothing about it is ever saved.
fn clip_engine(gpu: &GpuContext) -> Result<Engine, String> {
    use estream_core::{Alignment, BoundsMode, ItemTransform, Rational};
    use estream_project::{Project, SceneItem, SourceDef};
    let (w, h) = (1920, 1080);
    let mut project = Project::new_default(w, h, Rational::fps(60));
    let game = SourceDef::new(
        "Game",
        "game_capture",
        serde_json::json!({ "mode": "auto", "cursor": false, "audio": true }),
    );
    let mut item = SceneItem::source(game.id);
    item.transform = ItemTransform {
        bounds_mode: BoundsMode::Fit,
        bounds: [w as f32, h as f32].into(),
        bounds_alignment: Alignment::Center,
        ..Default::default()
    };
    let mut desktop = SourceDef::new(
        "Desktop audio",
        "audio_output",
        serde_json::json!({ "device": "" }),
    );
    desktop.audio.global = true;
    desktop.audio.tracks = 0b1;
    let mut mic = SourceDef::new("Mic", "audio_input", serde_json::json!({ "device": "" }));
    mic.audio.global = true;
    mic.audio.tracks = 0b1;
    project.scenes[0].items.push(item);
    project.sources.extend([game, desktop, mic]);
    let mut registry = estream_engine::SourceRegistry::with_builtins();
    estream_capture::register_all(&mut registry);
    Engine::start(
        project,
        estream_engine::EngineConfig {
            registry,
            gpu: Some(gpu.clone()),
            collection_path: None,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

fn spawn_thumbnail(clip: PathBuf, at: f64, done: Sender<PathBuf>) {
    let spawned = std::thread::Builder::new()
        .name("estream-thumbnail".into())
        .spawn(move || {
            let jpg = estream_clips::library::thumbnail_path(&clip);
            match estream_media::thumbnail::thumbnail(&clip, at, 320, &jpg) {
                Ok(_) => {
                    let _ = done.send(clip);
                }
                Err(e) => tracing::warn!(error = %e, "clip thumbnail failed"),
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the thumbnail thread");
    }
}

/// Plays the "clip saved" sound.
pub fn play_saved_sound() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
        // SAFETY: plays a system sound asynchronously; no pointers involved.
        let _ = unsafe { MessageBeep(MB_ICONASTERISK) };
    }
}

// ------------------------------------------------------------------ the Clips view

/// A background job's progress (0..=1000 = 0..1) and cancel flag.
#[derive(Default)]
struct Progress {
    permille: std::sync::atomic::AtomicU32,
    cancel: std::sync::atomic::AtomicBool,
}

/// A running trim.
struct TrimJob {
    source: PathBuf,
    output: PathBuf,
    replace: bool,
    progress: std::sync::Arc<Progress>,
    done: Receiver<Result<estream_media::trim::Trimmed, String>>,
}

/// A finished trim: the clip, the file written, whether it replaced the clip, and the result.
pub struct TrimDone {
    pub source: PathBuf,
    pub output: PathBuf,
    pub replaced: bool,
    pub result: Result<estream_media::trim::Trimmed, String>,
}

/// The Show filter choice that lists recordings instead of clips.
pub const SHOW_RECORDINGS: usize = 4;

/// State of the Clips view (filters, selection, trim, frame preview, thumbnail cache).
pub struct ClipsUi {
    pub game_filter: usize,
    pub kind_filter: usize,
    pub search: String,
    pub selected: Option<PathBuf>,
    /// Clips shown, in grid order.
    pub visible: Vec<PathBuf>,
    pub trim_in: f64,
    pub trim_out: f64,
    pub precise: bool,
    /// The frame wanted, and the one being made.
    preview_wanted: Option<(PathBuf, f64)>,
    preview_busy: Option<(PathBuf, f64)>,
    preview_rx: Option<Receiver<Result<PathBuf, String>>>,
    pub preview: Option<slint::Image>,
    trim: Option<TrimJob>,
    thumbs: HashMap<PathBuf, (std::time::SystemTime, slint::Image)>,
    pub refreshed_at: Instant,
    /// A recording whose details are being worked out.
    details_rx: Option<Receiver<(PathBuf, Result<ClipMeta, String>)>>,
    /// Recordings whose details couldn't be worked out (not retried).
    pub details_failed: std::collections::HashSet<PathBuf>,
}

impl Default for ClipsUi {
    fn default() -> Self {
        Self {
            game_filter: 0,
            kind_filter: 0,
            search: String::new(),
            selected: None,
            visible: Vec::new(),
            trim_in: 0.0,
            trim_out: 0.0,
            precise: false,
            preview_wanted: None,
            preview_busy: None,
            preview_rx: None,
            preview: None,
            trim: None,
            thumbs: HashMap::new(),
            refreshed_at: Instant::now(),
            details_rx: None,
            details_failed: Default::default(),
        }
    }
}

static PREVIEW_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl ClipsUi {
    /// A clip's thumbnail as an image (cached until the file changes).
    pub fn thumb(&mut self, path: &Path) -> Option<slint::Image> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        if let Some((t, img)) = self.thumbs.get(path)
            && *t == modified
        {
            return Some(img.clone());
        }
        let img = slint::Image::load_from_path(path).ok()?;
        self.thumbs
            .insert(path.to_path_buf(), (modified, img.clone()));
        Some(img)
    }

    /// Selects a clip (or none), resetting the trim to the whole clip.
    pub fn select(&mut self, clip: Option<(&Path, f64)>) {
        self.selected = clip.map(|(p, _)| p.to_path_buf());
        self.trim_in = 0.0;
        self.trim_out = clip.map_or(0.0, |(_, d)| d);
        self.preview = None;
        self.preview_wanted = clip.map(|(p, _)| (p.to_path_buf(), 0.0));
    }

    /// Moves a trim point and asks for its frame.
    pub fn set_trim(&mut self, out: bool, value: f64, duration: f64) {
        let v = value.clamp(0.0, duration.max(0.0));
        if out {
            self.trim_out = v.max(self.trim_in + 0.1).min(duration);
        } else {
            self.trim_in = v.min(self.trim_out - 0.1).max(0.0);
        }
        if let Some(p) = &self.selected {
            let t = if out { self.trim_out } else { self.trim_in };
            self.preview_wanted = Some((p.clone(), t));
        }
    }

    /// Trim progress 0..1, or `None` when no trim is running.
    pub fn trim_progress(&self) -> Option<f32> {
        self.trim.as_ref().map(|t| {
            t.progress
                .permille
                .load(std::sync::atomic::Ordering::Relaxed) as f32
                / 1000.0
        })
    }

    pub fn cancel_trim(&self) {
        if let Some(t) = &self.trim {
            t.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Starts trimming `source` to the current points, into `output` (which is `source`
    /// when replacing).
    pub fn start_trim(&mut self, source: PathBuf, output: PathBuf, replace: bool) {
        if self.trim.is_some() {
            return;
        }
        let (from, to, precise) = (self.trim_in, self.trim_out, self.precise);
        let progress = std::sync::Arc::new(Progress::default());
        let (tx, done) = channel();
        let job_progress = progress.clone();
        let (src, out) = (source.clone(), output.clone());
        let spawned = std::thread::Builder::new()
            .name("estream-trim".into())
            .spawn(move || {
                use std::sync::atomic::Ordering;
                let mode = if precise {
                    estream_media::trim::TrimMode::Precise
                } else {
                    estream_media::trim::TrimMode::Lossless
                };
                let result = estream_media::trim::trim(&src, &out, from, to, mode, &mut |p| {
                    job_progress
                        .permille
                        .store((p.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                    !job_progress.cancel.load(Ordering::Acquire)
                })
                .map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.trim = Some(TrimJob {
                    source,
                    output,
                    replace,
                    progress,
                    done,
                })
            }
            Err(e) => tracing::warn!(error = %e, "couldn't start the trim thread"),
        }
    }

    /// Polls the frame preview and the trim. Returns whether the view should update, and a
    /// trim that just finished.
    pub fn poll(&mut self) -> (bool, Option<TrimDone>) {
        let mut changed = false;
        // The frame preview: one at a time, always heading for the latest wanted.
        if let Some(rx) = &self.preview_rx
            && let Ok(result) = rx.try_recv()
        {
            self.preview_rx = None;
            let made = self.preview_busy.take();
            if let Ok(jpg) = result {
                self.preview = slint::Image::load_from_path(&jpg).ok();
                let _ = std::fs::remove_file(jpg);
            }
            if made == self.preview_wanted {
                self.preview_wanted = None;
            }
            changed = true;
        }
        if self.preview_rx.is_none()
            && let Some((clip, t)) = self.preview_wanted.clone()
        {
            let (tx, rx) = channel();
            let n = PREVIEW_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let jpg = std::env::temp_dir()
                .join(format!("estream-preview-{}-{n}.jpg", std::process::id()));
            let spawned = std::thread::Builder::new()
                .name("estream-preview".into())
                .spawn(move || {
                    let r = estream_media::thumbnail::thumbnail(&clip, t, 640, &jpg)
                        .map(|_| jpg)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(r);
                });
            if spawned.is_ok() {
                self.preview_busy = self.preview_wanted.clone();
                self.preview_rx = Some(rx);
            }
        }
        let finished = match &self.trim {
            Some(job) => match job.done.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err("the trim stopped".into())),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        if self.trim.is_some() {
            changed = true;
        }
        let done = finished.and_then(|result| {
            self.trim.take().map(|job| TrimDone {
                source: job.source,
                output: job.output,
                replaced: job.replace,
                result,
            })
        });
        (changed, done)
    }

    /// Whether a recording's details are being worked out.
    pub fn details_busy(&self) -> bool {
        self.details_rx.is_some()
    }

    /// Works out a recording's length, thumbnail and marker events in the background.
    pub fn start_details(&mut self, recording: PathBuf) {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("estream-recording-details".into())
            .spawn(move || {
                let details = recording_details(&recording);
                let _ = tx.send((recording, details));
            });
        match spawned {
            Ok(_) => self.details_rx = Some(rx),
            Err(e) => tracing::warn!(error = %e, "couldn't start the recording details thread"),
        }
    }

    /// A recording's details, once worked out.
    pub fn poll_details(&mut self) -> Option<(PathBuf, Result<ClipMeta, String>)> {
        let result = match self.details_rx.as_ref()?.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.details_rx = None;
                return None;
            }
        };
        self.details_rx = None;
        if result.1.is_err() {
            self.details_failed.insert(result.0.clone());
        }
        Some(result)
    }
}

/// A recording's details: its length, its game events (from its marker file) and a thumbnail
/// from the middle, written next to it.
fn recording_details(recording: &Path) -> Result<ClipMeta, String> {
    use estream_clips::markers::Markers;
    let duration = estream_media::probe_duration(recording).map_err(|e| e.to_string())?;
    if duration <= 0.0 {
        return Err("the file has no length".into());
    }
    let markers = Markers::load(recording).unwrap_or_default().markers;
    let jpg = estream_clips::library::thumbnail_path(recording);
    if let Err(e) = estream_media::thumbnail::thumbnail(recording, duration / 2.0, 320, &jpg) {
        tracing::warn!(error = %e, "recording thumbnail failed");
    }
    let created = std::fs::metadata(recording)
        .and_then(|m| m.modified())
        .map_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, Into::into);
    Ok(ClipMeta {
        game: markers.iter().find_map(|m| m.game.clone()),
        created,
        events: markers
            .iter()
            .map(|m| ClipMark {
                kind: m.kind.clone(),
                at: m.at,
            })
            .collect(),
        duration,
        ..ClipMeta::default()
    })
}

/// "0:17" or "1:02:05".
pub fn format_length(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// "Today 20:14", "Yesterday 09:03" or "12 Oct 2026 20:14".
pub fn format_when(at: chrono::DateTime<chrono::Utc>) -> String {
    let local = at.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = local.date_naive();
    let time = local.format("%H:%M");
    if day == today {
        format!("Today {time}")
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday {time}")
    } else {
        format!("{} {time}", local.format("%-d %b %Y"))
    }
}
