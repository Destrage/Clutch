//! Trimming clips to an in and out point.
//!
//! - **Lossless** copies the packets from the keyframe at or before the in point, so the clip
//!   may start a little early, but nothing is re-encoded.
//! - **Precise** decodes and re-encodes the kept range, so the first frame is the one at the in
//!   point. Clips are short, so re-encoding the whole range is quick with a hardware encoder.
//!
//! Both write to a temporary file next to the output and rename it at the end, so the output
//! (which may be the input itself) is only replaced once the trimmed file is complete.

use crate::MediaError;
use crate::encode::{self, AudioEncoder, VideoEncoder};
use crate::ffmpeg::{self, Rational as FfRational, codec, frame, media};
use crate::mux::{Muxer, StreamSpec};
use crate::player::{PixelLayout, convert_audio, convert_video};
use crate::settings::{AudioCodec, Container, Preset, RateControl, RecordingSettings, VideoCodec};
use estream_audio::{BLOCK, Block, SAMPLE_RATE};
use estream_core::{Rational, Timestamp};
use estream_engine::VideoFrame;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimMode {
    Lossless,
    Precise,
}

/// The source range that was kept, in seconds from the start of the input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trimmed {
    pub start: f64,
    pub end: f64,
}

/// Trims `input` to `from..to` (seconds from its start) into `output`.
///
/// `progress` receives 0..=1 and returns false to cancel; a cancelled trim leaves no file.
pub fn trim(
    input: &Path,
    output: &Path,
    from: f64,
    to: f64,
    mode: TrimMode,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Trimmed, MediaError> {
    crate::init();
    if from.is_nan() || to.is_nan() || to <= from {
        return Err(MediaError::Io(
            "the out point must be after the in point".into(),
        ));
    }
    let container = container_for(output)?;
    let temp = temp_path(output);
    let result = match mode {
        TrimMode::Lossless => lossless(input, &temp, container, from.max(0.0), to, progress),
        TrimMode::Precise => precise(input, &temp, container, from.max(0.0), to, progress),
    };
    match result {
        Ok(kept) => {
            std::fs::rename(&temp, output).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                MediaError::Io(e.to_string())
            })?;
            Ok(kept)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

fn container_for(path: &Path) -> Result<Container, MediaError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" => Ok(Container::Mp4),
        "mov" => Ok(Container::Mov),
        "mkv" => Ok(Container::Mkv),
        _ => Err(MediaError::Io(format!("can't write a .{ext} file"))),
    }
}

/// `<stem>.part.<ext>` next to `output`, so the final rename stays on one drive.
fn temp_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip");
    let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    output.with_file_name(format!("{stem}.part.{ext}"))
}

/// A stream's start time, or 0 when it has none.
fn start_ticks(start: i64) -> i64 {
    if start == ffmpeg::ffi::AV_NOPTS_VALUE {
        0
    } else {
        start
    }
}

/// Seconds of `ts` from the stream's start.
fn seconds(ts: i64, tb: FfRational, start: i64) -> f64 {
    (ts - start_ticks(start)) as f64 * tb.0 as f64 / tb.1.max(1) as f64
}

/// `secs` in units of `tb`.
fn ticks(secs: f64, tb: FfRational) -> i64 {
    (secs * tb.1 as f64 / tb.0.max(1) as f64).round() as i64
}

/// Seeks so the next video packet is the keyframe at or before `at` seconds.
fn seek(input: &mut ffmpeg::format::context::Input, at: f64) {
    if at > 0.0 {
        let ts = (at * 1e6) as i64;
        let _ = input.seek(ts, ..ts);
    }
}

struct Source {
    /// Input stream index → output stream index.
    map: Vec<Option<usize>>,
    specs: Vec<StreamSpec>,
    /// Per output stream: input time base and start time.
    clocks: Vec<(FfRational, i64)>,
}

fn source_streams(input: &ffmpeg::format::context::Input) -> Result<Source, MediaError> {
    let mut source = Source {
        map: Vec::new(),
        specs: Vec::new(),
        clocks: Vec::new(),
    };
    for stream in input.streams() {
        let medium = stream.parameters().medium();
        if !matches!(medium, media::Type::Video | media::Type::Audio) {
            source.map.push(None);
            continue;
        }
        source.map.push(Some(source.specs.len()));
        source.specs.push(StreamSpec {
            parameters: stream.parameters(),
            time_base: stream.time_base(),
            is_video: medium == media::Type::Video,
        });
        source
            .clocks
            .push((stream.time_base(), stream.start_time()));
    }
    if !source.specs.iter().any(|s| s.is_video) {
        return Err(MediaError::Io("this file has no video".into()));
    }
    Ok(source)
}

