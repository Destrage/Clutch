//! End-to-end streaming tests against the bundled `ffmpeg.exe` acting as a local server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Probe, engine, probe, temp_dir};
use estream_media::{Destination, MultiStream, StreamSettings, StreamState, VideoCodec, encode};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

/// The bundled FFmpeg CLI (set up by `.cargo/config.toml`), if present.
fn ffmpeg_cli() -> Option<PathBuf> {
    let exe = PathBuf::from(option_env!("FFMPEG_DIR")?)
        .join("bin")
        .join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
    exe.exists().then_some(exe)
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Runs `ffmpeg -i <input> -c copy <out>` as a receiving server.
fn server(exe: &Path, input_args: &[&str], out: &Path) -> Child {
    Command::new(exe)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(input_args)
        .args(["-c", "copy"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// A local RTMP server writing what it receives to `<dir>/<name>.flv`.
struct RtmpServer {
    child: Child,
    port: u16,
    out: PathBuf,
}

impl RtmpServer {
    fn start(exe: &Path, dir: &Path, name: &str) -> Self {
        let port = free_tcp_port();
        let out = dir.join(format!("{name}.flv"));
        let url = format!("rtmp://127.0.0.1:{port}/live/test-key");
        let child = server(exe, &["-listen", "1", "-timeout", "30", "-i", &url], &out);
        Self { child, port, out }
    }

    /// Waits for the server to finish (the publisher disconnected) and probes the file.
    fn received(mut self) -> Probe {
        wait_exit(&mut self.child, Duration::from_secs(10));
        probe(&self.out)
    }
}

fn wait_exit(child: &mut Child, timeout: Duration) {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_for(stream: &MultiStream, index: usize, state: StreamState, timeout: Duration) -> bool {
    wait_until(timeout, || stream.destinations()[index].state() == state)
}

fn rtmp(port: u16) -> StreamSettings {
    StreamSettings {
        server: format!("rtmp://127.0.0.1:{port}/live"),
        key: "test-key".into(),
        bitrate_kbps: 2_000,
        retry_delay_seconds: 1,
        ..Default::default()
    }
}

fn dest(name: &str, settings: StreamSettings) -> Destination {
    Destination {
        name: name.into(),
        settings,
    }
}

/// Lets the servers start listening before publishers connect.
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

fn stream_rtmp(codec: VideoCodec, expect: &str) {
    let Some(exe) = ffmpeg_cli() else {
        eprintln!("ffmpeg CLI not bundled; skipping");
        return;
    };
    let engine = engine();
    let dir = temp_dir(&format!("rtmp-{expect}"));
    let srv = RtmpServer::start(&exe, &dir, "received");
    settle();

    let stream = MultiStream::start(
        &engine,
        &[dest(
            "Local",
            StreamSettings {
                codec,
                ..rtmp(srv.port)
            },
        )],
    )
    .unwrap();
    let d = &stream.destinations()[0];
    assert!(
        !d.stats.destination.contains("test-key"),
        "{}",
        d.stats.destination
    );
    assert!(
        wait_for(&stream, 0, StreamState::Live, Duration::from_secs(10)),
        "state {:?}",
        stream.destinations()[0].state()
    );
    std::thread::sleep(Duration::from_secs(3));
    let d = &stream.destinations()[0];
    eprintln!(
        "{expect}: sent {} B, net-dropped {}",
        d.stats.bytes.load(Relaxed),
        d.stats.network.dropped.load(Relaxed)
    );
    stream.stop(&engine).unwrap();

    let p = srv.received();
    eprintln!(
        "{expect}: {:?} {:?} spans {:?}",
        p.video, p.audio, p.stream_durations
    );
    let (vcodec, w, h) = p.video.clone().unwrap();
    assert!(vcodec.contains(expect), "{vcodec}");
    assert_eq!((w, h), (640, 360));
    assert_eq!(p.audio, ["AAC"]);
    assert!(p.starts_with_key);
    assert!(
        p.stream_durations[0] > 2.0 && p.stream_durations[0] < 4.5,
        "video span {:?}",
        p.stream_durations
    );
}

#[test]
fn rtmp_h264_reaches_the_server() {
    stream_rtmp(VideoCodec::H264, "H264");
}

#[test]
fn enhanced_rtmp_carries_hevc() {
    if !encode::available(VideoCodec::Hevc)
        .iter()
        .any(|e| e.hardware)
    {
        eprintln!("no hardware HEVC encoder; skipping");
        return;
    }
    stream_rtmp(VideoCodec::Hevc, "HEVC");
}

#[test]
fn enhanced_rtmp_carries_av1() {
    if !encode::available(VideoCodec::Av1)
        .iter()
        .any(|e| e.hardware)
    {
        eprintln!("no hardware AV1 encoder; skipping");
        return;
    }
    stream_rtmp(VideoCodec::Av1, "AV1");
}

#[test]
fn srt_reaches_the_server() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("srt");
    let out = dir.join("received.ts");
    let port = free_udp_port();
    let url = format!("srt://127.0.0.1:{port}?mode=listener");
    let mut srv = server(&exe, &["-i", &url], &out);
    settle();

    let stream = MultiStream::start(
        &engine,
        &[dest(
            "SRT",
            StreamSettings {
                server: format!("srt://127.0.0.1:{port}"),
                bitrate_kbps: 2_000,
                retry_delay_seconds: 1,
                ..Default::default()
            },
        )],
    )
    .unwrap();
    assert!(wait_for(
        &stream,
        0,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    std::thread::sleep(Duration::from_secs(3));
    stream.stop(&engine).unwrap();
    wait_exit(&mut srv, Duration::from_secs(5));

    let p = probe(&out);
    eprintln!(
        "srt: {:?} {:?} spans {:?}",
        p.video, p.audio, p.stream_durations
    );
    assert!(p.video.unwrap().0.contains("H264"));
    assert_eq!(p.audio, ["AAC"]);
    assert!(p.stream_durations[0] > 2.0, "{:?}", p.stream_durations);
}

#[test]
fn reconnects_when_the_server_appears_later() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("reconnect");
    let port = free_tcp_port();

    let stream = MultiStream::start(&engine, &[dest("Late", rtmp(port))]).unwrap();
    // A refused connect to localhost takes ~5 s on Windows (SYN retries).
    assert!(
        wait_for(
            &stream,
            0,
            StreamState::Reconnecting,
            Duration::from_secs(15)
        ),
        "nothing listens yet: {:?}",
        stream.destinations()[0].state()
    );
    // Start the server on the same port the destination keeps retrying.
    let out = dir.join("received.flv");
    let url = format!("rtmp://127.0.0.1:{port}/live/test-key");
    let mut srv = server(&exe, &["-listen", "1", "-timeout", "30", "-i", &url], &out);
    assert!(
        wait_for(&stream, 0, StreamState::Live, Duration::from_secs(15)),
        "state {:?}",
        stream.destinations()[0].state()
    );
    std::thread::sleep(Duration::from_secs(2));
    stream.stop(&engine).unwrap();
    wait_exit(&mut srv, Duration::from_secs(10));

    let p = probe(&out);
    assert!(p.starts_with_key, "the new connection starts at a keyframe");
    assert!(p.stream_durations[0] > 1.0, "{:?}", p.stream_durations);
}

#[test]
fn stop_is_prompt_while_the_server_hangs() {
    let engine = engine();
    // Accepts TCP connections (via the OS backlog) but never answers the RTMP handshake.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let stream = MultiStream::start(&engine, &[dest("Hang", rtmp(port))]).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(stream.destinations()[0].state(), StreamState::Connecting);
    let t = Instant::now();
    stream.stop(&engine).unwrap();
    let took = t.elapsed();
    eprintln!("stop took {took:?}");
    assert!(took < Duration::from_secs(2), "stop took {took:?}");
}

#[test]
fn invalid_settings_are_refused_up_front() {
    let engine = engine();
    let err = MultiStream::start(
        &engine,
        &[dest(
            "No key",
            StreamSettings {
                server: "rtmp://127.0.0.1/live".into(),
                ..Default::default()
            },
        )],
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("stream key"), "{err}");
}

// ---------------------------------------------------------------- multistream

#[test]
fn two_destinations_share_one_encoder() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("shared");
    let a = RtmpServer::start(&exe, &dir, "a");
    let b = RtmpServer::start(&exe, &dir, "b");
    settle();

    let stream =
        MultiStream::start(&engine, &[dest("A", rtmp(a.port)), dest("B", rtmp(b.port))]).unwrap();
    assert_eq!(
        stream.encoder_count(),
        1,
        "identical settings share an encoder"
    );
    assert!(wait_for(
        &stream,
        0,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    assert!(wait_for(
        &stream,
        1,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    std::thread::sleep(Duration::from_secs(3));
    stream.stop(&engine).unwrap();

    for (name, p) in [("A", a.received()), ("B", b.received())] {
        eprintln!("{name}: {:?} spans {:?}", p.video, p.stream_durations);
        assert!(p.starts_with_key, "{name}");
        assert_eq!(p.audio, ["AAC"], "{name}");
        assert!(
            p.stream_durations[0] > 2.0,
            "{name}: {:?}",
            p.stream_durations
        );
    }
}

#[test]
fn different_settings_use_separate_encoders() {
    let engine = engine();
    // Nothing listens: only the encoder layout matters here.
    let (p1, p2) = (free_tcp_port(), free_tcp_port());
    let stream = MultiStream::start(
        &engine,
        &[
            dest("High", rtmp(p1)),
            dest(
                "Low",
                StreamSettings {
                    bitrate_kbps: 1_000,
                    ..rtmp(p2)
                },
            ),
        ],
    )
    .unwrap();
    assert_eq!(stream.encoder_count(), 2);
    stream.stop(&engine).unwrap();
}

#[test]
fn a_failing_destination_does_not_affect_the_others() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("isolation");
    let good = RtmpServer::start(&exe, &dir, "good");
    settle();
    let dead = free_tcp_port();

    let mut stream = MultiStream::start(
        &engine,
        &[
            dest("Good", rtmp(good.port)),
            dest(
                "Dead",
                StreamSettings {
                    reconnect: false,
                    ..rtmp(dead)
                },
            ),
        ],
    )
    .unwrap();
    assert!(wait_for(
        &stream,
        0,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    assert!(
        wait_until(Duration::from_secs(15), || stream.destinations()[1]
            .error()
            .is_some()),
        "the dead destination reports an error"
    );
    eprintln!("dead: {:?}", stream.destinations()[1].error());
    assert_eq!(stream.destinations()[1].state(), StreamState::Stopped);
    stream.poll(&engine);
    assert_eq!(stream.destinations()[0].state(), StreamState::Live);
    assert!(stream.is_active());
    std::thread::sleep(Duration::from_secs(1));
    let live_for = stream.destinations()[0].elapsed().as_secs_f64();
    stream.stop(&engine).unwrap();

    let p = good.received();
    eprintln!(
        "good: spans {:?}, live for {live_for:.1}s",
        p.stream_durations
    );
    assert!(
        p.stream_durations[0] > live_for - 2.5,
        "the good destination kept streaming: {:?} vs {live_for}",
        p.stream_durations
    );
}

#[test]
fn stop_one_and_add_one_while_live() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("live-edit");
    let a = RtmpServer::start(&exe, &dir, "a");
    let b = RtmpServer::start(&exe, &dir, "b");
    let c = RtmpServer::start(&exe, &dir, "c");
    settle();

    let mut stream =
        MultiStream::start(&engine, &[dest("A", rtmp(a.port)), dest("B", rtmp(b.port))]).unwrap();
    assert!(wait_for(
        &stream,
        0,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    assert!(wait_for(
        &stream,
        1,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    std::thread::sleep(Duration::from_secs(2));

    // Stop B only.
    stream.stop_destination(&engine, 1);
    assert_eq!(stream.destinations()[1].state(), StreamState::Stopped);
    assert_eq!(stream.destinations()[0].state(), StreamState::Live);
    let b = b.received();

    // Add C while live: it joins A's encoder.
    let index = stream.add_destination(&engine, &dest("C", rtmp(c.port)));
    assert_eq!(stream.encoder_count(), 1, "C joins the running encoder");
    assert!(wait_for(
        &stream,
        index,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    std::thread::sleep(Duration::from_secs(2));
    stream.stop(&engine).unwrap();

    let (a, c) = (a.received(), c.received());
    eprintln!(
        "a {:?} | b {:?} | c {:?}",
        a.stream_durations, b.stream_durations, c.stream_durations
    );
    assert!(b.stream_durations[0] > 1.5 && b.stream_durations[0] < 3.5);
    assert!(
        a.stream_durations[0] > 3.5,
        "A ran through: {:?}",
        a.stream_durations
    );
    assert!(c.starts_with_key, "C starts at a keyframe");
    assert!(c.stream_durations[0] > 1.0, "{:?}", c.stream_durations);
}

#[test]
fn invalid_destinations_do_not_block_valid_ones() {
    let Some(exe) = ffmpeg_cli() else {
        return;
    };
    let engine = engine();
    let dir = temp_dir("partial");
    let a = RtmpServer::start(&exe, &dir, "a");
    settle();
    let stream = MultiStream::start(
        &engine,
        &[
            dest(
                "Missing key",
                StreamSettings {
                    server: "rtmp://127.0.0.1/live".into(),
                    ..Default::default()
                },
            ),
            dest("A", rtmp(a.port)),
        ],
    )
    .unwrap();
    let invalid = &stream.destinations()[0];
    assert_eq!(invalid.state(), StreamState::Stopped);
    assert!(invalid.error().unwrap().contains("stream key"));
    assert!(wait_for(
        &stream,
        1,
        StreamState::Live,
        Duration::from_secs(10)
    ));
    std::thread::sleep(Duration::from_secs(1));
    stream.stop(&engine).unwrap();
    assert!(a.received().video.is_some());
}
