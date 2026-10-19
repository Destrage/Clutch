//! Recording and replay-buffer outputs. Each runs one encoder thread that owns
//! the video encoder, one audio encoder per track, and a muxer or packet ring.

use crate::MediaError;
use crate::encode::{self, AudioEncoder, VideoEncoder};
use crate::ffmpeg;
use crate::mux::{Muxer, PacketRing, StreamSpec, remux};
use crate::settings::{Container, RecordingSettings};
use crate::stream::{Entry, NetSpec, NetTarget, share};
use crossbeam_channel::{Receiver, Sender, bounded, select};
use estream_audio::{BLOCK, Block, SAMPLE_RATE, TRACKS, TrackSink};
use estream_core::{CanvasId, Rational, Timestamp};
use estream_engine::{Engine, VideoFrame};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Live statistics of an output.
#[derive(Debug, Default)]
pub struct OutputStats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    /// Frames dropped before reaching the encoder.
    pub dropped: AtomicU64,
    /// Milliseconds of audio that never arrived and were filled with silence.
    pub audio_gap_ms: AtomicU64,
    pub encoder: Mutex<String>,
    /// Set if the output failed while running.
    pub error: Mutex<Option<String>>,
}

struct AudioChunk {
    timestamp: Timestamp,
    tracks: [Block; TRACKS],
}

/// Forwards mixer blocks to an output's thread.
struct ChannelSink {
    tx: Sender<AudioChunk>,
    closed: Arc<AtomicBool>,
}

impl TrackSink for ChannelSink {
    fn push(&mut self, timestamp: Timestamp, tracks: &[Block; TRACKS]) {
        let _ = self.tx.try_send(AudioChunk {
            timestamp,
            tracks: tracks.clone(),
        });
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

enum Target {
    File(Muxer),
    Replay(PacketRing),
    Network(Vec<NetTarget>),
}

/// What a session produces.
pub(crate) enum Kind {
    File(PathBuf),
    Replay,
    Network(Vec<NetSpec>),
}

/// A clip written from the replay buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedClip {
    pub path: PathBuf,
    /// Engine-clock time of the clip's first frame (a keyframe at or before the request).
    pub start: Timestamp,
    /// Engine-clock time of the clip's last frame.
    pub end: Timestamp,
}

enum Control {
    SaveReplay(Sender<Result<PathBuf, MediaError>>),
    /// Save the engine-clock window `from..=to` to `path`.
    SaveRange {
        from: Timestamp,
        to: Timestamp,
        path: PathBuf,
        container: Container,
        reply: Sender<Result<SavedClip, MediaError>>,
    },
    /// A stream destination joining a running stream session.
    AddDestination(NetSpec),
}

pub(crate) struct Session {
    stop: Arc<AtomicBool>,
    sink_closed: Arc<AtomicBool>,
    consumer: u64,
    control: Sender<Control>,
    thread: Option<JoinHandle<Result<(), MediaError>>>,
    pub stats: Arc<OutputStats>,
    pub(crate) started: Instant,
    canvas: CanvasId,
    dropped_base: u64,
}

impl Session {
    pub(crate) fn start(
        engine: &Engine,
        settings: &RecordingSettings,
        kind: Kind,
    ) -> Result<Self, MediaError> {
        crate::init();
        let snapshot = engine.snapshot();
        let canvas = resolve_canvas(engine, settings.canvas)
            .and_then(|id| snapshot.canvas(id))
            .ok_or_else(|| MediaError::Engine("there is no canvas to record".into()))?;
        let fps = canvas.fps;
        // Pick the encoder up front so problems are reported immediately.
        let encoder = encode::choose(settings)?;
        let stats = Arc::new(OutputStats::default());
        if let Ok(mut e) = stats.encoder.lock() {
            *e = encoder.label.to_owned();
        }

        let (video_tx, video_rx) = bounded::<VideoFrame>(8);
        let (audio_tx, audio_rx) = bounded::<AudioChunk>(64);
        let (control_tx, control_rx) = bounded::<Control>(4);
        let stop = Arc::new(AtomicBool::new(false));
        let sink_closed = Arc::new(AtomicBool::new(false));
        let consumer = engine.add_video_consumer(canvas.id, video_tx);
        engine.add_track_sink(Box::new(ChannelSink {
            tx: audio_tx,
            closed: sink_closed.clone(),
        }));
        let dropped_base = engine
            .canvas_output(canvas.id)
            .map_or(0, |o| o.stats.encoder_dropped.load(Ordering::Relaxed));

        let thread = {
            let (settings, stats, stop) = (settings.clone(), stats.clone(), stop.clone());
            let encoder_name = encoder.name;
            std::thread::Builder::new()
                .name("estream-encoder".into())
                .spawn(move || {
                    let result = run(Run {
                        settings,
                        encoder_name,
                        fps,
                        video_rx,
                        audio_rx,
                        control_rx,
                        stop,
                        stats: stats.clone(),
                        kind,
                    });
                    if let Err(e) = &result {
                        tracing::error!(error = %e, "output failed");
                        if let Ok(mut slot) = stats.error.lock() {
                            *slot = Some(e.to_string());
                        }
                    }
                    result
                })
                .map_err(|e| MediaError::Io(e.to_string()))?
        };
        Ok(Self {
            stop,
            sink_closed,
            consumer,
            control: control_tx,
            thread: Some(thread),
            stats,
            started: Instant::now(),
            canvas: canvas.id,
            dropped_base,
        })
    }

