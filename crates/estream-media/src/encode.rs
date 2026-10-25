//! Video and audio encoders (FFmpeg), with hardware encoder probing.

use crate::MediaError;
use crate::ffmpeg::{self, Dictionary, Rational as FfRational, codec, format};
use crate::settings::{AudioCodec, Preset, RateControl, RecordingSettings, VideoCodec};
use estream_audio::{BLOCK, Block, SAMPLE_RATE};
use estream_core::Rational;
use estream_engine::VideoFrame;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, PartialEq)]
pub struct EncoderInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub codec: VideoCodec,
    pub hardware: bool,
}

/// Candidate encoders per codec, best first.
const CANDIDATES: &[(&str, &str, VideoCodec, bool)] = &[
    ("h264_nvenc", "NVIDIA NVENC H.264", VideoCodec::H264, true),
    ("h264_amf", "AMD AMF H.264", VideoCodec::H264, true),
    ("h264_qsv", "Intel QuickSync H.264", VideoCodec::H264, true),
    (
        "libopenh264",
        "Software H.264 (OpenH264)",
        VideoCodec::H264,
        false,
    ),
    ("hevc_nvenc", "NVIDIA NVENC HEVC", VideoCodec::Hevc, true),
    ("hevc_amf", "AMD AMF HEVC", VideoCodec::Hevc, true),
    ("hevc_qsv", "Intel QuickSync HEVC", VideoCodec::Hevc, true),
    (
        "libkvazaar",
        "Software HEVC (Kvazaar)",
        VideoCodec::Hevc,
        false,
    ),
    ("av1_nvenc", "NVIDIA NVENC AV1", VideoCodec::Av1, true),
    ("av1_amf", "AMD AMF AV1", VideoCodec::Av1, true),
    ("av1_qsv", "Intel QuickSync AV1", VideoCodec::Av1, true),
    (
        "libsvtav1",
        "Software AV1 (SVT-AV1)",
        VideoCodec::Av1,
        false,
    ),
];

/// Encoders that actually open on this machine (probed once, then cached).
pub fn available(codec: VideoCodec) -> Vec<EncoderInfo> {
    static CACHE: OnceLock<Mutex<HashMap<VideoCodec, Vec<EncoderInfo>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(found) = cache.lock().ok().and_then(|c| c.get(&codec).cloned()) {
        return found;
    }
    crate::init();
    let probe = RecordingSettings {
        codec,
        ..Default::default()
    };
    // Missing drivers (e.g. AMF without an AMD GPU) are expected here; keep the log quiet.
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Quiet);
    let found: Vec<EncoderInfo> = CANDIDATES
        .iter()
        .filter(|(_, _, c, _)| *c == codec)
        .filter(|(name, ..)| VideoEncoder::open(name, 1280, 720, Rational::fps(30), &probe).is_ok())
        .map(|&(name, label, codec, hardware)| EncoderInfo {
            name,
            label,
            codec,
            hardware,
        })
        .collect();
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    tracing::info!(?codec, encoders = ?found.iter().map(|e| e.name).collect::<Vec<_>>(), "video encoders probed");
    if let Ok(mut c) = cache.lock() {
        c.insert(codec, found.clone());
    }
    found
}

/// Resolves `"auto"` (best available) or a named encoder.
pub fn choose(settings: &RecordingSettings) -> Result<EncoderInfo, MediaError> {
    let list = available(settings.codec);
    if settings.encoder != "auto"
        && let Some(e) = list.iter().find(|e| e.name == settings.encoder)
    {
        return Ok(e.clone());
    }
    list.into_iter().next().ok_or_else(|| {
        MediaError::NoEncoder(format!(
            "no {} encoder could be opened",
            settings.codec.label()
        ))
    })
}

fn family(name: &str) -> &str {
    if name.contains("nvenc") {
        "nvenc"
    } else if name.contains("amf") {
        "amf"
    } else if name.contains("qsv") {
        "qsv"
    } else {
        name
    }
}

