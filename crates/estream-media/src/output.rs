//! Recording and replay-buffer outputs. Each runs one encoder thread that owns
//! the video encoder, one audio encoder per track, and a muxer or packet ring.

use crate::MediaError;
use crate::encode::{self, AudioEncoder, VideoEncoder};
use crate::ffmpeg;
use crate::mux::{Muxer, PacketRing, StreamSpec, remux};
use crate::settings::{Container, RecordingSettings, StreamSettings, redact};
use crate::stream::{Entry, NetTarget, NetworkStats, StreamState};
use crossbeam_channel::{Receiver, Sender, bounded, select};
use estream_audio::{Block, TRACKS, TrackSink};
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
    pub encoder: Mutex<String>,
    /// Set if the output failed while running.
    pub error: Mutex<Option<String>>,
    /// Streaming outputs only.
    pub network: NetworkStats,
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
    Network(NetTarget),
}

/// What a session produces.
enum Kind {
    File(PathBuf),
    Replay,
    Network(Box<StreamSettings>),
}

enum Control {
    SaveReplay(Sender<Result<PathBuf, MediaError>>),
}

struct Session {
    stop: Arc<AtomicBool>,
    sink_closed: Arc<AtomicBool>,
    consumer: u64,
    control: Sender<Control>,
    thread: Option<JoinHandle<Result<(), MediaError>>>,
    pub stats: Arc<OutputStats>,
    started: Instant,
    canvas: CanvasId,
    dropped_base: u64,
}