    pub(crate) fn update_dropped(&self, engine: &Engine) {
        if let Some(out) = engine.canvas_output(self.canvas) {
            let total = out.stats.encoder_dropped.load(Ordering::Relaxed);
            self.stats
                .dropped
                .store(total.saturating_sub(self.dropped_base), Ordering::Relaxed);
        }
    }

    /// Disconnects from the engine and asks the encoder thread to finish.
    pub(crate) fn detach(&mut self, engine: &Engine) {
        self.update_dropped(engine);
        engine.remove_video_consumer(self.consumer);
        self.sink_closed.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    pub(crate) fn stop(mut self, engine: &Engine) -> Result<(), MediaError> {
        self.detach(engine);
        self.join()
    }

    pub(crate) fn join(mut self) -> Result<(), MediaError> {
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(result)) => result,
            _ => Err(MediaError::Engine("encoder thread panicked".into())),
        }
    }

    /// Adds a destination to a running stream session.
    pub(crate) fn add_destination(&self, spec: NetSpec) -> Result<(), MediaError> {
        self.control
            .send(Control::AddDestination(spec))
            .map_err(|_| MediaError::Engine("the stream encoder has stopped".into()))
    }

    /// Stops on a helper thread so the caller doesn't wait for flushing.
    pub(crate) fn stop_in_background(mut self, engine: &Engine) {
        self.detach(engine);
        let spawned = std::thread::Builder::new()
            .name("estream-output-stop".into())
            .spawn(move || {
                if let Err(e) = self.join() {
                    tracing::warn!(error = %e, "output stopped with an error");
                }
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "couldn't spawn the output stop thread");
        }
    }
}

struct Run {
    settings: RecordingSettings,
    encoder_name: &'static str,
    fps: Rational,
    video_rx: Receiver<VideoFrame>,
    audio_rx: Receiver<AudioChunk>,
    control_rx: Receiver<Control>,
    stop: Arc<AtomicBool>,
    stats: Arc<OutputStats>,
    kind: Kind,
}

