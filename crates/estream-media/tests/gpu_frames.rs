//! Frames handed to NVENC on the GPU encode to the same picture as CPU frames.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use estream_core::{Rational, Timestamp};
use estream_engine::{Engine, EngineConfig, GpuExport, VideoFrame};
use estream_media::encode::{self, VideoEncoder};
use estream_media::ffmpeg;
use estream_media::{RateControl, RecordingSettings, VideoCodec};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FRAMES: usize = 30;

/// The common test-pattern engine, with GPU frames enabled.
fn engine() -> Option<Engine> {
    let gpu = common::gpu();
    let export: Arc<dyn GpuExport> = estream_media::gpu_frames::export(&gpu)?;
    let template = common::engine();
    let project = template.snapshot().as_ref().clone();
    drop(template);
    Some(
        Engine::start(
            project,
            EngineConfig {
                gpu: Some(gpu),
                gpu_export: Some(export),
                audio: false,
                ..Default::default()
            },
        )
        .unwrap(),
    )
}

fn settings() -> RecordingSettings {
    RecordingSettings {
        codec: VideoCodec::H264,
        rate_control: RateControl::ConstantQuality,
        quality: 16,
        ..Default::default()
    }
}

/// Encodes GPU frames as they arrive (they hold the engine's GPU slots until dropped), keeps
/// the CPU frames, then encodes the CPU frames with the same timestamps. Returns the decoded
/// luma planes of both, in order.
fn encode_both(engine: &Engine, name: &str) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let canvas = engine.snapshot().canvases[0].id;
    let tap = engine.video_tap();
    let (cpu_tx, cpu_rx) = crossbeam_channel::bounded(256);
    let (gpu_tx, gpu_rx) = crossbeam_channel::bounded(8);
    let cpu_id = tap.add(canvas, cpu_tx);
    let (gpu_id, wants_gpu) = tap.add_gpu(canvas, gpu_tx);
    let mut cpu = BTreeMap::<Timestamp, VideoFrame>::new();
    let mut gpu_times = Vec::new();
    let mut gpu_enc = None;
    let mut gpu_packets = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while gpu_times.len() < FRAMES && Instant::now() < deadline {
        while let Ok(f) = cpu_rx.try_recv() {
            cpu.insert(f.timestamp, f);
        }
        let Ok(f) = gpu_rx.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        let g = f.gpu.as_ref().expect("a GPU consumer got a CPU frame");
        let enc = gpu_enc.get_or_insert_with(|| {
            VideoEncoder::open_gpu(name, f.width, f.height, Rational::fps(30), &settings(), g)
                .unwrap()
        });
        enc.encode(&f, gpu_times.len() as i64, false, &mut |p| {
            gpu_packets.push(p)
        })
        .unwrap();
        gpu_times.push(f.timestamp);
    }
    // CPU frames for the last GPU timestamps may still be on their way.
    std::thread::sleep(Duration::from_millis(200));
    while let Ok(f) = cpu_rx.try_recv() {
        cpu.insert(f.timestamp, f);
    }
    tap.remove(cpu_id);
    tap.remove(gpu_id);
    assert!(
        wants_gpu.load(std::sync::atomic::Ordering::Relaxed),
        "the engine fell back to CPU frames"
    );
    assert_eq!(gpu_times.len(), FRAMES, "GPU frames stopped arriving");
    let mut gpu_enc = gpu_enc.unwrap();
    gpu_enc.finish(&mut |p| gpu_packets.push(p)).unwrap();
    let gpu_planes = decode(&gpu_enc, &gpu_packets);

    let first = &cpu[&gpu_times[0]];
    let mut cpu_enc = VideoEncoder::open(
        name,
        first.width,
        first.height,
        Rational::fps(30),
        &settings(),
    )
    .unwrap();
    let mut cpu_packets = Vec::new();
    for (i, t) in gpu_times.iter().enumerate() {
        let f = cpu
            .get(t)
            .expect("no CPU frame for a GPU frame's timestamp");
        cpu_enc
            .encode(f, i as i64, false, &mut |p| cpu_packets.push(p))
            .unwrap();
    }
    cpu_enc.finish(&mut |p| cpu_packets.push(p)).unwrap();
    (decode(&cpu_enc, &cpu_packets), gpu_planes)
}

