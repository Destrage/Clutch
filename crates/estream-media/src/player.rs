//! Media playback for media sources. One [`Player`] per source, shared by its video and audio
//! sides through a registry keyed by source id. A helper thread demuxes and decodes the
//! playlist; video frames are published when the playback clock reaches them and audio is
//! pushed slightly ahead into a ring that the audio side drains in real time.

use crate::ffmpeg::{self, format, frame, media, software};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Audio is pushed this far ahead of its time; the audio side delays by the same amount.
pub const AUDIO_LEAD: Duration = Duration::from_millis(100);
const SAMPLE_RATE: u32 = 48_000;
/// Cap on buffered audio (stereo frames) in case nobody reads it.
const AUDIO_CAP: usize = SAMPLE_RATE as usize * 2;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "webm", "avi", "flv", "ts", "m4v", "wmv", "gif",
];
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "wma"];
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"];

fn has_ext(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| list.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn is_media(path: &Path) -> bool {
    has_ext(path, VIDEO_EXTENSIONS)
        || has_ext(path, AUDIO_EXTENSIONS)
        || has_ext(path, IMAGE_EXTENSIONS)
}

// ---------------------------------------------------------------- settings & status

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSettings {
    pub items: Vec<PathBuf>,
    pub looping: bool,
    pub shuffle: bool,
    /// How long still images stay up.
    pub image_seconds: f32,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            looping: true,
            shuffle: false,
            image_seconds: 5.0,
        }
    }
}

