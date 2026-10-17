//! Live streaming transport: a send queue that absorbs network stalls (dropping video at
//! keyframe boundaries) and a sender thread that owns the connection and reconnects.

use crate::MediaError;
use crate::ffmpeg::{self, Dictionary};
use crate::mux::StreamSpec;
use crate::output::OutputStats;
use crate::settings::{Protocol, StreamSettings, redact};
use std::collections::VecDeque;
use std::ffi::{CString, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// After Stop, in-flight network calls get this long to flush before they are aborted.
const STOP_GRACE: Duration = Duration::from_millis(1000);
/// Socket read/write timeout for dead peers (microseconds).
const RW_TIMEOUT_US: &str = "10000000";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamState {
    Connecting,
    Live,
    /// Waiting to retry; `attempt` in [`OutputStats::attempt`].
    Reconnecting,
    Stopped,
}

impl StreamState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => StreamState::Live,
            2 => StreamState::Reconnecting,
            3 => StreamState::Stopped,
            _ => StreamState::Connecting,
        }
    }
}

/// Network side of [`OutputStats`].
#[derive(Debug, Default)]
pub struct NetworkStats {
    /// Video frames dropped because the network could not keep up.
    pub dropped: AtomicU64,
    /// Current send-queue delay in milliseconds.
    pub queue_ms: AtomicU64,
    state: AtomicU8,
    /// Reconnect attempt in progress (0 when connected).
    pub attempt: AtomicU32,
    pub reconnects: AtomicU32,
}

impl NetworkStats {
    pub fn state(&self) -> StreamState {
        StreamState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, state: StreamState) {
        self.state.store(state as u8, Ordering::Release);
    }
}

/// One queued packet.
pub struct Entry<T> {
    pub stream: usize,
    pub payload: T,
    pub video: bool,
    pub key: bool,
    /// Presentation time in seconds (encoder clock).
    pub time: f64,
}

struct QueueState<T> {
    entries: VecDeque<Entry<T>>,
    /// Video is accepted again only from the next keyframe.
    wait_key: bool,
    /// Whether video skipped while waiting counts as dropped: yes after congestion, no
    /// right after (re)connecting, where waiting for the forced keyframe is expected.
    count_waiting: bool,
    connected: bool,
    closed: bool,
}

/// Bounded-by-time packet queue between the encoder and the network.
/// Pushing never blocks; when queued video spans more than `threshold` seconds,
/// video is dropped from the front up to the next keyframe (audio is kept).
pub struct SendQueue<T> {
    state: Mutex<QueueState<T>>,
    ready: Condvar,
    threshold: f64,
    pub dropped: AtomicU64,
}

