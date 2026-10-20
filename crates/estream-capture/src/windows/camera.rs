//! Video capture devices (webcams, capture cards) via Media Foundation.
//!
//! A worker thread reads frames with an `IMFSourceReader` (which decodes MJPEG /
//! converts YUY2 to NV12 for us); the video thread uploads the NV12 planes and
//! converts them to RGB on the GPU with the stream's color matrix and range.

pub use crate::camera::{CameraFormat, best_format};
use estream_core::{Size, Timestamp};
use estream_engine::source::parse;
use estream_engine::{
    Property, PropertySchema, SelectOption, SourceContext, SourceError, SourceFrame, SourceStatus,
    SourceType, VideoSource,
};
use estream_gpu::{Nv12Frame, YuvConverter, YuvMatrix, wgpu};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{GUID, Interface, PWSTR};

fn startup() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| {
        // SAFETY: global MF initialization, done once per process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).is_ok() }
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraDevice {
    /// Symbolic link (stable id).
    pub id: String,
    pub name: String,
}

fn subtype_name(g: GUID) -> String {
    match g {
        x if x == MFVideoFormat_NV12 => "NV12".into(),
        x if x == MFVideoFormat_MJPG => "MJPG".into(),
        x if x == MFVideoFormat_YUY2 => "YUY2".into(),
        x if x == MFVideoFormat_RGB32 => "RGB32".into(),
        x if x == MFVideoFormat_H264 => "H264".into(),
        x if x == MFVideoFormat_I420 => "I420".into(),
        other => {
            // FourCC types store the code in the first 4 bytes.
            let b = other.data1.to_le_bytes();
            if b.iter().all(|c| c.is_ascii_alphanumeric()) {
                String::from_utf8_lossy(&b).into_owned()
            } else {
                format!("{other:?}")
            }
        }
    }
}

/// Lists connected video capture devices.
pub fn devices() -> Vec<CameraDevice> {
    if !startup() {
        return Vec::new();
    }
    // SAFETY: COM calls with checked results; the activate array and strings
    // are freed with CoTaskMemFree.
    unsafe {
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_err() {
            return Vec::new();
        }
        let Some(attrs) = attrs else {
            return Vec::new();
        };
        if attrs
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .is_err()
        {
            return Vec::new();
        }
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        if MFEnumDeviceSources(&attrs, &mut list, &mut count).is_err() || list.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..count as usize {
            let Some(activate) = (*list.add(i)).take() else {
                continue;
            };
            let name = get_string(&activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
            let id = get_string(
                &activate,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            );
            if let (Some(name), Some(id)) = (name, id) {
                out.push(CameraDevice { id, name });
            }
        }
        CoTaskMemFree(Some(list as *const _));
        out
    }
}

unsafe fn get_string(attrs: &IMFActivate, key: &GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: MF allocates the string; we free it after copying.
    unsafe {
        attrs.GetAllocatedString(key, &mut value, &mut len).ok()?;
        let s = value.to_string().ok();
        CoTaskMemFree(Some(value.0 as *const _));
        s
    }
}

fn open_reader(id: &str) -> windows::core::Result<IMFSourceReader> {
    // SAFETY: COM calls with checked results.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
        attrs.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        attrs.SetString(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            windows::core::PCWSTR(wide.as_ptr()),
        )?;
        let source = MFCreateDeviceSource(&attrs)?;

        let mut reader_attrs = None;
        MFCreateAttributes(&mut reader_attrs, 1)?;
        let reader_attrs = reader_attrs.ok_or_else(windows::core::Error::empty)?;
        // Lets the reader decode MJPEG and convert YUY2 etc. to NV12.
        reader_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        MFCreateSourceReaderFromMediaSource(&source, &reader_attrs)
    }
}

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn native_formats(reader: &IMFSourceReader) -> Vec<(u32, CameraFormat)> {
    let mut out = Vec::new();
    for index in 0.. {
        // SAFETY: COM queries with checked results.
        let Ok(t) = (unsafe { reader.GetNativeMediaType(STREAM, index) }) else {
            break;
        };
        // SAFETY: attribute reads on a valid media type.
        let parsed = unsafe {
            (|| -> windows::core::Result<CameraFormat> {
                let (width, height) = unpack(t.GetUINT64(&MF_MT_FRAME_SIZE)?);
                let (fps_num, fps_den) = unpack(t.GetUINT64(&MF_MT_FRAME_RATE)?);
                Ok(CameraFormat {
                    width,
                    height,
                    fps_num,
                    fps_den,
                    subtype: subtype_name(t.GetGUID(&MF_MT_SUBTYPE)?),
                })
            })()
        };
        if let Ok(f) = parsed {
            out.push((index, f));
        }
    }
    out
}

/// Formats a device supports (cached; opening a device is slow).
pub fn formats(id: &str) -> Vec<CameraFormat> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<CameraFormat>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(found) = cache.lock().ok().and_then(|c| c.get(id).cloned()) {
        return found;
    }
    if !startup() {
        return Vec::new();
    }
    let mut list: Vec<CameraFormat> = open_reader(id)
        .map(|r| native_formats(&r).into_iter().map(|(_, f)| f).collect())
        .unwrap_or_default();
    // Compressed bitstreams (H.264) need a decoder path we don't use here.
    list.retain(|f| f.subtype != "H264");
    list.sort_by(|a, b| {
        (b.width * b.height, (b.fps() * 1000.0) as u64)
            .cmp(&(a.width * a.height, (a.fps() * 1000.0) as u64))
    });
    list.dedup_by(|a, b| a.key() == b.key());
    if let Ok(mut c) = cache.lock() {
        c.insert(id.to_owned(), list.clone());
    }
    list
}