fn run(r: Run) -> Result<(), MediaError> {
    // Wait for the first frame: it fixes the size and time zero.
    let first = loop {
        if r.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Ok(f) = r.video_rx.recv_timeout(Duration::from_millis(100)) {
            break f;
        }
    };
    let t0 = first.timestamp;
    let mut video = VideoEncoder::open(
        r.encoder_name,
        first.width,
        first.height,
        r.fps,
        &r.settings,
    )?;
    let tracks: Vec<usize> = (0..TRACKS)
        .filter(|t| r.settings.tracks & (1 << t) != 0)
        .collect();
    let mut audio = tracks
        .iter()
        .map(|_| AudioEncoder::open(r.settings.audio_codec, r.settings.audio_bitrate_kbps))
        .collect::<Result<Vec<_>, _>>()?;

    let mut specs = vec![StreamSpec {
        parameters: video.parameters(),
        time_base: video.time_base,
        is_video: true,
    }];
    specs.extend(audio.iter().map(|a| StreamSpec {
        parameters: a.parameters(),
        time_base: a.time_base,
        is_video: false,
    }));
    let time_bases = specs.iter().map(|s| s.time_base).collect();
    // Kept for destinations that join a running stream session.
    let stream_specs = specs.clone();
    let target = match r.kind {
        Kind::Replay => Target::Replay(PacketRing::new(
            specs,
            r.settings.replay_seconds.clamp(5, MAX_REPLAY_SECONDS) as f64,
        )),
        Kind::File(path) => Target::File(Muxer::create(&path, r.settings.container, &specs)?),
        Kind::Network(destinations) => Target::Network(
            destinations
                .into_iter()
                .map(|d| NetTarget::start(d, specs.clone(), r.stop.clone()))
                .collect::<Result<_, _>>()?,
        ),
    };

    let mut state = State {
        target,
        stats: r.stats.clone(),
        write_error: None,
        t0,
        ns_per_frame: 1e9 * r.fps.den as f64 / r.fps.num as f64,
        last_pts: -1,
        audio_started: false,
        tracks,
        time_bases,
    };
    state.video(first, &mut video)?;

    while !r.stop.load(Ordering::Acquire) {
        select! {
            recv(r.video_rx) -> f => if let Ok(f) = f { state.video(f, &mut video)?; },
            recv(r.audio_rx) -> a => if let Ok(a) = a { state.audio(a, &mut audio)?; },
            recv(r.control_rx) -> c => match c {
                Ok(Control::SaveReplay(reply)) => {
                    let result = match &state.target {
                        Target::Replay(ring) => {
                            let container = replay_container(&r.settings);
                            let path = r.settings.next_path(" Replay", container);
                            ring.save(&path, container).map(|_| path)
                        }
                        _ => Err(MediaError::Io("not a replay buffer".into())),
                    };
                    let _ = reply.send(result);
                }
                Ok(Control::SaveRange { from, to, path, container, reply }) => {
                    let result = match &state.target {
                        Target::Replay(ring) => {
                            // The ring's clock is seconds since this output's first frame.
                            let secs = |t: Timestamp| (t.0 as f64 - t0.0 as f64) / 1e9;
                            let at = |s: f64| Timestamp((t0.0 as f64 + s * 1e9).max(0.0) as u64);
                            ring.save_range(secs(from), secs(to), &path, container)
                                .map(|(start, end)| SavedClip { path, start: at(start), end: at(end) })
                        }
                        _ => Err(MediaError::Io("not a replay buffer".into())),
                    };
                    let _ = reply.send(result);
                }
                Ok(Control::AddDestination(spec)) => {
                    let stats = spec.stats.clone();
                    match &mut state.target {
                        Target::Network(nets) => {
                            match NetTarget::start(spec, stream_specs.clone(), r.stop.clone()) {
                                Ok(net) => nets.push(net),
                                Err(e) => stats.set_error(e.to_string()),
                            }
                        }
                        _ => stats.set_error("not a stream session".into()),
                    }
                }
                Err(_) => {}
            },
            default(Duration::from_millis(50)) => {},
        }
    }
    // Drain what's already queued so the end of the recording isn't cut.
    while let Ok(f) = r.video_rx.try_recv() {
        state.video(f, &mut video)?;
    }
    while let Ok(a) = r.audio_rx.try_recv() {
        state.audio(a, &mut audio)?;
    }
    video.finish(&mut |p| state.write(0, p))?;
    for (k, enc) in audio.iter_mut().enumerate() {
        enc.finish(&mut |p| state.write(k + 1, p))?;
    }
    if let Some(e) = state.write_error.take() {
        return Err(e);
    }
    match state.target {
        Target::File(m) => m.finish()?,
        Target::Network(nets) => {
            // Close every queue first so all senders flush in parallel, then join.
            nets.iter().for_each(|n| n.queue.close());
            nets.into_iter().for_each(NetTarget::finish);
        }
        Target::Replay(_) => {}
    }
    Ok(())
}