fn lossless(
    input: &Path,
    temp: &Path,
    container: Container,
    from: f64,
    to: f64,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Trimmed, MediaError> {
    let mut ictx = ffmpeg::format::input(input)?;
    let source = source_streams(&ictx)?;
    seek(&mut ictx, from);
    let mut muxer = Muxer::create(temp, container, &source.specs)?;
    let mut start: Option<f64> = None;
    let mut end = from;
    let mut video_done = false;
    let mut packet = ffmpeg::Packet::empty();
    loop {
        match packet.read(&mut ictx) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(e.into()),
        }
        let Some(Some(index)) = source.map.get(packet.stream()).copied() else {
            continue;
        };
        let (tb, st) = source.clocks[index];
        let video = source.specs[index].is_video;
        let Some(ts) = packet.pts().or(packet.dts()) else {
            continue;
        };
        let t = seconds(ts, tb, st);
        let t0 = match start {
            Some(t0) => t0,
            // The clip starts at the first video keyframe after the seek.
            None if video && packet.is_key() => *start.insert(t),
            None => continue,
        };
        if video && t > to {
            video_done = true; // decode order: nothing after this may be kept
        }
        if video_done && t > to + 1.0 {
            break; // well past the end on every stream
        }
        if t < t0 || t > to || (video && video_done) {
            continue;
        }
        if video {
            end = end.max(t);
            if !progress(((t - t0) / (to - t0).max(1e-6)).clamp(0.0, 1.0) as f32) {
                return Err(MediaError::Io("trim cancelled".into()));
            }
        }
        let offset = start_ticks(st) + ticks(t0, tb);
        let mut p = packet.clone();
        p.set_pts(p.pts().map(|v| v - offset));
        p.set_dts(p.dts().map(|v| v - offset));
        muxer.write(index, p)?;
    }
    let start = start.ok_or_else(|| MediaError::Io("no video frames in that range".into()))?;
    muxer.finish()?;
    progress(1.0);
    Ok(Trimmed { start, end })
}

/// An audio stream being re-encoded.
struct AudioTrack {
    decoder: ffmpeg::decoder::Audio,
    resampler: Option<ffmpeg::software::resampling::Context>,
    encoder: AudioEncoder,
    clock: (FfRational, i64),
    /// Interleaved stereo samples waiting to fill a block.
    pending: Vec<f32>,
    /// Samples written so far, and the number to write.
    written: usize,
    wanted: usize,
    out: usize,
}

impl AudioTrack {
    /// Adds decoded audio starting at `t` seconds, dropping what is before `from`.
    fn add(&mut self, decoded: &frame::Audio, from: f64) -> Result<(), MediaError> {
        let t = seconds(
            decoded.timestamp().or(decoded.pts()).unwrap_or(0),
            self.clock.0,
            self.clock.1,
        );
        let samples = convert_audio(decoded, &mut self.resampler).map_err(MediaError::Io)?;
        if self.written == 0 && self.pending.is_empty() {
            // The first audio sets the alignment: skip up to the in point, or pad to it.
            let lead = ((from - t) * SAMPLE_RATE as f64).round() as i64;
            if lead > 0 {
                let skip = (lead as usize * 2).min(samples.len());
                self.pending.extend_from_slice(&samples[skip..]);
                return Ok(());
            }
            self.pending.resize((-lead) as usize * 2, 0.0);
        }
        self.pending.extend_from_slice(&samples);
        Ok(())
    }

    fn done(&self) -> bool {
        self.written >= self.wanted
    }