fn options(name: &str, settings: &RecordingSettings) -> Dictionary<'static> {
    let mut d = Dictionary::new();
    let q = settings.quality.clamp(1, 51).to_string();
    // Forced keyframes must be IDR so a reconnecting viewer can start decoding there.
    match family(name) {
        "nvenc" => d.set("forced-idr", "1"),
        "amf" | "qsv" => d.set("forced_idr", "1"),
        _ => {}
    }
    match family(name) {
        "nvenc" => {
            d.set(
                "preset",
                match settings.preset {
                    Preset::Performance => "p2",
                    Preset::Balanced => "p5",
                    Preset::Quality => "p7",
                },
            );
            d.set("tune", "hq");
            match settings.rate_control {
                RateControl::Cbr => d.set("rc", "cbr"),
                RateControl::Vbr => d.set("rc", "vbr"),
                RateControl::ConstantQuality => {
                    d.set("rc", "vbr");
                    d.set("cq", &q);
                }
            }
        }
        "amf" => {
            d.set(
                "quality",
                match settings.preset {
                    Preset::Performance => "speed",
                    Preset::Balanced => "balanced",
                    Preset::Quality => "quality",
                },
            );
            match settings.rate_control {
                RateControl::Cbr => d.set("rc", "cbr"),
                RateControl::Vbr => d.set("rc", "vbr_peak"),
                RateControl::ConstantQuality => {
                    d.set("rc", "cqp");
                    d.set("qp_i", &q);
                    d.set("qp_p", &q);
                }
            }
        }
        "qsv" => {
            d.set(
                "preset",
                match settings.preset {
                    Preset::Performance => "veryfast",
                    Preset::Balanced => "medium",
                    Preset::Quality => "slower",
                },
            );
            if settings.rate_control == RateControl::ConstantQuality {
                d.set("global_quality", &q);
            }
        }
        "libsvtav1" => {
            d.set(
                "preset",
                match settings.preset {
                    Preset::Performance => "10",
                    Preset::Balanced => "8",
                    Preset::Quality => "6",
                },
            );
            if settings.rate_control == RateControl::ConstantQuality {
                d.set("crf", &(settings.quality + 8).min(63).to_string());
            }
        }
        _ => {}
    }
    d
}

/// Software encoders here take planar YUV 4:2:0; hardware ones take NV12 directly.
fn takes_nv12(name: &str) -> bool {
    matches!(family(name), "nvenc" | "amf" | "qsv")
}

/// Encoders that can take frames on the GPU (D3D11 textures) on this platform.
pub fn takes_gpu_frames(name: &str) -> bool {
    cfg!(windows) && matches!(family(name), "nvenc" | "amf")
}

#[cfg(windows)]
type GpuInput = crate::gpu_frames::GpuInput;
/// No GPU frame input off Windows.
#[cfg(not(windows))]
enum GpuInput {}

pub struct VideoEncoder {
    encoder: ffmpeg::encoder::Video,
    frame: ffmpeg::frame::Video,
    nv12: bool,
    /// Set when the encoder takes GPU frames.
    gpu: Option<GpuInput>,
    pub name: String,
    pub time_base: FfRational,
}

impl VideoEncoder {
    pub fn open(
        name: &str,
        width: u32,
        height: u32,
        fps: Rational,
        settings: &RecordingSettings,
    ) -> Result<Self, MediaError> {
        Self::open_with(name, width, height, fps, settings, None)
    }

    /// Opens a hardware encoder that takes GPU frames like `first`.
    #[cfg(windows)]
    pub fn open_gpu(
        name: &str,
        width: u32,
        height: u32,
        fps: Rational,
        settings: &RecordingSettings,
        first: &estream_engine::GpuFrame,
    ) -> Result<Self, MediaError> {
        if !takes_gpu_frames(name) {
            return Err(MediaError::NoEncoder(format!(
                "{name} can't take GPU frames"
            )));
        }
        let input = GpuInput::new(first, width, height)?;
        Self::open_with(name, width, height, fps, settings, Some(input))
    }

