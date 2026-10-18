//! Frame-accurate video decoding for editing: "give me the frame at 12.40 s".
//!
//! [`FrameReader`] decodes one file's video to tightly packed NV12. Asking for a time just
//! after the last one decodes forward (the normal playback case); asking for an earlier time,
//! or one far ahead, seeks to the keyframe before it and decodes up to the frame. The frame
//! returned is the last one whose time is at or before the request.

use crate::MediaError;
use crate::ffmpeg::{self, codec, frame, media};
use crate::player::{PixelLayout, convert_video};
use std::path::Path;

/// A decoded picture: NV12, `width × height` luma then interleaved chroma.
#[derive(Clone, Debug, PartialEq)]
pub struct Nv12 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Seconds from the start of the file.
    pub time: f64,
    pub full_range: bool,
}

/// Decoding forward past more than this from the last frame seeks instead.
const SEEK_AHEAD: f64 = 3.0;

pub struct FrameReader {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    index: usize,
    time_base: ffmpeg::Rational,
    start: i64,
    scaler: Option<(
        ffmpeg::software::scaling::Context,
        ffmpeg::format::Pixel,
        u32,
        u32,
        PixelLayout,
    )>,
    /// The newest decoded frame, and one decoded past it (not yet due).
    current: Option<Nv12>,
    ahead: Option<Nv12>,
    eof: bool,
    pub width: u32,
    pub height: u32,
    /// Frames per second (from the stream), for stepping.
    pub fps: f64,
    pub duration: f64,
}

impl FrameReader {
    pub fn open(path: &Path) -> Result<Self, MediaError> {
        crate::init();
        let input = ffmpeg::format::input(path)?;
        let stream = input
            .streams()
            .best(media::Type::Video)
            .ok_or_else(|| MediaError::Io("this file has no video".into()))?;
        let index = stream.index();
        let time_base = stream.time_base();
        let start = match stream.start_time() {
            s if s == ffmpeg::ffi::AV_NOPTS_VALUE => 0,
            s => s,
        };
        let rate = stream.avg_frame_rate();
        let fps = if rate.0 > 0 && rate.1 > 0 {
            rate.0 as f64 / rate.1 as f64
        } else {
            30.0
        };
        let mut ctx = codec::context::Context::from_parameters(stream.parameters())?;
        if let Ok(threads) = std::thread::available_parallelism() {
            ctx.set_threading(codec::threading::Config {
                kind: codec::threading::Type::Frame,
                count: threads.get().min(8),
            });
        }
        let decoder = ctx.decoder().video()?;
        let (width, height) = (decoder.width() & !1, decoder.height() & !1);
        let duration = match input.duration() {
            d if d > 0 => d as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE),
            _ => 0.0,
        };
        Ok(Self {
            input,
            decoder,
            index,
            time_base,
            start,
            scaler: None,
            current: None,
            ahead: None,
            eof: false,
            width,
            height,
            fps,
            duration,
        })
    }

    fn seconds(&self, ts: i64) -> f64 {
        (ts - self.start) as f64 * self.time_base.0 as f64 / self.time_base.1.max(1) as f64
    }

    fn seek(&mut self, t: f64) {
        let ts = (t.max(0.0) * 1e6) as i64;
        let _ = self.input.seek(ts, ..ts);
        self.decoder.flush();
        self.current = None;
        self.ahead = None;
        self.eof = false;
    }

    /// Decodes the next frame, or `None` at the end of the file.
    fn next(&mut self) -> Result<Option<Nv12>, MediaError> {
        let mut decoded = frame::Video::empty();
        let mut packet = ffmpeg::Packet::empty();
        loop {
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                let t = self.seconds(decoded.timestamp().or(decoded.pts()).unwrap_or(0));
                let picture = convert_video(&decoded, &mut self.scaler).map_err(MediaError::Io)?;
                if picture.layout != PixelLayout::Nv12 {
                    return Err(MediaError::Io("unsupported video format".into()));
                }
                return Ok(Some(Nv12 {
                    width: picture.width,
                    height: picture.height,
                    data: picture.data,
                    time: t,
                    full_range: picture.full_range,
                }));
            }
            if self.eof {
                return Ok(None);
            }
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.index => {
                    let _ = self.decoder.send_packet(&packet);
                }
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// The frame showing at `t` seconds (the last one at or before it; the first frame for
    /// earlier times, the last one past the end).
    pub fn frame_at(&mut self, t: f64) -> Result<Option<&Nv12>, MediaError> {
        let half = 0.5 / self.fps.max(1.0);
        let behind = self.current.as_ref().is_some_and(|c| t + half < c.time);
        let far = self
            .current
            .as_ref()
            .is_none_or(|c| t - c.time > SEEK_AHEAD);
        if behind || far {
            self.seek(t);
        }
        loop {
            // The frame after `current` is already decoded: is it due?
            if let Some(next) = &self.ahead {
                if next.time <= t + half {
                    self.current = self.ahead.take();
                    continue;
                }
                break;
            }
            match self.next()? {
                Some(f) if self.current.is_none() || f.time <= t + half => self.current = Some(f),
                Some(f) => {
                    self.ahead = Some(f);
                    break;
                }
                None => break,
            }
        }
        Ok(self.current.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_fail_cleanly() {
        assert!(FrameReader::open(Path::new("C:/no/such/file.mp4")).is_err());
    }
}