// ------------------------------------------------------------------ worker

#[derive(Default)]
struct Shared {
    frame: Vec<u8>,
    width: u32,
    height: u32,
    stride: u32,
    matrix: Option<YuvMatrix>,
    full_range: bool,
    sequence: u64,
    /// Set when the worker stops (with a reason).
    stopped: Option<Stopped>,
}

/// Why a camera worker stopped.
#[derive(Debug)]
struct Stopped {
    reason: String,
    /// The camera is still connected: reading just failed (a USB hiccup, a glitched frame), so
    /// opening it again right away usually works.
    still_connected: bool,
}

struct Worker {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn spawn_worker(device: String, format: String) -> Worker {
    let stop = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Shared::default()));
    let thread = {
        let (stop, shared) = (stop.clone(), shared.clone());
        std::thread::Builder::new()
            .name("estream-camera".into())
            .spawn(move || {
                let reason = match run_worker(&device, &format, &stop, &shared) {
                    Ok(()) => "Camera stopped".to_owned(),
                    Err(e) => e,
                };
                let still_connected = devices().iter().any(|d| d.id == device);
                if let Ok(mut s) = shared.lock() {
                    s.stopped = Some(Stopped {
                        reason: if still_connected {
                            reason
                        } else {
                            "Camera disconnected".into()
                        },
                        still_connected,
                    });
                }
            })
            .ok()
    };
    Worker {
        stop,
        shared,
        thread,
    }
}