/// Each decoded frame's luma plane.
fn decode(enc: &VideoEncoder, packets: &[ffmpeg::Packet]) -> Vec<Vec<u8>> {
    let mut dec = ffmpeg::codec::context::Context::from_parameters(enc.parameters())
        .unwrap()
        .decoder()
        .video()
        .unwrap();
    let mut out = Vec::new();
    let mut take = |dec: &mut ffmpeg::decoder::Video| {
        let mut frame = ffmpeg::frame::Video::empty();
        while dec.receive_frame(&mut frame).is_ok() {
            let (fw, fh, stride) = (
                frame.width() as usize,
                frame.height() as usize,
                frame.stride(0),
            );
            let data = frame.data(0);
            out.push(
                (0..fh)
                    .flat_map(|r| data[r * stride..r * stride + fw].to_vec())
                    .collect(),
            );
        }
    };
    for p in packets {
        dec.send_packet(p).unwrap();
        take(&mut dec);
    }
    dec.send_eof().unwrap();
    take(&mut dec);
    out
}

/// A recording with NVENC on an engine with GPU frames uses them and writes a full file.
#[test]
fn recording_with_nvenc_uses_gpu_frames() {
    estream_media::init();
    if !encode::available(VideoCodec::H264)
        .iter()
        .any(|e| e.name == "h264_nvenc")
    {
        eprintln!("skipping: no NVENC");
        return;
    }
    let Some(engine) = engine() else {
        eprintln!("skipping: GPU frames unavailable");
        return;
    };
    let dir = common::temp_dir("gpu-frames");
    let settings = RecordingSettings {
        folder: dir.clone(),
        encoder: "h264_nvenc".into(),
        ..settings()
    };
    let rec = estream_media::Recording::start(&engine, &settings).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let stats = rec.stats().clone();
    let path = rec.stop(&engine).unwrap();
    assert!(
        stats.gpu_frames.load(std::sync::atomic::Ordering::Relaxed),
        "the recording used CPU frames"
    );
    let frames = stats.frames.load(std::sync::atomic::Ordering::Relaxed);
    assert!(frames >= 50, "only {frames} frames in 2 s at 30 fps");
    let p = common::probe(&path);
    assert_eq!(p.video.as_ref().map(|v| (v.1, v.2)), Some((640, 360)));
    assert!(p.starts_with_key);
    assert!(p.duration > 1.5, "{}", p.duration);
}

#[test]
fn gpu_frames_encode_like_cpu_frames() {
    estream_media::init();
    let Some(nvenc) = encode::available(VideoCodec::H264)
        .into_iter()
        .find(|e| e.name == "h264_nvenc")
    else {
        eprintln!("skipping: no NVENC");
        return;
    };
    let Some(engine) = engine() else {
        eprintln!("skipping: GPU frames unavailable");
        return;
    };
    let (a, b) = encode_both(&engine, nvenc.name);
    assert_eq!(a.len(), FRAMES);
    assert_eq!(b.len(), FRAMES);
    let mean_diff = |x: &[u8], y: &[u8]| {
        x.iter()
            .zip(y)
            .map(|(p, q)| (*p as i32 - *q as i32).unsigned_abs() as u64)
            .sum::<u64>() as f64
            / x.len() as f64
    };
    // The pattern's moving marker sits in rows 80–95% down; a stale or shifted frame shows
    // up there clearly even though it barely moves the whole-frame average.
    let (w, h) = (640, 360);
    let band = (h * 80 / 100) * w..(h * 95 / 100) * w;
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let all = mean_diff(x, y);
        let marker = mean_diff(&x[band.clone()], &y[band.clone()]);
        assert!(all < 1.5, "frame {i}: mean luma difference {all:.2}");
        assert!(
            marker < 3.0,
            "frame {i}: marker band differs by {marker:.2}"
        );
    }
}