/// Encoder-thread state: timestamps and where packets go.
struct State {
    target: Target,
    stats: Arc<OutputStats>,
    write_error: Option<MediaError>,
    t0: Timestamp,
    ns_per_frame: f64,
    last_pts: i64,
    audio_started: bool,
    tracks: Vec<usize>,
    time_bases: Vec<ffmpeg::Rational>,
}

impl State {
    fn write(&mut self, stream: usize, packet: ffmpeg::Packet) {
        match &mut self.target {
            Target::File(m) => {
                if let Err(e) = m.write(stream, packet) {
                    self.write_error.get_or_insert(e);
                }
                self.stats.bytes.store(m.bytes, Ordering::Relaxed);
            }
            Target::Replay(ring) => {
                ring.push(stream, packet);
                self.stats.bytes.store(ring.bytes, Ordering::Relaxed);
            }
            Target::Network(nets) => {
                let tb = self
                    .time_bases
                    .get(stream)
                    .copied()
                    .unwrap_or(ffmpeg::Rational(1, 1));
                let time = packet.pts().or(packet.dts()).unwrap_or(0) as f64 * tb.0 as f64
                    / tb.1.max(1) as f64;
                let key = packet.is_key();
                // Every destination gets a reference to the same packet data.
                for net in nets.iter() {
                    net.queue.push(Entry {
                        stream,
                        video: stream == 0,
                        key,
                        time,
                        payload: share(&packet),
                    });
                }
            }
        }
    }