impl PlayerSettings {
    /// From media source settings: `file`, `playlist` (lines; folders expand to their media
    /// files, sorted), `loop`, `shuffle`, `image_seconds`.
    pub fn from_json(settings: &serde_json::Value) -> Self {
        let text = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let flag = |k: &str, d: bool| settings.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
        let mut items = Vec::new();
        let lines = std::iter::once(text("file")).chain(text("playlist").lines());
        for line in lines.map(str::trim).filter(|l| !l.is_empty()) {
            let path = PathBuf::from(line.trim_matches('"'));
            if path.is_dir() {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&path)
                    .map(|d| {
                        d.flatten()
                            .map(|e| e.path())
                            .filter(|p| is_media(p))
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                items.extend(files);
            } else {
                items.push(path);
            }
        }
        Self {
            items,
            looping: flag("loop", true),
            shuffle: flag("shuffle", false),
            image_seconds: settings
                .get("image_seconds")
                .and_then(|v| v.as_f64())
                .map_or(5.0, |v| v.clamp(0.5, 3600.0) as f32),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PlayState {
    /// Nothing to play.
    #[default]
    Idle,
    Playing,
    Paused,
    /// Reached the end of the playlist (not looping).
    Ended,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerStatus {
    pub state: PlayState,
    /// File name of the current item.
    pub item: String,
    pub index: usize,
    pub count: usize,
    pub position: Duration,
    pub duration: Option<Duration>,
    /// The most recent item error (e.g. a missing file).
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelLayout {
    /// Tightly packed NV12 (`width` × `height` luma, then interleaved CbCr).
    Nv12,
    /// Straight-alpha RGBA8 (sRGB).
    Rgba,
}

/// The latest decoded picture.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub layout: PixelLayout,
    pub data: Vec<u8>,
    /// Increases with every new picture.
    pub sequence: u64,
    /// BT.709 for HD and up, BT.601 below (by height), limited range unless full.
    pub full_range: bool,
}

// ---------------------------------------------------------------- shared state

enum Command {
    Settings(PlayerSettings),
    Pause(bool),
    Restart,
    Next,
    Previous,
    Seek(Duration),
}

struct Shared {
    picture: Mutex<Option<Picture>>,
    sequence: AtomicU64,
    /// Interleaved stereo f32 at 48 kHz.
    audio: Mutex<VecDeque<f32>>,
    status: Mutex<PlayerStatus>,
    paused: AtomicBool,
    stop: AtomicBool,
}

pub struct Player {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
    settings: Mutex<PlayerSettings>,
}

impl Player {
    pub fn new(settings: PlayerSettings) -> Arc<Self> {
        crate::init();
        let shared = Arc::new(Shared {
            picture: Mutex::new(None),
            sequence: AtomicU64::new(0),
            audio: Mutex::new(VecDeque::new()),
            status: Mutex::new(PlayerStatus::default()),
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let (tx, rx) = unbounded();
        let thread = {
            let (shared, settings) = (shared.clone(), settings.clone());
            std::thread::Builder::new()
                .name("estream-media-player".into())
                .spawn(move || Decoder::new(shared, rx, settings).run())
                .ok()
        };
        Arc::new(Self {
            shared,
            commands: tx,
            thread: Mutex::new(thread),
            settings: Mutex::new(settings),
        })
    }

    /// Applies new settings (restarts playback only if they changed).
    pub fn update(&self, settings: PlayerSettings) {
        if let Ok(mut current) = self.settings.lock()
            && *current != settings
        {
            *current = settings.clone();
            let _ = self.commands.send(Command::Settings(settings));
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.commands.send(Command::Pause(paused));
    }

    pub fn toggle_pause(&self) {
        self.set_paused(!self.shared.paused.load(Ordering::Acquire));
    }

    pub fn restart(&self) {
        let _ = self.commands.send(Command::Restart);
    }

    pub fn next(&self) {
        let _ = self.commands.send(Command::Next);
    }

    pub fn previous(&self) {
        let _ = self.commands.send(Command::Previous);
    }

    pub fn seek(&self, position: Duration) {
        let _ = self.commands.send(Command::Seek(position));
    }

    pub fn status(&self) -> PlayerStatus {
        self.shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Sequence of the latest picture (0 = none yet).
    pub fn sequence(&self) -> u64 {
        self.shared.sequence.load(Ordering::Acquire)
    }

    /// The latest picture without waiting (`None` if the decoder holds the lock).
    pub fn try_picture(&self) -> Option<MutexGuard<'_, Option<Picture>>> {
        self.shared.picture.try_lock().ok()
    }

    /// Takes up to `left.len()` stereo frames of audio. Never waits; returns 0 if the
    /// decoder holds the lock, while paused, or if nothing is buffered.
    pub fn read_audio(&self, left: &mut [f32], right: &mut [f32]) -> usize {
        if self.shared.paused.load(Ordering::Acquire) {
            return 0;
        }
        let Ok(mut ring) = self.shared.audio.try_lock() else {
            return 0;
        };
        let n = left.len().min(right.len()).min(ring.len() / 2);
        for i in 0..n {
            left[i] = ring.pop_front().unwrap_or(0.0);
            right[i] = ring.pop_front().unwrap_or(0.0);
        }
        n
    }

    /// Stereo frames currently buffered.
    pub fn buffered_audio(&self) -> usize {
        self.shared.audio.try_lock().map_or(0, |r| r.len() / 2)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Players can be dropped on the real-time threads (when a source goes away), so
        // don't wait: the decode thread sees the flag within a few milliseconds and exits.
        self.shared.stop.store(true, Ordering::Release);
        if let Ok(mut t) = self.thread.lock() {
            drop(t.take());
        }
    }
}

// ---------------------------------------------------------------- registry

fn registry() -> &'static Mutex<HashMap<String, Weak<Player>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<Player>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// The player for source `id`, created (or updated) with `settings`.
pub fn acquire(id: &str, settings: PlayerSettings) -> Arc<Player> {
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, w| w.strong_count() > 0);
    if let Some(player) = map.get(id).and_then(Weak::upgrade) {
        player.update(settings);
        return player;
    }
    let player = Player::new(settings);
    map.insert(id.to_owned(), Arc::downgrade(&player));
    player
}

/// The running player of source `id` (for media controls).
pub fn get(id: &str) -> Option<Arc<Player>> {
    registry().lock().ok()?.get(id)?.upgrade()
}

// ---------------------------------------------------------------- decode thread

/// What ended the current item.
enum Outcome {
    Finished,
    Next,
    Previous,
    Restart,
    Seek(Duration),
    Settings(PlayerSettings),
    Stop,
    Failed(String),
}

struct Clock {
    base: Duration,
    started: Instant,
    paused_at: Option<Instant>,
}

impl Clock {
    fn new(base: Duration, paused: bool) -> Self {
        let now = Instant::now();
        Self {
            base,
            started: now,
            paused_at: paused.then_some(now),
        }
    }

    fn position(&self) -> Duration {
        let now = self.paused_at.unwrap_or_else(Instant::now);
        self.base + now.saturating_duration_since(self.started)
    }

    fn pause(&mut self, paused: bool) {
        match (paused, self.paused_at) {
            (true, None) => self.paused_at = Some(Instant::now()),
            (false, Some(at)) => {
                self.started += at.elapsed();
                self.paused_at = None;
            }
            _ => {}
        }
    }
}

struct Decoder {
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    settings: PlayerSettings,
    order: Vec<usize>,
    clock: Clock,
}

impl Decoder {
    fn new(shared: Arc<Shared>, commands: Receiver<Command>, settings: PlayerSettings) -> Self {
        let mut d = Self {
            shared,
            commands,
            settings,
            order: Vec::new(),
            clock: Clock::new(Duration::ZERO, false),
        };
        d.reorder();
        d
    }

    fn reorder(&mut self) {
        self.order = (0..self.settings.items.len()).collect();
        if self.settings.shuffle {
            // Simple deterministic shuffle seeded from the clock.
            let mut seed = Instant::now().elapsed().as_nanos() as u64 ^ 0x9e37_79b9_7f4a_7c15;
            for i in (1..self.order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                self.order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
        }
    }

    fn status(&self, f: impl FnOnce(&mut PlayerStatus)) {
        if let Ok(mut s) = self.shared.status.lock() {
            f(&mut s);
        }
    }

    fn set_paused(&mut self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
        self.clock.pause(paused);
        self.status(|s| {
            if matches!(s.state, PlayState::Playing | PlayState::Paused) {
                s.state = if paused {
                    PlayState::Paused
                } else {
                    PlayState::Playing
                };
            }
        });
    }

    /// Handles queued commands. Pause is handled here; anything else ends the item.
    fn poll(&mut self) -> Option<Outcome> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Some(Outcome::Stop);
        }
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                Command::Pause(p) => self.set_paused(p),
                Command::Restart => return Some(Outcome::Restart),
                Command::Next => return Some(Outcome::Next),
                Command::Previous => return Some(Outcome::Previous),
                Command::Seek(d) => return Some(Outcome::Seek(d)),
                Command::Settings(s) => return Some(Outcome::Settings(s)),
            }
        }
        None
    }

    /// Waits until the clock reaches `t`, handling commands meanwhile.
    fn wait_until(&mut self, t: Duration) -> Option<Outcome> {
        loop {
            if let Some(o) = self.poll() {
                return Some(o);
            }
            let now = self.clock.position();
            if now >= t && self.clock.paused_at.is_none() {
                return None;
            }
            let step = if self.clock.paused_at.is_some() {
                Duration::from_millis(20)
            } else {
                (t - now).min(Duration::from_millis(10))
            };
            std::thread::sleep(step.max(Duration::from_millis(1)));
            if self.clock.paused_at.is_none() && self.clock.position() >= t {
                return None;
            }
        }
    }

    /// Waits (handling commands) while nothing is playing.
    fn idle(&mut self) -> Outcome {
        loop {
            if let Some(o) = self.poll() {
                return o;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn run(mut self) {
        let mut pos = 0usize;
        let mut start_at = Duration::ZERO;
        let mut failures = 0usize;
        loop {
            if self.order.is_empty() {
                self.status(|s| {
                    *s = PlayerStatus {
                        error: s.error.clone(),
                        ..Default::default()
                    }
                });
                if let Ok(mut p) = self.shared.picture.lock() {
                    *p = None;
                }
                match self.idle() {
                    Outcome::Settings(s) => {
                        self.settings = s;
                        self.reorder();
                        pos = 0;
                        continue;
                    }
                    Outcome::Stop => return,
                    _ => continue,
                }
            }
            pos = pos.min(self.order.len() - 1);
            let path = self.settings.items[self.order[pos]].clone();
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let count = self.order.len();
            self.status(|s| {
                s.item = name.clone();
                s.index = pos;
                s.count = count;
                s.position = start_at;
                s.duration = None;
                s.state = if self.shared.paused.load(Ordering::Acquire) {
                    PlayState::Paused
                } else {
                    PlayState::Playing
                };
            });
            let outcome = self.play(&path, start_at);
            start_at = Duration::ZERO;
            match outcome {
                Outcome::Finished => {
                    failures = 0;
                    pos += 1;
                }
                Outcome::Failed(e) => {
                    let message = format!("{name}: {e}");
                    tracing::warn!(error = %message, "media item failed");
                    self.status(|s| s.error = Some(message));
                    failures += 1;
                    pos += 1;
                    if failures >= count {
                        // Every item failed: wait for new settings instead of spinning.
                        failures = 0;
                        match self.idle() {
                            Outcome::Settings(s) => {
                                self.settings = s;
                                self.reorder();
                                pos = 0;
                            }
                            Outcome::Stop => return,
                            Outcome::Restart => pos = 0,
                            _ => {}
                        }
                        continue;
                    }
                }
                Outcome::Next => pos += 1,
                Outcome::Previous => pos = pos.saturating_sub(1),
                Outcome::Restart => {}
                Outcome::Seek(d) => start_at = d,
                Outcome::Settings(s) => {
                    self.settings = s;
                    self.reorder();
                    pos = 0;
                    self.status(|s| s.error = None);
                    continue;
                }
                Outcome::Stop => return,
            }
            if pos >= self.order.len() {
                if self.settings.looping {
                    pos = 0;
                    if self.settings.shuffle {
                        self.reorder();
                    }
                } else {
                    self.status(|s| s.state = PlayState::Ended);
                    match self.idle() {
                        Outcome::Settings(s) => {
                            self.settings = s;
                            self.reorder();
                            pos = 0;
                        }
                        Outcome::Stop => return,
                        Outcome::Seek(d) => {
                            pos = self.order.len() - 1;
                            start_at = d;
                        }
                        Outcome::Previous => pos = self.order.len().saturating_sub(1),
                        _ => pos = 0,
                    }
                }
            }
        }
    }

    fn publish(&self, picture: Picture) {
        let seq = self.shared.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut p) = self.shared.picture.lock() {
            *p = Some(Picture {
                sequence: seq,
                ..picture
            });
        }
    }

    fn push_audio(&self, samples: &[f32]) {
        if let Ok(mut ring) = self.shared.audio.lock() {
            ring.extend(samples.iter().copied());
            let excess = ring.len().saturating_sub(AUDIO_CAP * 2);
            ring.drain(..excess);
        }
    }

    fn clear_audio(&self) {
        if let Ok(mut ring) = self.shared.audio.lock() {
            ring.clear();
        }
    }

    /// Plays one item from `start_at`.
    fn play(&mut self, path: &Path, start_at: Duration) -> Outcome {
        if !path.exists() {
            return Outcome::Failed("file not found".into());
        }
        let mut input = match format::input(path) {
            Ok(i) => i,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
        let duration =
            (input.duration() > 0).then(|| Duration::from_micros(input.duration() as u64));
        self.status(|s| s.duration = duration);
        let still = has_ext(path, IMAGE_EXTENSIONS);
        let video_index = input.streams().best(media::Type::Video).map(|s| s.index());
        let audio_index = if still {
            None
        } else {
            input.streams().best(media::Type::Audio).map(|s| s.index())
        };
        if video_index.is_none() && audio_index.is_none() {
            return Outcome::Failed("no audio or video in this file".into());
        }
        let open = |index: Option<usize>| -> Option<(ffmpeg::codec::decoder::Opened, ffmpeg::Rational, i64)> {
            let stream = input.stream(index?)?;
            let parameters = stream.parameters();
            // libvpx decodes the alpha channel of VP8/VP9 (WebM stingers); the native
            // decoders drop it.
            let codec = match parameters.id() {
                ffmpeg::codec::Id::VP9 => ffmpeg::decoder::find_by_name("libvpx-vp9"),
                ffmpeg::codec::Id::VP8 => ffmpeg::decoder::find_by_name("libvpx"),
                _ => None,
            }
            .or_else(|| ffmpeg::decoder::find(parameters.id()))?;
            let mut ctx = ffmpeg::codec::context::Context::from_parameters(parameters).ok()?;
            if let Ok(threads) = std::thread::available_parallelism() {
                ctx.set_threading(ffmpeg::codec::threading::Config {
                    kind: ffmpeg::codec::threading::Type::Frame,
                    count: threads.get().min(8),
                });
            }
            Some((
                ctx.decoder().open_as(codec).ok()?,
                stream.time_base(),
                stream.start_time(),
            ))
        };
        let mut video = open(video_index).and_then(|(d, tb, st)| Some((d.video().ok()?, tb, st)));
        let mut audio = open(audio_index).and_then(|(d, tb, st)| Some((d.audio().ok()?, tb, st)));
        if video.is_none() && audio.is_none() {
            return Outcome::Failed("unsupported codec".into());
        }
        if video.is_none()
            && let Ok(mut p) = self.shared.picture.lock()
        {
            *p = None; // audio-only item: show nothing
        }
        if start_at > Duration::ZERO {
            let ts = start_at.as_micros() as i64;
            let _ = input.seek(ts, ..ts);
        }
        self.clear_audio();
        let paused = self.shared.paused.load(Ordering::Acquire);
        self.clock = Clock::new(start_at, paused);

        let seconds = |ts: i64, tb: ffmpeg::Rational, start: i64| -> Duration {
            let start = if start == ffmpeg::ffi::AV_NOPTS_VALUE {
                0
            } else {
                start
            };
            let s = (ts - start) as f64 * tb.0 as f64 / tb.1.max(1) as f64;
            Duration::from_secs_f64(s.max(0.0))
        };

        let mut scaler: Option<(
            software::scaling::Context,
            format::Pixel,
            u32,
            u32,
            PixelLayout,
        )> = None;
        let mut resampler: Option<software::resampling::Context> = None;
        let mut last_video = Duration::ZERO;
        let mut packet = ffmpeg::Packet::empty();
        let mut eof = false;

        // Handles every decoded frame available from the decoders.
        macro_rules! drain_video {
            () => {
                if let Some((dec, tb, st)) = &mut video {
                    let mut decoded = frame::Video::empty();
                    while dec.receive_frame(&mut decoded).is_ok() {
                        let t =
                            seconds(decoded.timestamp().or(decoded.pts()).unwrap_or(0), *tb, *st);
                        if t + Duration::from_millis(40) < start_at {
                            continue; // before a seek target
                        }
                        if let Some(o) = self.wait_until(t) {
                            return o;
                        }
                        match convert_video(&decoded, &mut scaler) {
                            Ok(picture) => self.publish(picture),
                            Err(e) => return Outcome::Failed(e),
                        }
                        last_video = t;
                        self.status(|s| s.position = t);
                        if still {
                            let hold = start_at.max(t)
                                + Duration::from_secs_f32(self.settings.image_seconds);
                            return self.wait_until(hold).unwrap_or(Outcome::Finished);
                        }
                    }
                }
            };
        }
        macro_rules! drain_audio {
            () => {
                if let Some((dec, tb, st)) = &mut audio {
                    let mut decoded = frame::Audio::empty();
                    while dec.receive_frame(&mut decoded).is_ok() {
                        let t =
                            seconds(decoded.timestamp().or(decoded.pts()).unwrap_or(0), *tb, *st);
                        if t + Duration::from_millis(20) < start_at {
                            continue;
                        }
                        if let Some(o) = self.wait_until(t.saturating_sub(AUDIO_LEAD)) {
                            return o;
                        }
                        match convert_audio(&decoded, &mut resampler) {
                            Ok(samples) => self.push_audio(&samples),
                            Err(e) => return Outcome::Failed(e),
                        }
                        if video.is_none() {
                            self.status(|s| s.position = t);
                        }
                    }
                }
            };
        }

        loop {
            if let Some(o) = self.poll() {
                return o;
            }
            if !eof {
                match packet.read(&mut input) {
                    Ok(()) => {
                        let index = packet.stream();
                        if Some(index) == video_index
                            && let Some((dec, ..)) = &mut video
                        {
                            let _ = dec.send_packet(&packet);
                            drain_video!();
                        } else if Some(index) == audio_index
                            && let Some((dec, ..)) = &mut audio
                        {
                            let _ = dec.send_packet(&packet);
                            drain_audio!();
                        }
                    }
                    Err(ffmpeg::Error::Eof) => {
                        eof = true;
                        if let Some((dec, ..)) = &mut video {
                            let _ = dec.send_eof();
                        }
                        if let Some((dec, ..)) = &mut audio {
                            let _ = dec.send_eof();
                        }
                        drain_video!();
                        drain_audio!();
                    }
                    Err(e) => return Outcome::Failed(e.to_string()),
                }
                continue;
            }
            // End of file: let the last frame stay up for one frame time and the audio drain.
            let end = duration.unwrap_or(last_video + Duration::from_millis(40));
            if let Some(o) = self.wait_until(end.max(last_video)) {
                return o;
            }
            while self.shared.audio.lock().map_or(0, |r| r.len()) > 0 {
                if let Some(o) = self.poll() {
                    return o;
                }
                std::thread::sleep(Duration::from_millis(10));
                if self.clock.position() > end + AUDIO_LEAD * 3 {
                    break;
                }
            }
            return Outcome::Finished;
        }
    }
}

fn has_alpha(format: format::Pixel) -> bool {
    format
        .descriptor()
        .is_some_and(|d| d.nb_components() == 4 || d.nb_components() == 2)
}

/// Converts a decoded frame to tightly packed NV12 (or RGBA if it has alpha).
pub(crate) fn convert_video(
    decoded: &frame::Video,
    scaler: &mut Option<(
        software::scaling::Context,
        format::Pixel,
        u32,
        u32,
        PixelLayout,
    )>,
) -> Result<Picture, String> {
    let (fmt, w, h) = (decoded.format(), decoded.width(), decoded.height());
    if w < 2 || h < 2 {
        return Err("video too small".into());
    }
    let layout = if has_alpha(fmt) {
        PixelLayout::Rgba
    } else {
        PixelLayout::Nv12
    };
    // NV12 needs even sizes.
    let (ow, oh) = match layout {
        PixelLayout::Nv12 => (w & !1, h & !1),
        PixelLayout::Rgba => (w, h),
    };
    let out_fmt = match layout {
        PixelLayout::Nv12 => format::Pixel::NV12,
        PixelLayout::Rgba => format::Pixel::RGBA,
    };
    if scaler
        .as_ref()
        .is_none_or(|(_, f, sw, sh, l)| *f != fmt || *sw != w || *sh != h || *l != layout)
    {
        let ctx = software::scaling::Context::get(
            fmt,
            w,
            h,
            out_fmt,
            ow,
            oh,
            software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| e.to_string())?;
        *scaler = Some((ctx, fmt, w, h, layout));
    }
    let Some((ctx, ..)) = scaler else {
        return Err("no scaler".into());
    };
    let mut out = frame::Video::empty();
    ctx.run(decoded, &mut out).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    let copy = |data: &mut Vec<u8>, plane: usize, row: usize, rows: usize| {
        let stride = out.stride(plane);
        let src = out.data(plane);
        for r in 0..rows {
            data.extend_from_slice(&src[r * stride..r * stride + row]);
        }
    };
    match layout {
        PixelLayout::Nv12 => {
            data.reserve((ow * oh * 3 / 2) as usize);
            copy(&mut data, 0, ow as usize, oh as usize);
            copy(&mut data, 1, ow as usize, oh as usize / 2);
        }
        PixelLayout::Rgba => {
            data.reserve((ow * oh * 4) as usize);
            copy(&mut data, 0, ow as usize * 4, oh as usize);
        }
    }
    Ok(Picture {
        width: ow,
        height: oh,
        layout,
        data,
        sequence: 0,
        full_range: matches!(decoded.color_range(), ffmpeg::util::color::Range::JPEG),
    })
}

/// Resamples a decoded audio frame to interleaved f32 stereo at 48 kHz.
pub(crate) fn convert_audio(
    decoded: &frame::Audio,
    resampler: &mut Option<software::resampling::Context>,
) -> Result<Vec<f32>, String> {
    if resampler.is_none() {
        let layout = if decoded.channel_layout().channels() > 0 {
            decoded.channel_layout()
        } else {
            ffmpeg::ChannelLayout::default(decoded.channels() as i32)
        };
        *resampler = Some(
            software::resampling::Context::get(
                decoded.format(),
                layout,
                decoded.rate(),
                format::Sample::F32(format::sample::Type::Packed),
                ffmpeg::ChannelLayout::STEREO,
                SAMPLE_RATE,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let Some(ctx) = resampler else {
        return Err("no resampler".into());
    };
    let capacity = decoded.samples() * SAMPLE_RATE as usize / decoded.rate().max(1) as usize + 64;
    let mut out = frame::Audio::new(
        format::Sample::F32(format::sample::Type::Packed),
        capacity,
        ffmpeg::ChannelLayout::STEREO,
    );
    ctx.run(decoded, &mut out).map_err(|e| e.to_string())?;
    let n = out.samples() * 2;
    let bytes = &out.data(0)[..n * 4];
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}