fn run_worker(
    device: &str,
    format: &str,
    stop: &AtomicBool,
    shared: &Mutex<Shared>,
) -> Result<(), String> {
    // SAFETY: per-thread COM init; balanced by thread exit.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if !startup() {
        return Err("Media Foundation is unavailable".into());
    }
    if !devices().iter().any(|d| d.id == device) {
        return Err("Camera is not connected".into());
    }
    let reader =
        open_reader(device).map_err(|e| format!("Couldn't open the camera: {}", e.message()))?;
    let natives = native_formats(&reader);
    let wanted = natives
        .iter()
        .find(|(_, f)| f.key() == format)
        .or_else(|| {
            let best = best_format(&natives.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>())?;
            natives.iter().find(|(_, f)| *f == best)
        })
        .cloned()
        .ok_or("The camera reports no usable formats")?;

    // SAFETY: COM calls with checked results.
    let (width, height, matrix, full_range) = unsafe {
        let native = reader
            .GetNativeMediaType(STREAM, wanted.0)
            .map_err(|e| e.message())?;
        reader
            .SetCurrentMediaType(STREAM, None, &native)
            .map_err(|e| e.message())?;
        let out = MFCreateMediaType().map_err(|e| e.message())?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| e.message())?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
            .map_err(|e| e.message())?;
        out.SetUINT64(
            &MF_MT_FRAME_SIZE,
            ((wanted.1.width as u64) << 32) | wanted.1.height as u64,
        )
        .map_err(|e| e.message())?;
        reader
            .SetCurrentMediaType(STREAM, None, &out)
            .map_err(|e| format!("The camera can't deliver this format: {}", e.message()))?;
        let current = reader
            .GetCurrentMediaType(STREAM)
            .map_err(|e| e.message())?;
        let (w, h) = unpack(
            current
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .map_err(|e| e.message())?,
        );
        let matrix = match current
            .GetUINT32(&MF_MT_YUV_MATRIX)
            .map(|m| MFVideoTransferMatrix(m as i32))
        {
            Ok(m) if m == MFVideoTransferMatrix_BT601 => Some(YuvMatrix::Bt601),
            Ok(m) if m == MFVideoTransferMatrix_BT709 => Some(YuvMatrix::Bt709),
            _ => None,
        };
        let full = current
            .GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE)
            .is_ok_and(|r| MFNominalRange(r as i32) == MFNominalRange_0_255);
        (w, h, matrix, full)
    };
    tracing::info!(device, format = %wanted.1.label(), "camera started");

    let mut back: Vec<u8> = Vec::new();
    // Reads that failed in a row. Cheap USB webcams drop the odd read; only a run of them means
    // the reader needs opening again.
    let mut failed = 0u32;
    while !stop.load(Ordering::Acquire) {
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        // SAFETY: synchronous read with valid out-pointers.
        let read = unsafe {
            reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample))
        };
        // The reader can't go on after these.
        let fatal = flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0
            || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
        if let Err(e) = &read {
            failed += 1;
            tracing::warn!(device, error = %e.code(), failed, "camera read failed");
            if !fatal && failed < 3 {
                std::thread::sleep(Duration::from_millis(30));
                continue;
            }
        }
        if fatal || read.is_err() {
            tracing::warn!(device, flags, "camera stream ended");
            return Err("The camera stopped sending pictures".into());
        }
        failed = 0;
        let Some(sample) = sample else { continue };
        // SAFETY: the buffer is locked only for the copy below.
        let stride = unsafe {
            let Ok(buffer) = sample.ConvertToContiguousBuffer() else {
                continue;
            };
            let mut stride = width;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let two_d = buffer.cast::<IMF2DBuffer>().ok();
            let locked = match &two_d {
                Some(b) => {
                    let mut pitch = 0i32;
                    let r = b.Lock2D(&mut ptr, &mut pitch);
                    if pitch > 0 {
                        stride = pitch as u32;
                    }
                    r
                }
                None => buffer.Lock(&mut ptr, None, None),
            };
            if locked.is_err() || ptr.is_null() {
                continue;
            }
            let len = (stride * height + stride * (height / 2)) as usize;
            back.clear();
            back.extend_from_slice(std::slice::from_raw_parts(ptr, len));
            match &two_d {
                Some(b) => {
                    let _ = b.Unlock2D();
                }
                None => {
                    let _ = buffer.Unlock();
                }
            }
            stride
        };
        if let Ok(mut s) = shared.lock() {
            std::mem::swap(&mut s.frame, &mut back);
            s.width = width;
            s.height = height;
            s.stride = stride;
            s.matrix = matrix;
            s.full_range = full_range;
            s.sequence += 1;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ source

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    pub device: String,
    /// Format key (see [`CameraFormat::key`]); empty = automatic.
    pub format: String,
}

