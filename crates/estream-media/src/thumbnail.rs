//! Clip thumbnails: one frame, scaled down, saved as a JPEG.

use crate::MediaError;
use crate::ffmpeg::{self, codec, format, frame, media, software};
use std::path::Path;

/// Saves the frame at `at` seconds (or the last one before the end) of `input` as a JPEG
/// `width` pixels wide at `output`. Returns the time of the frame used.
pub fn thumbnail(input: &Path, at: f64, width: u32, output: &Path) -> Result<f64, MediaError> {
    crate::init();
    let mut ictx = ffmpeg::format::input(input)?;
    let stream = ictx
        .streams()
        .best(media::Type::Video)
        .ok_or_else(|| MediaError::Io("this file has no video".into()))?;
    let index = stream.index();
    let (tb, start) = (stream.time_base(), stream.start_time());
    let start = if start == ffmpeg::ffi::AV_NOPTS_VALUE {
        0
    } else {
        start
    };
    let mut decoder = codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    if at > 0.0 {
        let ts = (at * 1e6) as i64;
        let _ = ictx.seek(ts, ..ts);
    }

    // Decode up to the first frame at or after `at`, keeping the latest one in case the
    // file ends first.
    let mut picked = frame::Video::empty();
    let mut picked_at = None;
    let mut decoded = frame::Video::empty();
    let mut packet = ffmpeg::Packet::empty();
    let mut take = |decoder: &mut ffmpeg::decoder::Video| -> bool {
        while decoder.receive_frame(&mut decoded).is_ok() {
            let ts = decoded.timestamp().or(decoded.pts()).unwrap_or(0);
            let t = (ts - start) as f64 * tb.0 as f64 / tb.1.max(1) as f64;
            std::mem::swap(&mut picked, &mut decoded);
            picked_at = Some(t);
            if t >= at - 0.001 {
                return true;
            }
        }
        false
    };
    loop {
        match packet.read(&mut ictx) {
            Ok(()) if packet.stream() == index => {
                let _ = decoder.send_packet(&packet);
                if take(&mut decoder) {
                    break;
                }
            }
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => {
                let _ = decoder.send_eof();
                take(&mut decoder);
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }
    let t = picked_at.ok_or_else(|| MediaError::Io("no video frame to use".into()))?;

    let (w, h) = (picked.width(), picked.height());
    let out_w = width.min(w).max(2) & !1;
    let out_h = ((h as u64 * out_w as u64 / w.max(1) as u64) as u32).max(2) & !1;
    let jpeg_format = format::Pixel::YUVJ420P;
    let mut scaler = software::scaling::Context::get(
        picked.format(),
        w,
        h,
        jpeg_format,
        out_w,
        out_h,
        software::scaling::Flags::BICUBIC,
    )?;
    let mut small = frame::Video::empty();
    scaler.run(&picked, &mut small)?;
    small.set_pts(Some(0));

    let jpeg = ffmpeg::encoder::find(codec::Id::MJPEG)
        .ok_or_else(|| MediaError::NoEncoder("no JPEG encoder in this FFmpeg build".into()))?;
    let mut enc = codec::context::Context::new_with_codec(jpeg)
        .encoder()
        .video()?;
    enc.set_width(out_w);
    enc.set_height(out_h);
    enc.set_format(jpeg_format);
    enc.set_time_base(ffmpeg::Rational(1, 25));
    // MJPEG quality: lower is better (2..31).
    enc.set_global_quality(4 * ffmpeg::ffi::FF_QP2LAMBDA);
    enc.set_flags(codec::Flags::QSCALE);
    let mut encoder = enc.open_as(jpeg)?;
    encoder.send_frame(&small)?;
    encoder.send_eof()?;
    let mut out = ffmpeg::Packet::empty();
    encoder.receive_packet(&mut out)?;
    let data = out
        .data()
        .ok_or_else(|| MediaError::Io("the JPEG encoder produced nothing".into()))?;
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(|e| MediaError::Io(e.to_string()))?;
    }
    std::fs::write(output, data).map_err(|e| MediaError::Io(e.to_string()))?;
    Ok(t)
}