impl Session {
    fn start(
        engine: &Engine,
        settings: &RecordingSettings,
        kind: Kind,
    ) -> Result<Self, MediaError> {
        crate::init();
        let snapshot = engine.snapshot();
        let canvas = snapshot
            .canvases
            .first()
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

    fn update_dropped(&self, engine: &Engine) {
        if let Some(out) = engine.canvas_output(self.canvas) {
            let total = out.stats.encoder_dropped.load(Ordering::Relaxed);
            self.stats
                .dropped
                .store(total.saturating_sub(self.dropped_base), Ordering::Relaxed);
        }
    }

    /// Disconnects from the engine and asks the encoder thread to finish.
    fn detach(&mut self, engine: &Engine) {
        self.update_dropped(engine);
        engine.remove_video_consumer(self.consumer);
        self.sink_closed.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    fn stop(mut self, engine: &Engine) -> Result<(), MediaError> {
        self.detach(engine);
        self.join()
    }

    fn join(mut self) -> Result<(), MediaError> {
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(result)) => result,
            _ => Err(MediaError::Engine("encoder thread panicked".into())),
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
    let target = match r.kind {
        Kind::Replay => Target::Replay(PacketRing::new(
            specs,
            r.settings.replay_seconds.clamp(5, 300) as f64,
        )),
        Kind::File(path) => Target::File(Muxer::create(&path, r.settings.container, &specs)?),
        Kind::Network(stream) => Target::Network(NetTarget::start(
            *stream,
            specs,
            r.stop.clone(),
            r.stats.clone(),
        )?),
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
            recv(r.control_rx) -> c => if let Ok(Control::SaveReplay(reply)) = c {
                let result = match &state.target {
                    Target::Replay(ring) => {
                        let container = replay_container(&r.settings);
                        let path = r.settings.next_path(" Replay", container);
                        ring.save(&path, container).map(|_| path)
                    }
                    _ => Err(MediaError::Io("not a replay buffer".into())),
                };
                let _ = reply.send(result);
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
        Target::Network(net) => net.finish(),
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
            Target::Network(net) => {
                let tb = self
                    .time_bases
                    .get(stream)
                    .copied()
                    .unwrap_or(ffmpeg::Rational(1, 1));
                let time = packet.pts().or(packet.dts()).unwrap_or(0) as f64 * tb.0 as f64
                    / tb.1.max(1) as f64;
                net.queue.push(Entry {
                    stream,
                    video: stream == 0,
                    key: packet.is_key(),
                    time,
                    payload: packet,
                });
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
            Target::Network(net) => net.force_key.swap(false, Ordering::AcqRel),
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
        if !self.audio_started {
            let start = ((chunk.timestamp.0 - self.t0.0) as f64 * 48_000.0 / 1e9).round() as i64;
            encoders.iter_mut().for_each(|a| a.set_start(start));
            self.audio_started = true;
        }
        for (k, enc) in encoders.iter_mut().enumerate() {
            let track = self.tracks[k];
            enc.push(&chunk.tracks[track], &mut |p| self.write(k + 1, p))?;
        }
        Ok(())
    }
}
/// Replays are saved as MP4 for easy sharing, unless MOV was chosen.
fn replay_container(settings: &RecordingSettings) -> Container {
    match settings.container {
        Container::Mov => Container::Mov,
        _ => Container::Mp4,
    }
}

/// A running recording.
pub struct Recording {
    session: Session,
    pub path: PathBuf,
    settings: RecordingSettings,
}

impl Recording {
    pub fn start(engine: &Engine, settings: &RecordingSettings) -> Result<Self, MediaError> {
        let path = settings.next_path("", settings.container);
        let session = Session::start(engine, settings, Kind::File(path.clone()))?;
        tracing::info!(path = %path.display(), "recording started");
        Ok(Self {
            session,
            path,
            settings: settings.clone(),
        })
    }

    pub fn stats(&self) -> &Arc<OutputStats> {
        &self.session.stats
    }

    pub fn elapsed(&self) -> Duration {
        self.session.started.elapsed()
    }

    /// Updates the dropped-frame counter from the engine.
    pub fn poll(&self, engine: &Engine) {
        self.session.update_dropped(engine);
    }

    /// Stops and finalizes the file. Returns the final path (the MP4 if remuxed).
    pub fn stop(self, engine: &Engine) -> Result<PathBuf, MediaError> {
        self.session.stop(engine)?;
        tracing::info!(path = %self.path.display(), "recording stopped");
        if self.settings.auto_remux && self.settings.container == Container::Mkv {
            let mp4 = self.path.with_extension("mp4");
            match remux(&self.path, &mp4, Container::Mp4) {
                Ok(()) => return Ok(mp4),
                Err(e) => tracing::warn!(error = %e, "remux to MP4 failed; keeping MKV"),
            }
        }
        Ok(self.path)
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

    pub fn stop(self, engine: &Engine) -> Result<(), MediaError> {
        self.session.stop(engine)
    }
}

/// A running live stream.
pub struct Stream {
    session: Session,
    /// Destination with secrets removed, for display.
    pub destination: String,
}

impl Stream {
    /// Validates the settings, picks the encoder and starts connecting in the background.
    pub fn start(engine: &Engine, settings: &StreamSettings) -> Result<Self, MediaError> {
        let protocol = settings.validate().map_err(MediaError::Engine)?;
        let destination = redact(&settings.url());
        let session = Session::start(
            engine,
            &settings.encoder_settings(),
            Kind::Network(Box::new(settings.clone())),
        )?;
        tracing::info!(%destination, protocol = protocol.label(), "stream started");
        Ok(Self {
            session,
            destination,
        })
    }

    pub fn stats(&self) -> &Arc<OutputStats> {
        &self.session.stats
    }

    pub fn state(&self) -> StreamState {
        self.session.stats.network.state()
    }

    pub fn elapsed(&self) -> Duration {
        self.session.started.elapsed()
    }

    /// Updates the encoder-lag counter from the engine.
    pub fn poll(&self, engine: &Engine) {
        self.session.update_dropped(engine);
    }

    pub fn stop(self, engine: &Engine) -> Result<(), MediaError> {
        self.session.stop(engine)
    }

    /// Stops without blocking the caller: the encoder and connection wind down on a
    /// helper thread (flushing can take up to about a second).
    pub fn stop_in_background(self, engine: &Engine) {
        let mut session = self.session;
        session.detach(engine);
        let spawned = std::thread::Builder::new()
            .name("estream-stream-stop".into())
            .spawn(move || {
                if let Err(e) = session.join() {
                    tracing::warn!(error = %e, "stream stopped with an error");
                }
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "couldn't spawn the stream stop thread");
        }
    }
}
