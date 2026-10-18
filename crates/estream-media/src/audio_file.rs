//! Decoding a file's audio to 48 kHz stereo `f32`, for analysis (highlight scans, beat
//! detection) and for mixing montages. Not for real-time playback (see [`crate::player`]).

use crate::MediaError;
use crate::ffmpeg::{self, frame, media};
use crate::player::convert_audio;
use std::path::Path;

pub const RATE: u32 = 48_000;

/// Decodes the first audio stream of `path` from `start` seconds and hands interleaved stereo
/// chunks to `chunk` until the file ends or `chunk` returns false. Returns the seconds
/// decoded.
pub fn for_each_chunk(
    path: &Path,
    start: f64,
    mut chunk: impl FnMut(&[f32]) -> bool,
) -> Result<f64, MediaError> {
    crate::init();
    let mut input = ffmpeg::format::input(path)?;
    let stream = input
        .streams()
        .best(media::Type::Audio)
        .ok_or_else(|| MediaError::Io("this file has no audio".into()))?;
    let index = stream.index();
    let (tb, st) = (stream.time_base(), stream.start_time());
    let st = if st == ffmpeg::ffi::AV_NOPTS_VALUE {
        0
    } else {
        st
    };
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;
    if start > 0.0 {
        let ts = (start * 1e6) as i64;
        let _ = input.seek(ts, ..ts);
    }
    let mut resampler = None;
    let mut decoded = frame::Audio::empty();
    let mut packet = ffmpeg::Packet::empty();
    // Samples (per channel) handed out so far, and the first frame's time.
    let mut frames = 0usize;
    let mut first: Option<f64> = None;
    let mut done = false;
    let mut handle = |decoded: &frame::Audio,
                      resampler: &mut Option<ffmpeg::software::resampling::Context>,
                      first: &mut Option<f64>,
                      frames: &mut usize|
     -> Result<bool, MediaError> {
        let t = decoded.timestamp().or(decoded.pts()).unwrap_or(0);
        let t = (t - st) as f64 * tb.0 as f64 / tb.1.max(1) as f64;
        let samples = convert_audio(decoded, resampler).map_err(MediaError::Io)?;
        let first = *first.get_or_insert(t);
        // After a seek, drop what's before `start`.
        let skip = if first < start && *frames == 0 {
            let lead = ((start - t) * RATE as f64).round().max(0.0) as usize * 2;
            lead.min(samples.len())
        } else {
            0
        };
        let rest = &samples[skip..];
        if rest.is_empty() {
            return Ok(true);
        }
        *frames += rest.len() / 2;
        Ok(chunk(rest))
    };
    while !done {
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == index => {
                let _ = decoder.send_packet(&packet);
                while decoder.receive_frame(&mut decoded).is_ok() {
                    if !handle(&decoded, &mut resampler, &mut first, &mut frames)? {
                        done = true;
                        break;
                    }
                }
            }
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => {
                let _ = decoder.send_eof();
                while decoder.receive_frame(&mut decoded).is_ok() {
                    if !handle(&decoded, &mut resampler, &mut first, &mut frames)? {
                        break;
                    }
                }
                done = true;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(frames as f64 / RATE as f64)
}

/// The whole audio of `path` as interleaved stereo at 48 kHz (for short files such as music).
pub fn decode_stereo(path: &Path) -> Result<Vec<f32>, MediaError> {
    let mut out = Vec::new();
    for_each_chunk(path, 0.0, |c| {
        out.extend_from_slice(c);
        true
    })?;
    Ok(out)
}

/// Stereo → mono (the average of both channels).
pub fn to_mono(stereo: &[f32]) -> Vec<f32> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[l, r]| (l + r) * 0.5)
        .collect()
}