    fn video(&mut self, frame: VideoFrame, encoder: &mut VideoEncoder) -> Result<(), MediaError> {
        let pts =
            (frame.timestamp.0.saturating_sub(self.t0.0) as f64 / self.ns_per_frame).round() as i64;
        if pts <= self.last_pts {
            return Ok(()); // keep frame timestamps strictly increasing
        }
        self.last_pts = pts;
        let force_key = match &self.target {
            // Any (re)connecting destination asks for a keyframe; clear every request.
            // (Not `any()`: it would stop early and leave other requests pending.)
            Target::Network(nets) => {
                let mut requested = false;
                for n in nets.iter() {
                    requested |= n.force_key.swap(false, Ordering::AcqRel);
                }
                requested
            }
            _ => false,
        };
        encoder.encode(&frame, pts, force_key, &mut |p| self.write(0, p))?;
        self.stats.frames.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn audio(
        &mut self,
        chunk: AudioChunk,
        encoders: &mut [AudioEncoder],
    ) -> Result<(), MediaError> {
        if chunk.timestamp < self.t0 {
            return Ok(());
        }
        let at = ((chunk.timestamp.0 - self.t0.0) as f64 * SAMPLE_RATE as f64 / 1e9).round() as i64;
        if !self.audio_started {
            encoders.iter_mut().for_each(|a| a.set_start(at));
            self.audio_started = true;
        }
        // Audio positions count samples, so audio that never arrived (a stalled mixer, a full
        // queue) would pull everything after it early and the stream drifts out of sync.
        // Keep audio on the clock: gaps become silence, stale blocks are dropped.
        let pad = match encoders.first().map(|a| realign(at, a.next_sample())) {
            Some(Realign::Drop) => return Ok(()),
            Some(Realign::Pad(n)) => {
                let ms = n as u64 * 1000 / SAMPLE_RATE as u64;
                self.stats.audio_gap_ms.fetch_add(ms, Ordering::Relaxed);
                n
            }
            _ => 0,
        };
        for (k, enc) in encoders.iter_mut().enumerate() {
            if pad > 0 {
                enc.pad_silence(pad, &mut |p| self.write(k + 1, p))?;
            }
            let track = self.tracks[k];
            enc.push(&chunk.tracks[track], &mut |p| self.write(k + 1, p))?;
        }
        Ok(())
    }
}

/// How far (in samples, 20 ms) audio may stray from the clock before it is realigned.
const RESYNC_SAMPLES: i64 = 2 * BLOCK as i64;

/// What to do with an audio block due at sample `at` when the encoder is at sample `next`.
#[derive(Debug, PartialEq)]
enum Realign {
    Keep,
    /// Audio went missing: fill this many samples of silence first.
    Pad(usize),
    /// The block is for time already covered.
    Drop,
}

fn realign(at: i64, next: i64) -> Realign {
    match at - next {
        d if d > RESYNC_SAMPLES => Realign::Pad(d as usize),
        d if d < -RESYNC_SAMPLES => Realign::Drop,
        _ => Realign::Keep,
    }
}

/// The longest replay buffer, in seconds (clip mode needs room for post-roll).
pub const MAX_REPLAY_SECONDS: u32 = 600;

/// Replays are saved as MP4 for easy sharing, unless MOV was chosen.
fn replay_container(settings: &RecordingSettings) -> Container {
    match settings.container {
        Container::Mov => Container::Mov,
        _ => Container::Mp4,
    }
}

/// The canvas a recording setting refers to, falling back to the first canvas when the
/// stored id isn't in this collection.
pub(crate) fn resolve_canvas(engine: &Engine, wanted: Option<CanvasId>) -> Option<CanvasId> {
    let snapshot = engine.snapshot();
    wanted
        .filter(|id| snapshot.canvas(*id).is_some())
        .or_else(|| snapshot.canvases.first().map(|c| c.id))
}

/// A running recording (one file per recorded canvas).
pub struct Recording {
    /// Main canvas first.
    sessions: Vec<(Session, PathBuf)>,
    /// The main file.
    pub path: PathBuf,
    settings: RecordingSettings,
}

impl Recording {
    /// Every file being written (one per recorded canvas, main first).
    pub fn paths(&self) -> impl Iterator<Item = &std::path::Path> {
        self.sessions.iter().map(|(_, p)| p.as_path())
    }

    pub fn start(engine: &Engine, settings: &RecordingSettings) -> Result<Self, MediaError> {
        let snapshot = engine.snapshot();
        let main = resolve_canvas(engine, settings.canvas)
            .ok_or_else(|| MediaError::Engine("there is no canvas to record".into()))?;
        let mut canvases = vec![main];
        if settings.all_canvases {
            canvases.extend(
                snapshot
                    .canvases
                    .iter()
                    .map(|c| c.id)
                    .filter(|c| *c != main),
            );
        }
        let mut sessions = Vec::new();
        for (i, canvas) in canvases.into_iter().enumerate() {
            // Extra canvases get their name in the file name.
            let suffix = match (i, snapshot.canvas(canvas)) {
                (0, _) | (_, None) => String::new(),
                (_, Some(c)) => format!(" ({})", c.name),
            };
            let path = settings.next_path(&suffix, settings.container);
            let per_canvas = RecordingSettings {
                canvas: Some(canvas),
                ..settings.clone()
            };
            match Session::start(engine, &per_canvas, Kind::File(path.clone())) {
                Ok(session) => {
                    tracing::info!(path = %path.display(), "recording started");
                    sessions.push((session, path));
                }
                Err(e) => {
                    // Don't leave half a recording running.
                    for (s, _) in sessions {
                        let _ = s.stop(engine);
                    }
                    return Err(e);
                }
            }
        }
        let path = sessions[0].1.clone();
        Ok(Self {
            sessions,
            path,
            settings: settings.clone(),
        })
    }

    /// Statistics of the main canvas's recording.
    pub fn stats(&self) -> &Arc<OutputStats> {
        &self.sessions[0].0.stats
    }

