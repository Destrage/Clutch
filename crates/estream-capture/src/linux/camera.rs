//! Video capture devices (webcams, capture cards) through V4L2.
//!
//! A worker thread reads memory-mapped buffers. YUYV and NV12 are repacked to tight NV12 for
//! the GPU's YUV → RGB conversion (with the stream's matrix and range); MJPEG is decoded to
//! RGBA on the worker.

use super::frames::{Frame, FrameSlot, Layout};
use crate::camera::{CameraFormat, best_format};
use crate::media::upload::CpuTexture;
use estream_core::{Size, Timestamp};
use estream_engine::source::parse;
use estream_engine::{
    Property, PropertySchema, SelectOption, SourceContext, SourceError, SourceFrame, SourceStatus,
    SourceType, VideoSource,
};
use estream_gpu::{Nv12Frame, YuvConverter, YuvMatrix, wgpu};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use v4l::buffer::Type;
use v4l::format::{Colorspace, FourCC, Quantization};
use v4l::frameinterval::FrameIntervalEnum;
use v4l::framesize::FrameSizeEnum;
use v4l::io::traits::CaptureStream;
use v4l::prelude::*;
use v4l::video::Capture;
use v4l::video::capture::Parameters;

#[derive(Clone, Debug, PartialEq)]
pub struct CameraDevice {
    /// `/dev/v4l/by-id/…` when there is one (stable across reboots and ports), else the node.
    pub id: String,
    pub name: String,
}

/// V4L2 pixel formats we read, with the names camera formats use on every system.
const FOURCCS: &[(&[u8; 4], &str)] = &[(b"NV12", "NV12"), (b"YUYV", "YUY2"), (b"MJPG", "MJPG")];

fn subtype(fourcc: FourCC) -> Option<&'static str> {
    FOURCCS
        .iter()
        .find(|(code, _)| **code == fourcc.repr)
        .map(|(_, name)| *name)
}

fn fourcc(subtype: &str) -> Option<FourCC> {
    FOURCCS
        .iter()
        .find(|(_, name)| *name == subtype)
        .map(|(code, _)| FourCC::new(code))
}

/// Capture nodes with a format we read (a webcam's metadata node has none).
pub fn devices() -> Vec<CameraDevice> {
    let Ok(dir) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    let by_id: Vec<PathBuf> = std::fs::read_dir("/dev/v4l/by-id")
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let mut nodes: Vec<(u32, CameraDevice)> = dir
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().into_string().ok()?;
            let index: u32 = file.strip_prefix("video")?.parse().ok()?;
            let node = PathBuf::from("/dev").join(&file);
            if formats_of(&Device::with_path(&node).ok()?).is_empty() {
                return None;
            }
            // Names from sysfs: the driver cuts them at 32 bytes, maybe inside a character.
            let name = std::fs::read(e.path().join("name"))
                .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
                .unwrap_or_else(|_| file.clone());
            let id = by_id
                .iter()
                .find(|link| std::fs::canonicalize(link).is_ok_and(|p| p == node))
                .unwrap_or(&node)
                .to_string_lossy()
                .into_owned();
            Some((index, CameraDevice { id, name }))
        })
        .collect();
    nodes.sort_by_key(|(i, _)| *i);
    nodes.into_iter().map(|(_, d)| d).collect()
}

fn formats_of(dev: &Device) -> Vec<CameraFormat> {
    let mut list = Vec::new();
    for desc in dev.enum_formats().unwrap_or_default() {
        let Some(name) = subtype(desc.fourcc) else {
            continue;
        };
        for size in dev.enum_framesizes(desc.fourcc).unwrap_or_default() {
            let sizes: Vec<(u32, u32)> = match size.size {
                FrameSizeEnum::Discrete(d) => vec![(d.width, d.height)],
                FrameSizeEnum::Stepwise(s) => vec![(s.max_width, s.max_height)],
            };
            for (width, height) in sizes {
                for interval in dev
                    .enum_frameintervals(desc.fourcc, width, height)
                    .unwrap_or_default()
                {
                    // An interval is seconds per frame: fps is its inverse.
                    let fraction = match interval.interval {
                        FrameIntervalEnum::Discrete(f) => f,
                        FrameIntervalEnum::Stepwise(s) => s.min,
                    };
                    if fraction.numerator == 0 {
                        continue;
                    }
                    list.push(CameraFormat {
                        width,
                        height,
                        fps_num: fraction.denominator,
                        fps_den: fraction.numerator,
                        subtype: name.into(),
                    });
                }
            }
        }
    }
    list
}