pub struct CameraSource {
    settings: CameraSettings,
    worker: Option<Worker>,
    converter: Arc<YuvConverter>,
    frame: Option<Nv12Frame>,
    binding: Option<wgpu::BindGroup>,
    sequence: u64,
    status: SourceStatus,
    next_attempt: Instant,
    /// When the camera recently dropped out while still connected (to notice it happening often).
    dropouts: Vec<Instant>,
}

/// Dropouts within this window that make the camera count as unreliable.
const DROPOUT_WINDOW: Duration = Duration::from_secs(20);
const DROPOUTS_SHOWN: usize = 3;

impl CameraSource {
    fn new(ctx: &SourceContext<'_>) -> Self {
        Self {
            settings: CameraSettings::default(),
            worker: None,
            converter: YuvConverter::new(ctx.gpu),
            frame: None,
            binding: None,
            sequence: 0,
            status: SourceStatus::Unavailable("Starting camera…".into()),
            next_attempt: Instant::now(),
            dropouts: Vec::new(),
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
            self.worker = Some(spawn_worker(
                self.settings.device.clone(),
                self.settings.format.clone(),
            ));
        }
        let Some(worker) = &self.worker else { return };
        // Never block the video thread: skip this tick if the worker holds the lock.
        let Ok(mut shared) = worker.shared.try_lock() else {
            return;
        };
        if let Some(stopped) = shared.stopped.take() {
            drop(shared);
            self.worker = None;
            let now = Instant::now();
            if stopped.still_connected && self.frame.is_some() {
                // A hiccup while running: open it again at once and keep showing the last
                // picture meanwhile. Only say something when it keeps happening.
                self.dropouts
                    .retain(|t| now.duration_since(*t) < DROPOUT_WINDOW);
                self.dropouts.push(now);
                self.next_attempt = now;
                if self.dropouts.len() >= DROPOUTS_SHOWN {
                    self.status = SourceStatus::Unavailable(
                        "The camera keeps dropping out. Try another USB port (not a hub) or a \
                         lower resolution."
                            .into(),
                    );
                }
            } else {
                self.next_attempt = now + Duration::from_secs(2);
                self.status = SourceStatus::Unavailable(stopped.reason);
            }
            return;
        }
        if shared.sequence == self.sequence || shared.frame.is_empty() {
            return;
        }
        self.sequence = shared.sequence;
        let (w, h) = (shared.width, shared.height);
        if self
            .frame
            .as_ref()
            .is_none_or(|f| f.width != (w & !1) || f.height != (h & !1))
        {
            let frame = self.converter.create_frame(ctx.gpu, w, h);
            self.binding = Some(
                ctx.compositor
                    .create_source_binding(ctx.gpu, &frame.output.create_view(&Default::default())),
            );
            self.frame = Some(frame);
        }
        if let Some(frame) = &self.frame {
            let matrix = shared.matrix.unwrap_or(YuvMatrix::for_height(h));
            frame.upload(
                ctx.gpu,
                &shared.frame,
                shared.stride,
                matrix,
                shared.full_range,
            );
            drop(shared);
            frame.convert(&self.converter, encoder);
            // A warning about repeated dropouts stays up until they stop for a while.
            let now = Instant::now();
            self.dropouts
                .retain(|t| now.duration_since(*t) < DROPOUT_WINDOW);
            if self.dropouts.len() < DROPOUTS_SHOWN {
                self.status = SourceStatus::Active;
            }
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        let frame = self.frame.as_ref()?;
        Some(SourceFrame {
            binding: self.binding.as_ref()?,
            size: Size::new(frame.width as f32, frame.height as f32),
        })
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
            if !device.is_empty() {
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
    fn enumeration_does_not_crash() {
        for d in devices() {
            assert!(!d.id.is_empty());
        }
    }
}