    /// Number of files being written.
    pub fn file_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn elapsed(&self) -> Duration {
        self.sessions[0].0.started.elapsed()
    }

    /// Updates the dropped-frame counters from the engine.
    pub fn poll(&self, engine: &Engine) {
        for (s, _) in &self.sessions {
            s.update_dropped(engine);
        }
    }

    /// Stops and finalizes the main file. Returns its final path (the MP4 if remuxed).
    pub fn stop(self, engine: &Engine) -> Result<PathBuf, MediaError> {
        self.stop_all(engine)?
            .into_iter()
            .next()
            .ok_or_else(|| MediaError::Engine("nothing was recorded".into()))
    }

    /// Stops every canvas's recording. Returns the final paths, main canvas first.
    pub fn stop_all(self, engine: &Engine) -> Result<Vec<PathBuf>, MediaError> {
        let mut result = Ok(());
        let mut paths = Vec::new();
        for (session, path) in self.sessions {
            if let Err(e) = session.stop(engine) {
                result = Err(e);
            }
            tracing::info!(path = %path.display(), "recording stopped");
            let mut final_path = path.clone();
            if self.settings.auto_remux && self.settings.container == Container::Mkv {
                let mp4 = path.with_extension("mp4");
                match remux(&path, &mp4, Container::Mp4) {
                    Ok(()) => final_path = mp4,
                    Err(e) => tracing::warn!(error = %e, "remux to MP4 failed; keeping MKV"),
                }
            }
            paths.push(final_path);
        }
        result.map(|()| paths)
    }
}

/// A running replay buffer.
pub struct ReplayBuffer {
    session: Session,
}

impl ReplayBuffer {
    pub fn start(engine: &Engine, settings: &RecordingSettings) -> Result<Self, MediaError> {
        let session = Session::start(engine, settings, Kind::Replay)?;
        Ok(Self { session })
    }

    pub fn stats(&self) -> &Arc<OutputStats> {
        &self.session.stats
    }

    /// Saves the buffered seconds to a new file; the buffer keeps running.
    pub fn save(&self) -> Result<PathBuf, MediaError> {
        let (tx, rx) = bounded(1);
        self.session
            .control
            .send(Control::SaveReplay(tx))
            .map_err(|_| MediaError::Engine("replay buffer stopped".into()))?;
        rx.recv_timeout(Duration::from_secs(20))
            .map_err(|_| MediaError::Engine("saving the replay timed out".into()))?
    }

    /// Asks the buffer to save the engine-clock window `from..=to` to `path`. The clip starts
    /// at the last keyframe at or before `from`. The result arrives on the returned channel
    /// once the file is written, so the caller never waits on the disk.
    pub fn save_range(
        &self,
        from: Timestamp,
        to: Timestamp,
        path: PathBuf,
        container: Container,
    ) -> Receiver<Result<SavedClip, MediaError>> {
        let (reply, rx) = bounded(1);
        let request = Control::SaveRange {
            from,
            to,
            path,
            container,
            reply: reply.clone(),
        };
        if self.session.control.send(request).is_err() {
            let _ = reply.send(Err(MediaError::Engine("replay buffer stopped".into())));
        }
        rx
    }

    pub fn stop(self, engine: &Engine) -> Result<(), MediaError> {
        self.session.stop(engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_on_time_is_kept() {
        assert_eq!(realign(48_000, 48_000), Realign::Keep);
        // Rounding and one block of jitter stay within the window.
        assert_eq!(realign(48_000 + BLOCK as i64, 48_000), Realign::Keep);
        assert_eq!(realign(48_000 - BLOCK as i64, 48_000), Realign::Keep);
    }

    #[test]
    fn missing_audio_is_filled_with_silence() {
        // A 300 ms stall: the block is due 14 400 samples after where the encoder is.
        assert_eq!(realign(62_400, 48_000), Realign::Pad(14_400));
    }

    #[test]
    fn stale_audio_is_dropped() {
        assert_eq!(realign(48_000, 48_000 + 3 * BLOCK as i64), Realign::Drop);
    }
}