pub fn formats(id: &str) -> Vec<CameraFormat> {
    Device::with_path(id)
        .map(|d| formats_of(&d))
        .unwrap_or_default()
}

// ------------------------------------------------------------------ conversions

/// Repacks YUYV (4:2:2) into tight NV12 (4:2:0), averaging chroma over each row pair. Odd
/// widths and heights drop their last column or row. `false` if `src` is too short.
pub fn yuyv_to_nv12(src: &[u8], width: u32, height: u32, stride: usize, out: &mut Vec<u8>) -> bool {
    let (w, h) = ((width & !1) as usize, (height & !1) as usize);
    if w == 0 || h == 0 || stride < w * 2 || src.len() < stride * (h - 1) + w * 2 {
        return false;
    }
    out.resize(w * h * 3 / 2, 0);
    let (y, uv) = out.split_at_mut(w * h);
    for row in (0..h).step_by(2) {
        let a = &src[row * stride..row * stride + w * 2];
        let b = &src[(row + 1) * stride..(row + 1) * stride + w * 2];
        for (x, (pa, pb)) in a
            .as_chunks::<2>()
            .0
            .iter()
            .zip(b.as_chunks::<2>().0)
            .enumerate()
        {
            y[row * w + x] = pa[0];
            y[(row + 1) * w + x] = pb[0];
        }
        let avg = |p: u8, q: u8| (p as u16 + q as u16).div_ceil(2) as u8;
        let chroma = &mut uv[row / 2 * w..row / 2 * w + w];
        for (dst, (qa, qb)) in chroma
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0))
        {
            *dst = [avg(qa[1], qb[1]), avg(qa[3], qb[3])];
        }
    }
    true
}

/// Copies NV12 with row padding into tight NV12.
pub fn nv12_tight(src: &[u8], width: u32, height: u32, stride: usize, out: &mut Vec<u8>) -> bool {
    let (w, h) = ((width & !1) as usize, (height & !1) as usize);
    let rows = h + h / 2;
    // Planes are contiguous: the CbCr rows follow the luma rows at the same stride.
    if w == 0 || stride < w || src.len() < stride * (rows - 1) + w {
        return false;
    }
    out.resize(w * rows, 0);
    for (dst, src) in out.chunks_exact_mut(w).zip(src.chunks(stride)) {
        dst.copy_from_slice(&src[..w]);
    }
    true
}

/// The YUV matrix and range a V4L2 format declares (V4L2's defaults when it doesn't say).
fn yuv_params(format: &v4l::Format) -> (YuvMatrix, bool) {
    let matrix = match format.colorspace {
        Colorspace::Rec709 => YuvMatrix::Bt709,
        Colorspace::SMPTE170M | Colorspace::SRGB | Colorspace::JPEG | Colorspace::NTSC => {
            YuvMatrix::Bt601
        }
        _ => YuvMatrix::for_height(format.height),
    };
    let full = match format.quantization {
        Quantization::FullRange => true,
        Quantization::LimitedRange => false,
        _ => matches!(format.colorspace, Colorspace::JPEG),
    };
    (matrix, full)
}

// ------------------------------------------------------------------ worker

struct Worker {
    frames: Arc<FrameSlot>,
    status: Arc<Mutex<Option<SourceStatus>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // The thread notices within one frame (or the read timeout) and closes the device.
        self.stop.store(true, Ordering::Release);
    }
}

