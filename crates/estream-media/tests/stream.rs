//! End-to-end streaming tests against the bundled `ffmpeg.exe` acting as a local server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{engine, probe, temp_dir};
use estream_media::{Stream, StreamSettings, StreamState, VideoCodec, encode};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

fn rtmp_server(exe: &Path, port: u16, out: &Path) -> Child {
    let url = format!("rtmp://127.0.0.1:{port}/live/test-key");
    server(exe, &["-listen", "1", "-timeout", "30", "-i", &url], out)
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

fn wait_for(stream: &Stream, state: StreamState, timeout: Duration) -> bool {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if stream.state() == state {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
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

fn stream_rtmp(codec: VideoCodec, expect: &str) {
    let Some(exe) = ffmpeg_cli() else {
        eprintln!("ffmpeg CLI not bundled; skipping");
        return;
    };
    let engine = engine();
    let dir = temp_dir(&format!("rtmp-{expect}"));
    let out = dir.join("received.flv");
    let port = free_tcp_port();
    let mut srv = rtmp_server(&exe, port, &out);
    std::thread::sleep(Duration::from_millis(1500));

    let stream = Stream::start(
        &engine,
        &StreamSettings {
            codec,
            ..rtmp(port)
        },
    )
    .unwrap();
    assert!(
        !stream.destination.contains("test-key"),
        "{}",
        stream.destination
    );
    assert!(
        wait_for(&stream, StreamState::Live, Duration::from_secs(10)),
        "state {:?}",
        stream.state()
    );
    std::thread::sleep(Duration::from_secs(3));
    let stats = stream.stats();
    let relaxed = std::sync::atomic::Ordering::Relaxed;
    let sent = stats.bytes.load(relaxed);
    eprintln!(
        "frames {} net-dropped {} queue {} ms",
        stats.frames.load(relaxed),
        stats.network.dropped.load(relaxed),
        stats.network.queue_ms.load(relaxed)
    );
    stream.stop(&engine).unwrap();
    wait_exit(&mut srv, Duration::from_secs(10));

    let p = probe(&out);
    eprintln!(
        "{expect}: sent {sent} B, received {:?} audio {:?} spans {:?}",
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
    std::thread::sleep(Duration::from_millis(1500));

    let stream = Stream::start(
        &engine,
        &StreamSettings {
            server: format!("srt://127.0.0.1:{port}"),
            bitrate_kbps: 2_000,
            retry_delay_seconds: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(wait_for(
        &stream,
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
    let out = dir.join("received.flv");
    let port = free_tcp_port();

    let stream = Stream::start(&engine, &rtmp(port)).unwrap();
    // A refused connect to localhost takes ~5 s on Windows (SYN retries).
    assert!(
        wait_for(&stream, StreamState::Reconnecting, Duration::from_secs(15)),
        "nothing listens yet: {:?}",
        stream.state()
    );
    let mut srv = rtmp_server(&exe, port, &out);
    assert!(
        wait_for(&stream, StreamState::Live, Duration::from_secs(15)),
        "state {:?}",
        stream.state()
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
    let stream = Stream::start(&engine, &rtmp(port)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(stream.state(), StreamState::Connecting);
    let t = Instant::now();
    stream.stop(&engine).unwrap();
    let took = t.elapsed();
    eprintln!("stop took {took:?}");
    assert!(took < Duration::from_secs(2), "stop took {took:?}");
}

#[test]
fn invalid_settings_are_refused_up_front() {
    let engine = engine();
    let err = Stream::start(
        &engine,
        &StreamSettings {
            server: "rtmp://127.0.0.1/live".into(),
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("stream key"), "{err}");
}