impl<T> SendQueue<T> {
    pub fn new(threshold: Duration) -> Self {
        Self {
            state: Mutex::new(QueueState {
                entries: VecDeque::new(),
                wait_key: true,
                count_waiting: false,
                connected: false,
                closed: false,
            }),
            ready: Condvar::new(),
            threshold: threshold.as_secs_f64(),
            dropped: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, entry: Entry<T>) {
        let mut s = self.lock();
        if s.closed || !s.connected {
            return; // nothing to send to; the next connection starts at a fresh keyframe
        }
        if s.wait_key {
            if !(entry.video && entry.key) {
                if entry.video && s.count_waiting {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            s.wait_key = false;
        }
        s.entries.push_back(entry);
        self.trim(&mut s);
        drop(s);
        self.ready.notify_one();
    }

    fn trim(&self, s: &mut QueueState<T>) {
        while span(&s.entries) > self.threshold {
            let first_video = s.entries.iter().position(|e| e.video).unwrap_or(0);
            let next_key = s
                .entries
                .iter()
                .enumerate()
                .skip(first_video + 1)
                .find(|(_, e)| e.video && e.key)
                .map(|(i, _)| i);
            let cut = next_key.unwrap_or(s.entries.len());
            let mut i = 0;
            let mut dropped = 0;
            s.entries.retain(|e| {
                let keep = !e.video || i >= cut;
                i += 1;
                dropped += u64::from(!keep);
                keep
            });
            self.dropped.fetch_add(dropped, Ordering::Relaxed);
            if next_key.is_none() {
                s.wait_key = true;
                s.count_waiting = true;
                break;
            }
        }
    }

    /// Seconds of video currently queued.
    pub fn delay(&self) -> f64 {
        span(&self.lock().entries)
    }

    /// Next entry, waiting up to `timeout`. `None` on timeout or when closed and empty.
    pub fn pop(&self, timeout: Duration) -> Option<Entry<T>> {
        let mut s = self.lock();
        if s.entries.is_empty() && !s.closed {
            s = self
                .ready
                .wait_timeout(s, timeout)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        s.entries.pop_front()
    }

    /// Connected: start accepting packets from the next keyframe. Disconnected: discard.
    pub fn set_connected(&self, connected: bool) {
        let mut s = self.lock();
        s.entries.clear();
        s.wait_key = true;
        s.count_waiting = false;
        s.connected = connected;
    }

    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    pub fn is_closed_and_empty(&self) -> bool {
        let s = self.lock();
        s.closed && s.entries.is_empty()
    }
}

fn span<T>(entries: &VecDeque<Entry<T>>) -> f64 {
    let first = entries.iter().find(|e| e.video).map(|e| e.time);
    let last = entries.iter().rev().find(|e| e.video).map(|e| e.time);
    match (first, last) {
        (Some(a), Some(b)) => b - a,
        _ => 0.0,
    }
}

/// Polled by FFmpeg during blocking network calls: abort once Stop has been requested
/// for longer than [`STOP_GRACE`].
struct Abort {
    stop: Arc<AtomicBool>,
    since: Mutex<Option<Instant>>,
}

impl Abort {
    fn aborted(&self) -> bool {
        if !self.stop.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut since) = self.since.lock() else {
            return true;
        };
        since.get_or_insert_with(Instant::now).elapsed() >= STOP_GRACE
    }
}

extern "C" fn interrupt(opaque: *mut c_void) -> c_int {
    // SAFETY: `opaque` points to the `Abort` owned by the `Connection` that also owns the
    // context; the context is freed before the `Abort` (field order).
    let abort = unsafe { &*(opaque as *const Abort) };
    c_int::from(abort.aborted())
}

/// An open network output.
struct Connection {
    // Drop order matters: the context goes before the `Abort` its callback points to.
    output: ffmpeg::format::context::Output,
    _abort: Arc<Abort>,
    specs: Vec<StreamSpec>,
    /// Time of the first packet sent on this connection (rebased to 0).
    t0: Option<f64>,
}

impl Connection {
    fn open(
        url: &str,
        protocol: Protocol,
        bearer: &str,
        specs: &[StreamSpec],
        abort: Arc<Abort>,
    ) -> Result<Self, MediaError> {
        crate::init();
        let c_url = CString::new(url).map_err(|_| MediaError::Io("invalid URL".into()))?;
        let c_format =
            CString::new(protocol.format_name()).map_err(|_| MediaError::Io("format".into()))?;
        // SAFETY: plain FFmpeg C API use; the context is wrapped (and so freed) right after
        // allocation, and the interrupt opaque outlives it (see `Connection` field order).
        let mut output = unsafe {
            let mut ctx = std::ptr::null_mut();
            let r = ffmpeg::ffi::avformat_alloc_output_context2(
                &mut ctx,
                std::ptr::null(),
                c_format.as_ptr(),
                c_url.as_ptr(),
            );
            if r < 0 || ctx.is_null() {
                return Err(ffmpeg::Error::from(r).into());
            }
            (*ctx).interrupt_callback = ffmpeg::ffi::AVIOInterruptCB {
                callback: Some(interrupt),
                opaque: Arc::as_ptr(&abort) as *mut c_void,
            };
            let output = ffmpeg::format::context::Output::wrap(ctx);
            let nofile = ((*(*ctx).oformat).flags & ffmpeg::ffi::AVFMT_NOFILE as c_int) != 0;
            if !nofile {
                let mut opts = Dictionary::new();
                opts.set("rw_timeout", RW_TIMEOUT_US);
                let mut raw = opts.disown();
                let r = ffmpeg::ffi::avio_open2(
                    &mut (*ctx).pb,
                    c_url.as_ptr(),
                    ffmpeg::ffi::AVIO_FLAG_WRITE as c_int,
                    &(*ctx).interrupt_callback,
                    &mut raw,
                );
                drop(Dictionary::own(raw));
                if r < 0 {
                    return Err(ffmpeg::Error::from(r).into());
                }
            }
            output
        };
        for spec in specs {
            let mut stream = output.add_stream(ffmpeg::encoder::find(spec.parameters.id()))?;
            stream.set_parameters(spec.parameters.clone());
            stream.set_time_base(spec.time_base);
            // SAFETY: resetting the codec tag lets the muxer pick its own (as in `Muxer`).
            unsafe {
                (*stream.parameters().as_ptr().cast_mut()).codec_tag = 0;
            }
        }
        let mut options = Dictionary::new();
        match protocol {
            Protocol::Rtmp => options.set("flvflags", "no_duration_filesize"),
            Protocol::Whip if !bearer.is_empty() => options.set("authorization", bearer),
            _ => {}
        }
        output.write_header_with(options)?;
        Ok(Self {
            output,
            _abort: abort,
            specs: specs.to_vec(),
            t0: None,
        })
    }

    /// Writes one packet with timestamps rebased to this connection. Returns bytes written.
    fn write(&mut self, entry: Entry<ffmpeg::Packet>) -> Result<u64, MediaError> {
        let t0 = *self.t0.get_or_insert(entry.time);
        if entry.time < t0 {
            return Ok(0); // audio from just before the first keyframe
        }
        let Some(spec) = self.specs.get(entry.stream) else {
            return Ok(0);
        };
        let tb = spec.time_base;
        let offset = (t0 * tb.1 as f64 / tb.0.max(1) as f64).round() as i64;
        let mut packet = entry.payload;
        packet.set_pts(packet.pts().map(|v| v - offset));
        packet.set_dts(packet.dts().map(|v| v - offset));
        let target = self
            .output
            .stream(entry.stream)
            .map(|s| s.time_base())
            .unwrap_or(tb);
        packet.rescale_ts(tb, target);
        packet.set_stream(entry.stream);
        packet.set_position(-1);
        let size = packet.size() as u64;
        packet.write_interleaved(&mut self.output)?;
        Ok(size)
    }

    fn finish(mut self) {
        if let Err(e) = self.output.write_trailer() {
            tracing::debug!(error = %e, "stream trailer");
        }
    }
}

/// The network target of an encoder session.
pub(crate) struct NetTarget {
    pub queue: Arc<SendQueue<ffmpeg::Packet>>,
    pub force_key: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl NetTarget {
    pub fn start(
        settings: StreamSettings,
        specs: Vec<StreamSpec>,
        stop: Arc<AtomicBool>,
        stats: Arc<OutputStats>,
    ) -> Result<Self, MediaError> {
        let queue = Arc::new(SendQueue::new(Duration::from_millis(
            settings.drop_threshold_ms.clamp(200, 10_000) as u64,
        )));
        let force_key = Arc::new(AtomicBool::new(false));
        let thread = {
            let (queue, force_key) = (queue.clone(), force_key.clone());
            std::thread::Builder::new()
                .name("estream-stream-send".into())
                .spawn(move || send_loop(settings, specs, queue, force_key, stop, stats))
                .map_err(|e| MediaError::Io(e.to_string()))?
        };
        Ok(Self {
            queue,
            force_key,
            thread: Some(thread),
        })
    }

    /// Lets the sender flush what is queued (bounded by the stop grace) and joins it.
    pub fn finish(mut self) {
        self.queue.close();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn send_loop(
    settings: StreamSettings,
    specs: Vec<StreamSpec>,
    queue: Arc<SendQueue<ffmpeg::Packet>>,
    force_key: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    stats: Arc<OutputStats>,
) {
    let url = settings.url();
    let shown = redact(&url);
    let protocol = settings.protocol().unwrap_or(Protocol::Rtmp);
    let bearer = if protocol == Protocol::Whip {
        settings.key.trim().to_owned()
    } else {
        String::new()
    };
    let net = &stats.network;
    let mut attempt = 0u32;
    let fail = |message: String| {
        tracing::error!(url = %shown, %message, "stream failed");
        if let Ok(mut slot) = stats.error.lock() {
            *slot = Some(message);
        }
    };
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        net.set_state(if attempt == 0 {
            StreamState::Connecting
        } else {
            StreamState::Reconnecting
        });
        let abort = Arc::new(Abort {
            stop: stop.clone(),
            since: Mutex::new(None),
        });
        match Connection::open(&url, protocol, &bearer, &specs, abort) {
            Ok(mut conn) => {
                tracing::info!(url = %shown, protocol = protocol.label(), "stream connected");
                if attempt > 0 {
                    net.reconnects.fetch_add(1, Ordering::Relaxed);
                }
                attempt = 0;
                net.attempt.store(0, Ordering::Relaxed);
                queue.set_connected(true);
                force_key.store(true, Ordering::Release);
                net.set_state(StreamState::Live);
                let error = loop {
                    if stop.load(Ordering::Acquire) && queue.is_closed_and_empty() {
                        break None;
                    }
                    net.queue_ms
                        .store((queue.delay() * 1000.0) as u64, Ordering::Relaxed);
                    net.dropped
                        .store(queue.dropped.load(Ordering::Relaxed), Ordering::Relaxed);
                    let Some(entry) = queue.pop(Duration::from_millis(100)) else {
                        continue;
                    };
                    match conn.write(entry) {
                        Ok(n) => {
                            stats.bytes.fetch_add(n, Ordering::Relaxed);
                        }
                        Err(e) => break Some(e),
                    }
                };
                queue.set_connected(false);
                match error {
                    None => {
                        conn.finish();
                        break;
                    }
                    Some(e) => {
                        drop(conn);
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        tracing::warn!(url = %shown, error = %e, "stream disconnected");
                    }
                }
            }
            Err(e) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                tracing::warn!(url = %shown, error = %e, attempt, "stream connect failed");
                if !settings.reconnect || attempt >= settings.max_retries {
                    fail(if attempt == 0 {
                        format!("Couldn't connect to {shown}: {e}")
                    } else {
                        format!("Gave up reconnecting to {shown} after {attempt} attempts: {e}")
                    });
                    break;
                }
            }
        }
        attempt += 1;
        net.attempt.store(attempt, Ordering::Relaxed);
        net.set_state(StreamState::Reconnecting);
        let until = Instant::now() + settings.retry_delay(attempt);
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    net.dropped
        .store(queue.dropped.load(Ordering::Relaxed), Ordering::Relaxed);
    net.set_state(StreamState::Stopped);
    tracing::info!(url = %shown, "stream stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> SendQueue<u32> {
        let q = SendQueue::new(Duration::from_millis(500));
        q.set_connected(true);
        q
    }

    fn video(q: &SendQueue<u32>, n: u32, key: bool) {
        q.push(Entry {
            stream: 0,
            payload: n,
            video: true,
            key,
            time: n as f64 * 0.1,
        });
    }

    fn audio(q: &SendQueue<u32>, n: u32) {
        q.push(Entry {
            stream: 1,
            payload: 1000 + n,
            video: false,
            key: true,
            time: n as f64 * 0.1,
        });
    }

    fn drain(q: &SendQueue<u32>) -> Vec<u32> {
        std::iter::from_fn(|| q.pop(Duration::ZERO).map(|e| e.payload)).collect()
    }

    #[test]
    fn waits_for_a_keyframe_first() {
        let q = queue();
        video(&q, 0, false);
        audio(&q, 0);
        video(&q, 1, true);
        audio(&q, 1);
        assert_eq!(drain(&q), [1, 1001]);
        // Waiting for the first keyframe after connecting is not a network drop.
        assert_eq!(q.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn congestion_drops_video_to_the_next_keyframe_and_keeps_audio() {
        let q = queue();
        // Keyframes every 3 frames (0.3 s); threshold 0.5 s.
        for n in 0..8 {
            video(&q, n, n % 3 == 0);
            audio(&q, n);
        }
        let out = drain(&q);
        let videos: Vec<u32> = out.iter().copied().filter(|v| *v < 1000).collect();
        let audios = out.iter().filter(|v| **v >= 1000).count();
        assert_eq!(videos.first(), Some(&3), "starts at a keyframe: {out:?}");
        assert_eq!(videos, [3, 4, 5, 6, 7]);
        assert_eq!(audios, 8, "audio is never dropped");
        assert_eq!(q.dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn congestion_without_a_later_keyframe_waits_for_one() {
        let q = queue();
        video(&q, 0, true);
        for n in 1..8 {
            video(&q, n, false);
        }
        assert!(drain(&q).is_empty());
        video(&q, 8, false);
        video(&q, 9, true);
        assert_eq!(drain(&q), [9]);
        // v0..v6 dropped by congestion, then v7 and v8 while waiting for a keyframe.
        assert_eq!(q.dropped.load(Ordering::Relaxed), 9);
    }

    #[test]
    fn disconnected_queue_discards() {
        let q = queue();
        q.set_connected(false);
        video(&q, 0, true);
        assert!(drain(&q).is_empty());
        assert_eq!(q.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn closed_queue_returns_promptly() {
        let q = queue();
        q.close();
        let t = Instant::now();
        assert!(q.pop(Duration::from_secs(5)).is_none());
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