    /// Whether this encoder takes GPU frames.
    pub fn is_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    fn open_with(
        name: &str,
        width: u32,
        height: u32,
        fps: Rational,
        settings: &RecordingSettings,
        gpu: Option<GpuInput>,
    ) -> Result<Self, MediaError> {
        crate::init();
        let codec = ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| MediaError::NoEncoder(format!("{name} is not in this FFmpeg build")))?;
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        let nv12 = takes_nv12(name);
        let pixel = if nv12 {
            format::Pixel::NV12
        } else {
            format::Pixel::YUV420P
        };
        let time_base = FfRational(fps.den as i32, fps.num as i32);
        enc.set_width(width);
        enc.set_height(height);
        enc.set_format(pixel);
        #[cfg(windows)]
        if let Some(input) = &gpu {
            enc.set_format(format::Pixel::D3D11);
            // SAFETY: the context is not open yet; it takes ownership of the new reference.
            unsafe {
                (*enc.as_mut_ptr()).hw_frames_ctx = input.frames_ref();
            }
        }
        enc.set_time_base(time_base);
        enc.set_frame_rate(Some(FfRational(fps.num as i32, fps.den as i32)));
        enc.set_gop((fps.as_f64() * settings.keyframe_seconds.max(1) as f64).round() as u32);
        enc.set_max_b_frames(0);
        if settings.rate_control != RateControl::ConstantQuality || !takes_nv12(name) {
            enc.set_bit_rate(settings.bitrate_kbps as usize * 1000);
        }
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
        let encoder = enc.open_as_with(codec, options(name, settings))?;
        Ok(Self {
            encoder,
            frame: ffmpeg::frame::Video::new(pixel, width, height),
            nv12,
            gpu,
            name: name.to_owned(),
            time_base,
        })
    }

    pub fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.encoder)
    }

    /// Encodes one NV12 frame at `pts` (in frames); finished packets go to `out`.
    /// `force_key` makes this frame a keyframe (IDR), e.g. when a stream reconnects.
    pub fn encode(
        &mut self,
        frame: &VideoFrame,
        pts: i64,
        force_key: bool,
        out: &mut dyn FnMut(ffmpeg::Packet),
    ) -> Result<(), MediaError> {
        let kind = if force_key {
            ffmpeg::picture::Type::I
        } else {
            ffmpeg::picture::Type::None
        };
        match (&frame.gpu, self.gpu.as_mut()) {
            #[cfg(windows)]
            (Some(gpu_frame), Some(input)) => {
                let mut hw = input.upload(gpu_frame)?;
                hw.set_pts(Some(pts));
                hw.set_kind(kind);
                self.encoder.send_frame(&hw)?;
                return self.drain(out);
            }
            // GPU frames still queued after an output switched to CPU frames: skip them.
            (Some(_), _) => return Ok(()),
            // The engine stopped handing out GPU frames; fail loudly rather than freeze.
            (None, Some(_)) => {
                return Err(MediaError::Engine(
                    "the encoder takes GPU frames but got a CPU frame".into(),
                ));
            }
            (None, None) => {}
        }
        let (w, h) = (frame.width as usize, frame.height as usize);
        let copy_rows = |dst: &mut [u8], stride: usize, src: &[u8], row: usize, rows: usize| {
            for r in 0..rows {
                dst[r * stride..r * stride + row].copy_from_slice(&src[r * row..(r + 1) * row]);
            }
        };
        let y_stride = self.frame.stride(0);
        copy_rows(self.frame.data_mut(0), y_stride, frame.y(), w, h);
        if self.nv12 {
            let uv_stride = self.frame.stride(1);
            copy_rows(self.frame.data_mut(1), uv_stride, frame.uv(), w, h / 2);
        } else {
            // De-interleave CbCr into the U and V planes.
            let (su, sv) = (self.frame.stride(1), self.frame.stride(2));
            let uv = frame.uv();
            for r in 0..h / 2 {
                for c in 0..w / 2 {
                    let i = r * w + c * 2;
                    self.frame.data_mut(1)[r * su + c] = uv[i];
                    self.frame.data_mut(2)[r * sv + c] = uv[i + 1];
                }
            }
        }
        self.frame.set_pts(Some(pts));
        self.frame.set_kind(kind);
        self.encoder.send_frame(&self.frame)?;
        self.drain(out)
    }

    fn drain(&mut self, out: &mut dyn FnMut(ffmpeg::Packet)) -> Result<(), MediaError> {
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            out(std::mem::replace(&mut packet, ffmpeg::Packet::empty()));
        }
        Ok(())
    }

    pub fn finish(&mut self, out: &mut dyn FnMut(ffmpeg::Packet)) -> Result<(), MediaError> {
        self.encoder.send_eof()?;
        self.drain(out)
    }
}