    /// Encodes whole blocks (or everything left, padded, when `flush`).
    fn encode(&mut self, flush: bool, muxer: &mut Muxer) -> Result<(), MediaError> {
        let mut error = None;
        let mut block = Block::SILENT;
        while !self.done()
            && (self.pending.len() >= BLOCK * 2 || (flush && !self.pending.is_empty()))
        {
            let n = (self.pending.len() / 2).min(BLOCK);
            block.clear();
            for i in 0..n {
                block.l[i] = self.pending[i * 2];
                block.r[i] = self.pending[i * 2 + 1];
            }
            self.pending.drain(..n * 2);
            self.written += BLOCK;
            let out = self.out;
            self.encoder.push(&block, &mut |p| {
                if let Err(e) = muxer.write(out, p) {
                    error.get_or_insert(e);
                }
            })?;
        }
        error.map_or(Ok(()), Err)
    }
}

fn precise(
    input: &Path,
    temp: &Path,
    container: Container,
    from: f64,
    to: f64,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Trimmed, MediaError> {
    let mut ictx = ffmpeg::format::input(input)?;
    let video_stream = ictx
        .streams()
        .best(media::Type::Video)
        .ok_or_else(|| MediaError::Io("this file has no video".into()))?;
    let video_index = video_stream.index();
    let params = video_stream.parameters();
    let rate = video_stream.avg_frame_rate();
    let fps = if rate.0 > 0 && rate.1 > 0 {
        Rational::new(rate.0 as u32, rate.1 as u32)
    } else {
        Rational::fps(60)
    };
    let video_clock = (video_stream.time_base(), video_stream.start_time());
    let source_codec = params.id();
    let mut video_decoder = codec::context::Context::from_parameters(params)?
        .decoder()
        .video()?;
    // NV12 needs even sizes.
    let (width, height) = (
        video_decoder.width().max(2) & !1,
        video_decoder.height().max(2) & !1,
    );
    let duration = ictx.duration();
    let file_kbps = match std::fs::metadata(input) {
        Ok(m) if duration > 0 => (m.len() * 8 * 1_000_000 / duration as u64 / 1000) as u32,
        _ => 12_000,
    };
    let settings = RecordingSettings {
        codec: match source_codec {
            codec::Id::HEVC => VideoCodec::Hevc,
            codec::Id::AV1 => VideoCodec::Av1,
            _ => VideoCodec::H264,
        },
        encoder: "auto".into(),
        rate_control: RateControl::Vbr,
        // Match the source so the trimmed clip looks the same; audio is a small share.
        bitrate_kbps: file_kbps.clamp(2_000, 150_000),
        preset: Preset::Quality,
        keyframe_seconds: 2,
        ..Default::default()
    };
    let chosen = encode::choose(&settings)?;
    let mut video_encoder = VideoEncoder::open(chosen.name, width, height, fps, &settings)?;

    let wanted = ((to - from) * SAMPLE_RATE as f64).round() as usize;
    let mut specs = vec![StreamSpec {
        parameters: video_encoder.parameters(),
        time_base: video_encoder.time_base,
        is_video: true,
    }];
    let mut audio: Vec<(usize, AudioTrack)> = Vec::new();
    for stream in ictx.streams() {
        let p = stream.parameters();
        if p.medium() != media::Type::Audio {
            continue;
        }
        let audio_codec = if p.id() == codec::Id::OPUS {
            AudioCodec::Opus
        } else {
            AudioCodec::Aac
        };
        let Ok(decoder) =
            codec::context::Context::from_parameters(p).and_then(|c| c.decoder().audio())
        else {
            continue; // an audio stream we can't decode is left out
        };
        let encoder = AudioEncoder::open(audio_codec, 192)?;
        specs.push(StreamSpec {
            parameters: encoder.parameters(),
            time_base: encoder.time_base,
            is_video: false,
        });
        audio.push((
            stream.index(),
            AudioTrack {
                decoder,
                resampler: None,
                encoder,
                clock: (stream.time_base(), stream.start_time()),
                pending: Vec::new(),
                written: 0,
                wanted,
                out: specs.len() - 1,
            },
        ));
    }

    seek(&mut ictx, from);
    let mut muxer = Muxer::create(temp, container, &specs)?;
    let mut scaler = None;
    let frame_secs = fps.den as f64 / fps.num as f64;
    let mut last_pts = -1;
    let mut first: Option<f64> = None;
    let mut end = from;
    let mut video_done = false;
    let mut write_error = None;
    let mut decoded = frame::Video::empty();
    let mut decoded_audio = frame::Audio::empty();
    let mut packet = ffmpeg::Packet::empty();
    let mut eof = false;

    // Encodes every decoded video frame inside the range.
    let mut drain_video = |decoder: &mut ffmpeg::decoder::Video,
                           encoder: &mut VideoEncoder,
                           muxer: &mut Muxer,
                           video_done: &mut bool,
                           progress: &mut dyn FnMut(f32) -> bool|
     -> Result<(), MediaError> {
        while decoder.receive_frame(&mut decoded).is_ok() {
            let t = seconds(
                decoded.timestamp().or(decoded.pts()).unwrap_or(0),
                video_clock.0,
                video_clock.1,
            );
            if t < from - frame_secs / 2.0 {
                continue; // before the in point
            }
            if t > to - frame_secs / 2.0 {
                *video_done = true;
                continue;
            }
            let pts = ((t - from) / frame_secs).round() as i64;
            if pts <= last_pts {
                continue;
            }
            last_pts = pts;
            first.get_or_insert(t);
            end = t;
            let picture = convert_video(&decoded, &mut scaler).map_err(MediaError::Io)?;
            if picture.layout != PixelLayout::Nv12
                || (picture.width, picture.height) != (width, height)
            {
                return Err(MediaError::Io(
                    "unsupported video format for precise trim".into(),
                ));
            }
            let nv12 = VideoFrame::from_nv12(Timestamp::ZERO, width, height, picture.data);
            encoder.encode(&nv12, pts, false, &mut |p| {
                if let Err(e) = muxer.write(0, p) {
                    write_error.get_or_insert(e);
                }
            })?;
            if !progress(((t - from) / (to - from)).clamp(0.0, 1.0) as f32) {
                return Err(MediaError::Io("trim cancelled".into()));
            }
        }
        Ok(())
    };

    while !(video_done && audio.iter().all(|(_, a)| a.done())) {
        if eof {
            break;
        }
        match packet.read(&mut ictx) {
            Ok(()) => {
                let index = packet.stream();
                if index == video_index {
                    if !video_done {
                        let _ = video_decoder.send_packet(&packet);
                        drain_video(
                            &mut video_decoder,
                            &mut video_encoder,
                            &mut muxer,
                            &mut video_done,
                            progress,
                        )?;
                    }
                } else if let Some((_, track)) = audio.iter_mut().find(|(i, _)| *i == index)
                    && !track.done()
                {
                    let _ = track.decoder.send_packet(&packet);
                    while track.decoder.receive_frame(&mut decoded_audio).is_ok() {
                        let t = seconds(
                            decoded_audio
                                .timestamp()
                                .or(decoded_audio.pts())
                                .unwrap_or(0),
                            track.clock.0,
                            track.clock.1,
                        );
                        let length =
                            decoded_audio.samples() as f64 / f64::from(decoded_audio.rate().max(1));
                        if t + length < from {
                            continue; // wholly before the in point
                        }
                        track.add(&decoded_audio, from)?;
                    }
                    track.encode(false, &mut muxer)?;
                }
            }
            Err(ffmpeg::Error::Eof) => {
                eof = true;
                let _ = video_decoder.send_eof();
                drain_video(
                    &mut video_decoder,
                    &mut video_encoder,
                    &mut muxer,
                    &mut video_done,
                    progress,
                )?;
                for (_, track) in &mut audio {
                    let _ = track.decoder.send_eof();
                    while track.decoder.receive_frame(&mut decoded_audio).is_ok() {
                        track.add(&decoded_audio, from)?;
                    }
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    drop(drain_video);
    if let Some(e) = write_error.take() {
        return Err(e);
    }
    let start = first.ok_or_else(|| MediaError::Io("no video frames in that range".into()))?;

    let mut error = None;
    video_encoder.finish(&mut |p| {
        if let Err(e) = muxer.write(0, p) {
            error.get_or_insert(e);
        }
    })?;
    for (_, track) in &mut audio {
        track.encode(true, &mut muxer)?;
        let out = track.out;
        track.encoder.finish(&mut |p| {
            if let Err(e) = muxer.write(out, p) {
                error.get_or_insert(e);
            }
        })?;
    }
    if let Some(e) = error {
        return Err(e);
    }
    muxer.finish()?;
    progress(1.0);
    Ok(Trimmed { start, end })
}