impl Worker {
    fn spawn(device: String, format: String) -> Self {
        let worker = Self {
            frames: Arc::default(),
            status: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (frames, status, stop) = (
            worker.frames.clone(),
            worker.status.clone(),
            worker.stop.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("estream-camera".into())
            .spawn(move || {
                let reason = match read(&device, &format, &frames, &stop) {
                    Ok(()) => return,
                    Err(e) => e,
                };
                tracing::warn!(%device, %reason, "camera stopped");
                *status.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(SourceStatus::Unavailable(reason));
            });
        if let Err(e) = spawned {
            *worker.status.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(SourceStatus::Error(e.to_string()));
        }
        worker
    }

    /// Why the worker stopped, once it has.
    fn stopped(&self) -> Option<SourceStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

const EBUSY: i32 = 16;
const IN_USE: &str = "The camera is in use by another app";

fn read(device: &str, key: &str, frames: &FrameSlot, stop: &AtomicBool) -> Result<(), String> {
    let dev = Device::with_path(device).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "Camera disconnected".to_owned(),
        std::io::ErrorKind::PermissionDenied => {
            "No permission to open the camera (are you in the “video” group?)".to_owned()
        }
        _ => format!("Couldn't open the camera: {e}"),
    })?;
    let list = formats_of(&dev);
    let chosen = list
        .iter()
        .find(|f| f.key() == key)
        .cloned()
        .or_else(|| best_format(&list))
        .ok_or("This camera has no format Clutch can read (NV12, YUYV or MJPEG)")?;
    let code = fourcc(&chosen.subtype).ok_or("unsupported format")?;
    let mut format = dev.format().map_err(|e| e.to_string())?;
    (format.width, format.height, format.fourcc) = (chosen.width, chosen.height, code);
    let format = dev
        .set_format(&format)
        .map_err(|e| match e.raw_os_error() {
            Some(EBUSY) => IN_USE.to_owned(),
            _ => format!("Couldn't set the camera format: {e}"),
        })?;
    let _ = dev.set_params(&Parameters::new(v4l::Fraction::new(
        chosen.fps_den,
        chosen.fps_num,
    )));
    let (matrix, full_range) = yuv_params(&format);
    tracing::info!(%device, format = %chosen.key(), ?matrix, full_range, "camera started");
    let mut stream = MmapStream::with_buffers(&dev, Type::VideoCapture, 4).map_err(|e| match e
        .raw_os_error()
    {
        Some(EBUSY) => IN_USE.to_owned(),
        _ => format!("Couldn't start the camera: {e}"),
    })?;
    stream.set_timeout(Duration::from_millis(500));
    let mut frame = Frame::default();
    let mut jpeg = Vec::new();
    let stride = format.stride as usize;
    let mut silent_since = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let (buf, meta) = match stream.next() {
            Ok(next) => next,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if silent_since.elapsed() > Duration::from_secs(5) {
                    return Err("The camera stopped sending pictures".into());
                }
                continue;
            }
            Err(e) => return Err(format!("Camera disconnected ({e})")),
        };
        silent_since = Instant::now();
        let data = &buf[..(meta.bytesused as usize).min(buf.len())];
        let ok = match chosen.subtype.as_str() {
            "YUY2" => yuyv_to_nv12(data, format.width, format.height, stride, &mut frame.data),
            "NV12" => nv12_tight(data, format.width, format.height, stride, &mut frame.data),
            _ => match image::load_from_memory_with_format(
                super::mjpeg::with_huffman_tables(data, &mut jpeg),
                image::ImageFormat::Jpeg,
            ) {
                Ok(img) => {
                    frame.data = img.to_rgba8().into_raw();
                    frame.width = img.width();
                    frame.height = img.height();
                    frame.layout = Layout::Packed(wgpu::TextureFormat::Rgba8UnormSrgb);
                    frames.publish(&mut frame);
                    continue;
                }
                // Cameras send the odd broken JPEG: skip it.
                Err(_) => false,
            },
        };
        if ok {
            frame.width = format.width & !1;
            frame.height = format.height & !1;
            frame.layout = Layout::Nv12 { matrix, full_range };
            frames.publish(&mut frame);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ source

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct CameraSettings {
    device: String,
    /// Format key (see [`CameraFormat::key`]); empty = automatic.
    format: String,
}

/// The texture the latest frame went to.
enum Output {
    None,
    Nv12(Box<(Nv12Frame, wgpu::BindGroup)>),
    Packed(CpuTexture),
}

pub struct CameraSource {
    settings: CameraSettings,
    worker: Option<Worker>,
    converter: Arc<YuvConverter>,
    output: Output,
    frame: Frame,
    last: u64,
    status: SourceStatus,
    next_attempt: Instant,
}

impl CameraSource {
    fn new(ctx: &SourceContext<'_>) -> Self {
        Self {
            settings: CameraSettings::default(),
            worker: None,
            converter: YuvConverter::new(ctx.gpu),
            output: Output::None,
            frame: Frame::default(),
            last: 0,
            status: SourceStatus::Unavailable("Starting camera…".into()),
            next_attempt: Instant::now(),
        }
    }
}

impl VideoSource for CameraSource {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let new: CameraSettings = parse(settings)?;
        if new != self.settings {
            self.worker = None;
            self.next_attempt = Instant::now();
            self.settings = new;
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, encoder: &mut wgpu::CommandEncoder, _: Timestamp) {
        if self.settings.device.is_empty() {
            self.status = SourceStatus::Unavailable("Choose a camera".into());
            return;
        }
        if self.worker.is_none() && Instant::now() >= self.next_attempt {
            self.worker = Some(Worker::spawn(
                self.settings.device.clone(),
                self.settings.format.clone(),
            ));
        }
        let Some(worker) = &self.worker else { return };
        if let Some(status) = worker.stopped() {
            self.worker = None;
            self.output = Output::None;
            self.next_attempt = Instant::now() + Duration::from_secs(2);
            self.status = status;
            return;
        }
        if !worker.frames.take(&mut self.last, &mut self.frame) {
            return;
        }
        let f = &self.frame;
        match f.layout {
            Layout::Nv12 { matrix, full_range } => {
                let reuse = matches!(&self.output, Output::Nv12(n) if n.0.width == f.width && n.0.height == f.height);
                if !reuse {
                    let nv12 = self.converter.create_frame(ctx.gpu, f.width, f.height);
                    let binding = ctx.compositor.create_source_binding(
                        ctx.gpu,
                        &nv12.output.create_view(&Default::default()),
                    );
                    self.output = Output::Nv12(Box::new((nv12, binding)));
                }
                if let Output::Nv12(n) = &self.output {
                    let nv12 = &n.0;
                    nv12.upload(ctx.gpu, &f.data, f.width, matrix, full_range);
                    nv12.convert(&self.converter, encoder);
                }
            }
            Layout::Packed(format) => {
                if !matches!(self.output, Output::Packed(_)) {
                    self.output = Output::Packed(CpuTexture::default());
                }
                if let Output::Packed(texture) = &mut self.output {
                    texture.upload_raw(ctx, f.width, f.height, format, &f.data, f.width * 4);
                }
            }
        }
        self.status = SourceStatus::Active;
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        match &self.output {
            Output::None => None,
            Output::Nv12(n) => Some(SourceFrame {
                binding: &n.1,
                size: Size::new(n.0.width as f32, n.0.height as f32),
            }),
            Output::Packed(texture) => texture.frame(),
        }
    }

    fn status(&self) -> SourceStatus {
        self.status.clone()
    }
}

pub fn source_type() -> SourceType {
    SourceType::new(
        "camera",
        "Video capture device",
        "Webcams and capture cards",
        |settings| {
            let devices = devices();
            let mut device_options: Vec<SelectOption> = devices
                .iter()
                .map(|d| SelectOption::new(d.id.clone(), d.name.clone()))
                .collect();
            let current = settings
                .get("device")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !current.is_empty() && !devices.iter().any(|d| d.id == current) {
                device_options.insert(0, SelectOption::new(current.clone(), "Disconnected camera"));
            }
            let device = if current.is_empty() {
                devices.first().map(|d| d.id.clone()).unwrap_or_default()
            } else {
                current
            };
            let mut format_options = vec![SelectOption::new(
                "",
                "Automatic (best quality up to 1080p60)",
            )];
            if Path::new(&device).exists() {
                format_options.extend(
                    formats(&device)
                        .iter()
                        .map(|f| SelectOption::new(f.key(), f.label())),
                );
            }
            PropertySchema::new(vec![
                Property::select("device", "Device", device_options, device),
                Property::select("format", "Resolution / frame rate", format_options, ""),
            ])
        },
        |ctx, settings| {
            let mut s = CameraSource::new(ctx);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    )
    .with_validation(|_| {
        PropertySchema::new(vec![
            Property::select("device", "Device", vec![], ""),
            Property::select("format", "Resolution / frame rate", vec![], ""),
        ])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuyv_becomes_nv12_with_averaged_chroma() {
        // 2×2 YUYV: [Y0 U Y1 V] per row, rows 6 bytes apart (2 bytes of padding).
        let src = [10, 100, 20, 200, 0, 0, 30, 110, 40, 210, 0, 0];
        let mut out = Vec::new();
        assert!(yuyv_to_nv12(&src, 2, 2, 6, &mut out));
        assert_eq!(out, [10, 20, 30, 40, 105, 205]);
        assert!(!yuyv_to_nv12(&src[..9], 2, 2, 6, &mut out), "too short");
    }

    #[test]
    fn nv12_padding_is_removed() {
        // 2×2 NV12, stride 3: two Y rows, one CbCr row.
        let src = [1, 2, 0, 3, 4, 0, 5, 6, 0];
        let mut out = Vec::new();
        assert!(nv12_tight(&src, 2, 2, 3, &mut out));
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn subtypes_match_other_systems() {
        assert_eq!(subtype(FourCC::new(b"YUYV")), Some("YUY2"));
        assert_eq!(fourcc("MJPG"), Some(FourCC::new(b"MJPG")));
        assert_eq!(subtype(FourCC::new(b"H264")), None);
    }

    #[test]
    fn enumeration_does_not_crash() {
        for d in devices() {
            assert!(!d.id.is_empty());
        }
    }
}