pub struct AudioEncoder {
    encoder: ffmpeg::encoder::Audio,
    frame: ffmpeg::frame::Audio,
    frame_size: usize,
    planar: bool,
    left: Vec<f32>,
    right: Vec<f32>,
    pts: i64,
    pub time_base: FfRational,
}

impl AudioEncoder {
    pub fn open(codec: AudioCodec, bitrate_kbps: u32) -> Result<Self, MediaError> {
        crate::init();
        let (name, sample, planar) = match codec {
            AudioCodec::Aac => (
                "aac",
                format::Sample::F32(format::sample::Type::Planar),
                true,
            ),
            AudioCodec::Opus => (
                "libopus",
                format::Sample::F32(format::sample::Type::Packed),
                false,
            ),
        };
        let found = ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| MediaError::NoEncoder(format!("{name} is not in this FFmpeg build")))?;
        let mut enc = codec::context::Context::new_with_codec(found)
            .encoder()
            .audio()?;
        let layout = ffmpeg::ChannelLayout::default(2);
        let time_base = FfRational(1, SAMPLE_RATE as i32);
        enc.set_rate(SAMPLE_RATE as i32);
        enc.set_channel_layout(layout);
        enc.set_format(sample);
        enc.set_bit_rate(bitrate_kbps as usize * 1000);
        enc.set_time_base(time_base);
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
        let encoder = enc.open_as_with(found, Dictionary::new())?;
        let frame_size = (encoder.frame_size() as usize).max(BLOCK);
        let mut frame = ffmpeg::frame::Audio::new(sample, frame_size, layout);
        frame.set_rate(SAMPLE_RATE);
        Ok(Self {
            encoder,
            frame,
            frame_size,
            planar,
            left: Vec::with_capacity(frame_size * 2),
            right: Vec::with_capacity(frame_size * 2),
            pts: 0,
            time_base,
        })
    }

    pub fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.encoder)
    }

    /// Sets the sample position of the next audio (relative to the output start).
    pub fn set_start(&mut self, samples: i64) {
        self.pts = samples;
    }

    /// The sample position the next pushed audio will get.
    pub fn next_sample(&self) -> i64 {
        self.pts + self.left.len() as i64
    }

    /// Fills `samples` of silence (audio that never arrived), keeping later audio in place.
    pub fn pad_silence(
        &mut self,
        samples: usize,
        out: &mut dyn FnMut(ffmpeg::Packet),
    ) -> Result<(), MediaError> {
        self.left.resize(self.left.len() + samples, 0.0);
        self.right.resize(self.right.len() + samples, 0.0);
        self.flush(out)
    }

    pub fn push(
        &mut self,
        block: &Block,
        out: &mut dyn FnMut(ffmpeg::Packet),
    ) -> Result<(), MediaError> {
        self.left.extend_from_slice(&block.l);
        self.right.extend_from_slice(&block.r);
        self.flush(out)
    }

    /// Encodes every whole frame that is buffered.
    fn flush(&mut self, out: &mut dyn FnMut(ffmpeg::Packet)) -> Result<(), MediaError> {
        while self.left.len() >= self.frame_size {
            let n = self.frame_size;
            if self.planar {
                self.frame.plane_mut::<f32>(0)[..n].copy_from_slice(&self.left[..n]);
                self.frame.plane_mut::<f32>(1)[..n].copy_from_slice(&self.right[..n]);
            } else {
                let data = self.frame.plane_mut::<(f32, f32)>(0);
                for (d, (&l, &r)) in data[..n].iter_mut().zip(self.left.iter().zip(&self.right)) {
                    *d = (l, r);
                }
            }
            self.left.drain(..n);
            self.right.drain(..n);
            self.frame.set_pts(Some(self.pts));
            self.pts += n as i64;
            self.encoder.send_frame(&self.frame)?;
            let mut packet = ffmpeg::Packet::empty();
            while self.encoder.receive_packet(&mut packet).is_ok() {
                out(std::mem::replace(&mut packet, ffmpeg::Packet::empty()));
            }
        }
        Ok(())
    }

    pub fn finish(&mut self, out: &mut dyn FnMut(ffmpeg::Packet)) -> Result<(), MediaError> {
        self.encoder.send_eof()?;
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            out(std::mem::replace(&mut packet, ffmpeg::Packet::empty()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use estream_core::Timestamp;

    /// Silence for missing audio moves the audio after it to the right time.
    #[test]
    fn padded_silence_keeps_later_audio_in_place() {
        let mut enc = AudioEncoder::open(AudioCodec::Aac, 128).unwrap();
        let mut last_pts = 0;
        let mut out = |p: ffmpeg::Packet| last_pts = last_pts.max(p.pts().unwrap_or(0));
        let block = Block::SILENT;
        for _ in 0..10 {
            enc.push(&block, &mut out).unwrap();
        }
        enc.pad_silence(SAMPLE_RATE as usize / 10, &mut out)
            .unwrap(); // 100 ms missing
        for _ in 0..10 {
            enc.push(&block, &mut out).unwrap();
        }
        assert_eq!(enc.next_sample(), 3 * 4_800);
        enc.finish(&mut out).unwrap();
        // The last packet ends at the full length, give or take the AAC frame and priming.
        let frame = 1_024;
        assert!(
            (3 * 4_800 - 2 * frame..=3 * 4_800 + frame).contains(&last_pts),
            "last audio packet at {last_pts}"
        );
    }

    /// Every probed encoder honours a forced keyframe in the middle of a GOP.
    #[test]
    fn forced_keyframes_are_idr() {
        let settings = RecordingSettings {
            rate_control: RateControl::Cbr,
            bitrate_kbps: 2000,
            keyframe_seconds: 4,
            ..Default::default()
        };
        for codec in VideoCodec::ALL {
            for info in available(codec) {
                let mut enc =
                    match VideoEncoder::open(info.name, 320, 192, Rational::fps(30), &settings) {
                        Ok(e) => e,
                        Err(e) => panic!("{} failed to open at 320x192: {e}", info.name),
                    };

                let mut keys = Vec::new();
                let mut n = 0;
                for i in 0..20i64 {
                    let data = vec![(i * 10 % 255) as u8; 320 * 192 * 3 / 2];
                    let frame = VideoFrame::from_nv12(Timestamp(i as u64), 320, 192, data);
                    enc.encode(&frame, i, i == 10, &mut |p| {
                        if p.is_key() {
                            keys.push(n);
                        }
                        n += 1;
                    })
                    .unwrap();
                }
                enc.finish(&mut |p| {
                    if p.is_key() {
                        keys.push(n);
                    }
                    n += 1;
                })
                .unwrap();
                eprintln!("{}: keyframes at {keys:?} of {n}", info.name);
                // FFmpeg's Kvazaar wrapper ignores forced keyframes; streams then wait for
                // the next regular keyframe instead.
                if info.name != "libkvazaar" {
                    assert!(keys.contains(&10), "{}: {keys:?}", info.name);
                }
            }
        }
    }
}
