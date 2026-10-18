//! The Media Foundation camera source that Windows' Frame Server loads for the virtual camera:
//! an activator, a live media source and one NV12 video stream. Frames come from Clutch
//! through [`crate::shared`]; without them the camera shows a dark placeholder.

use crate::shared::{Shared, fill_nv12, fit_nv12, nv12_len};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{E_INVALIDARG, E_POINTER, ERROR_SET_NOT_FOUND, S_OK};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsControl_Impl, KSIDENTIFIER, PINNAME_VIDEO_CAPTURE,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{
    BOOL, ComObject, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
};

/// Frame sizes offered, best first (apps usually pick 1280×720).
pub const FORMATS: [(u32, u32); 3] = [(1920, 1080), (1280, 720), (640, 360)];
pub const FPS: u32 = 30;
const FRAME_100NS: i64 = 10_000_000 / FPS as i64;
const NV12: u32 = u32::from_le_bytes(*b"NV12");

/// A COM pointer moved between threads. Media Foundation objects used here are free-threaded.
struct Agile<T>(T);
// SAFETY: see above — MF event queues, attributes, descriptors, samples and allocators are
// documented as callable from any thread.
unsafe impl<T> Send for Agile<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Agile<T> {}

fn debug(message: &str) {
    let text = windows::core::HSTRING::from(format!("estream-vcam: {message}\n"));
    // SAFETY: valid null-terminated string.
    unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(&text) };
    crate::log(message);
}

fn pv_i64(value: i64) -> PROPVARIANT {
    let mut pv = PROPVARIANT::default();
    // SAFETY: plain union writes of a VT_I8 variant.
    unsafe {
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = windows::Win32::System::Variant::VT_I8;
        inner.Anonymous.hVal = value;
    }
    pv
}

fn attributes(count: u32) -> Result<IMFAttributes> {
    let mut out = None;
    // SAFETY: out-pointer to an Option.
    unsafe { MFCreateAttributes(&mut out, count)? };
    out.ok_or_else(|| E_POINTER.into())
}

fn media_type(width: u32, height: u32) -> Result<IMFMediaType> {
    // SAFETY: plain MF calls on a new media type.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        t.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, width)?;
        t.SetUINT32(&MF_MT_SAMPLE_SIZE, nv12_len(width, height) as u32)?;
        // Clutch renders BT.709, limited range; say so, or apps guess (BT.601 for small sizes).
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        Ok(t)
    }
}

fn frame_size(t: &IMFMediaType) -> Result<(u32, u32)> {
    // SAFETY: reads an attribute.
    let packed = unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE)? };
    let (w, h) = ((packed >> 32) as u32, packed as u32);
    if FORMATS.contains(&(w, h)) {
        Ok((w, h))
    } else {
        Err(E_INVALIDARG.into())
    }
}

/// Creates the media source (what the activator hands to Frame Server). `mapping` is where
/// Clutch's frames are shared ([`crate::shared::MAPPING_NAME`] in production).
pub fn create_source(mapping: &str) -> Result<IMFMediaSource> {
    let stream_attributes = attributes(4)?;
    let types = FORMATS
        .iter()
        .map(|&(w, h)| media_type(w, h).map(Some))
        .collect::<Result<Vec<_>>>()?;
    // SAFETY: plain MF calls building descriptors.
    let (descriptor, stream_descriptor) = unsafe {
        let attrs = &stream_attributes;
        attrs.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        attrs.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        attrs.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        attrs.SetUINT32(
            &MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
            MFFrameSourceTypes_Color.0 as u32,
        )?;
        let sd = MFCreateStreamDescriptor(0, &types)?;
        stream_attributes.CopyAllItems(&sd)?;
        let handler = sd.GetMediaTypeHandler()?;
        handler.SetCurrentMediaType(types[0].as_ref().ok_or(E_POINTER)?)?;
        let pd = MFCreatePresentationDescriptor(Some(&[Some(sd.clone())]))?;
        pd.SelectStream(0)?;
        (pd, sd)
    };
    // SAFETY: new event queues.
    let (source_queue, stream_queue) = unsafe { (MFCreateEventQueue()?, MFCreateEventQueue()?) };
    let stream = ComObject::new(MediaStream {
        queue: Agile(stream_queue.clone()),
        descriptor: Agile(stream_descriptor),
        attributes: Agile(stream_attributes),
        source: Mutex::new(None),
        inner: Arc::new(StreamInner {
            queue: Agile(stream_queue),
            state: Mutex::new(MF_STREAM_STATE_STOPPED),
            tokens: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            allocator: Mutex::new(None),
            format: Mutex::new(FORMATS[0]),
            mapping: mapping.to_owned(),
            frames: Mutex::new(Frames::default()),
            stop: AtomicBool::new(false),
        }),
        worker: Mutex::new(None),
        _live: Live::new(),
    });
    let source = ComObject::new(MediaSource {
        queue: Agile(source_queue),
        attributes: Agile(attributes(2)?),
        descriptor: Agile(descriptor),
        stream: Mutex::new(Some(Agile(stream.clone()))),
        started_once: AtomicBool::new(false),
        shut_down: AtomicBool::new(false),
        _live: Live::new(),
    });
    let interface: IMFMediaSource = source.to_interface::<IMFMediaSourceEx>().into();
    *stream.source.lock().unwrap_or_else(|e| e.into_inner()) = Some(Agile(interface.clone()));
    Ok(interface)
}

// ---------------------------------------------------------------- source

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl, IMFSampleAllocatorControl)]
struct MediaSource {
    queue: Agile<IMFMediaEventQueue>,
    attributes: Agile<IMFAttributes>,
    descriptor: Agile<IMFPresentationDescriptor>,
    stream: Mutex<Option<Agile<ComObject<MediaStream>>>>,
    started_once: AtomicBool,
    shut_down: AtomicBool,
    _live: Live,
}

/// COM objects alive in this module (the DLL may unload when there are none).
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Counts one live object for [`objects_alive`].
struct Live;

impl Live {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

/// True while any activator, source or stream from this module exists.
pub fn objects_alive() -> bool {
    LIVE.load(Ordering::Acquire) > 0
}

impl MediaSource {
    fn check(&self) -> Result<()> {
        if self.shut_down.load(Ordering::Acquire) {
            Err(MF_E_SHUTDOWN.into())
        } else {
            Ok(())
        }
    }

    fn stream(&self) -> Result<ComObject<MediaStream>> {
        self.stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.0.clone())
            .ok_or_else(|| MF_E_SHUTDOWN.into())
    }
}

macro_rules! event_generator {
    ($ty:ty) => {
        impl IMFMediaEventGenerator_Impl for $ty {
            fn GetEvent(
                &self,
                flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
            ) -> Result<IMFMediaEvent> {
                // SAFETY: forwarded to the event queue.
                unsafe { self.queue.0.GetEvent(flags.0 as u32) }
            }

            fn BeginGetEvent(
                &self,
                callback: Ref<IMFAsyncCallback>,
                state: Ref<IUnknown>,
            ) -> Result<()> {
                // SAFETY: forwarded to the event queue.
                unsafe {
                    self.queue
                        .0
                        .BeginGetEvent(callback.as_ref(), state.as_ref())
                }
            }

            fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
                // SAFETY: forwarded to the event queue.
                unsafe { self.queue.0.EndGetEvent(result.as_ref()) }
            }

            fn QueueEvent(
                &self,
                met: u32,
                extended: *const GUID,
                status: HRESULT,
                value: *const PROPVARIANT,
            ) -> Result<()> {
                // SAFETY: forwarded to the event queue.
                unsafe {
                    self.queue
                        .0
                        .QueueEventParamVar(met, extended, status, value)
                }
            }
        }

        impl IKsControl_Impl for $ty {
            fn KsProperty(
                &self,
                _: *const KSIDENTIFIER,
                _: u32,
                _: *mut core::ffi::c_void,
                _: u32,
                _: *mut u32,
            ) -> Result<()> {
                Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
            }

            fn KsMethod(
                &self,
                _: *const KSIDENTIFIER,
                _: u32,
                _: *mut core::ffi::c_void,
                _: u32,
                _: *mut u32,
            ) -> Result<()> {
                Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
            }

            fn KsEvent(
                &self,
                _: *const KSIDENTIFIER,
                _: u32,
                _: *mut core::ffi::c_void,
                _: u32,
                _: *mut u32,
            ) -> Result<()> {
                Err(HRESULT::from_win32(ERROR_SET_NOT_FOUND.0).into())
            }
        }
    };
}

event_generator!(MediaSource_Impl);
event_generator!(MediaStream_Impl);

impl IMFMediaSource_Impl for MediaSource_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.check()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.check()?;
        // SAFETY: clones our descriptor.
        unsafe { self.descriptor.0.Clone() }
    }

    fn Start(
        &self,
        descriptor: Ref<IMFPresentationDescriptor>,
        time_format: *const GUID,
        _start: *const PROPVARIANT,
    ) -> Result<()> {
        self.check()?;
        // SAFETY: optional pointer from the caller.
        if !time_format.is_null() && unsafe { *time_format } != GUID::zeroed() {
            return Err(MF_E_UNSUPPORTED_TIME_FORMAT.into());
        }
        let pd = descriptor.ok()?;
        let stream = self.stream()?;
        let mut selected = BOOL(0);
        let mut sd = None;
        // SAFETY: reads the caller's descriptor.
        unsafe { pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)? };
        let time = unsafe { MFGetSystemTime() };
        if selected.as_bool() {
            let sd = sd.ok_or(E_POINTER)?;
            // SAFETY: reads the chosen media type.
            let current = unsafe { sd.GetMediaTypeHandler()?.GetCurrentMediaType()? };
            let (w, h) = frame_size(&current)?;
            let unknown: IUnknown = stream.to_interface();
            let met = if self.started_once.swap(true, Ordering::AcqRel) {
                MEUpdatedStream
            } else {
                MENewStream
            };
            // SAFETY: queues an event carrying the stream.
            unsafe {
                self.queue
                    .0
                    .QueueEventParamUnk(met.0 as u32, &GUID::zeroed(), S_OK, &unknown)?;
            }
            stream.start(w, h, &current, time)?;
        } else {
            stream.stop(false);
        }
        debug(&format!(
            "source started (selected: {})",
            selected.as_bool()
        ));
        // SAFETY: queues MESourceStarted with the start time.
        unsafe {
            self.queue.0.QueueEventParamVar(
                MESourceStarted.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &pv_i64(time),
            )
        }
    }

    fn Stop(&self) -> Result<()> {
        self.check()?;
        self.stream()?.stop(true);
        let time = unsafe { MFGetSystemTime() };
        // SAFETY: queues MESourceStopped.
        unsafe {
            self.queue.0.QueueEventParamVar(
                MESourceStopped.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &pv_i64(time),
            )
        }
    }

    fn Pause(&self) -> Result<()> {
        self.check()?;
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> Result<()> {
        if self.shut_down.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        debug("source shut down");
        if let Some(stream) = self.stream.lock().unwrap_or_else(|e| e.into_inner()).take() {
            stream.0.shutdown();
        }
        // SAFETY: shuts our own queue down.
        unsafe { self.queue.0.Shutdown() }
    }
}

impl IMFMediaSourceEx_Impl for MediaSource_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        self.check()?;
        Ok(self.attributes.0.clone())
    }

    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        self.check()?;
        if id != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(self.stream()?.attributes.0.clone())
    }

    fn SetD3DManager(&self, _: Ref<IUnknown>) -> Result<()> {
        // Samples stay in system memory; Frame Server uploads them as needed.
        self.check()
    }
}

impl IMFGetService_Impl for MediaSource_Impl {
    fn GetService(
        &self,
        _: *const GUID,
        _: *const GUID,
        _: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IMFSampleAllocatorControl_Impl for MediaSource_Impl {
    fn SetDefaultAllocator(&self, stream: u32, allocator: Ref<IUnknown>) -> Result<()> {
        self.check()?;
        if stream != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        let allocator = allocator
            .as_ref()
            .and_then(|a| a.cast::<IMFVideoSampleAllocator>().ok());
        *self
            .stream()?
            .inner
            .allocator
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = allocator.map(Agile);
        Ok(())
    }

    fn GetAllocatorUsage(
        &self,
        stream: u32,
        input: *mut u32,
        usage: *mut MFSampleAllocatorUsage,
    ) -> Result<()> {
        if input.is_null() || usage.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked out-pointers.
        unsafe {
            *input = stream;
            *usage = MFSampleAllocatorUsage_UsesProvidedAllocator;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- stream

#[implement(IMFMediaStream2, IKsControl)]
struct MediaStream {
    queue: Agile<IMFMediaEventQueue>,
    descriptor: Agile<IMFStreamDescriptor>,
    attributes: Agile<IMFAttributes>,
    source: Mutex<Option<Agile<IMFMediaSource>>>,
    inner: Arc<StreamInner>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    _live: Live,
}

/// Stream state shared with the delivery thread.
struct StreamInner {
    queue: Agile<IMFMediaEventQueue>,
    state: Mutex<MF_STREAM_STATE>,
    /// Pending RequestSample tokens (`None` = no token).
    tokens: Mutex<VecDeque<Option<Agile<IUnknown>>>>,
    wake: Condvar,
    allocator: Mutex<Option<Agile<IMFVideoSampleAllocator>>>,
    format: Mutex<(u32, u32)>,
    mapping: String,
    frames: Mutex<Frames>,
    stop: AtomicBool,
}

/// The current output picture and where it comes from.
#[derive(Default)]
struct Frames {
    shared: Option<Shared>,
    /// Retry opening the mapping after this.
    retry_at: Option<Instant>,
    picture: Vec<u8>,
    scratch: Vec<u8>,
    last_seq: u64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl MediaStream {
    fn start(&self, w: u32, h: u32, media_type: &IMFMediaType, time: i64) -> Result<()> {
        let inner = &self.inner;
        *lock(&inner.format) = (w, h);
        if let Some(allocator) = lock(&inner.allocator).as_ref() {
            // SAFETY: (re)initializes the provided allocator for the chosen type.
            unsafe {
                let _ = allocator.0.UninitializeSampleAllocator();
                allocator.0.InitializeSampleAllocator(10, media_type)?;
            }
        }
        {
            let mut frames = lock(&inner.frames);
            frames.last_seq = 0;
            frames.picture = vec![0; nv12_len(w, h)];
            placeholder(&mut frames.picture, w, h);
            if let Some(shared) = &frames.shared {
                shared.set_wanted(w, h);
            }
        }
        *lock(&inner.state) = MF_STREAM_STATE_RUNNING;
        inner.stop.store(false, Ordering::Release);
        let mut worker = lock(&self.worker);
        if worker.as_ref().is_none_or(|w| w.is_finished()) {
            let inner = inner.clone();
            *worker = std::thread::Builder::new()
                .name("estream-vcam".into())
                .spawn(move || deliver(inner))
                .ok();
        }
        // SAFETY: queues MEStreamStarted.
        unsafe {
            self.queue.0.QueueEventParamVar(
                MEStreamStarted.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &pv_i64(time),
            )
        }
    }

    fn stop(&self, notify: bool) {
        *lock(&self.inner.state) = MF_STREAM_STATE_STOPPED;
        self.inner.stop.store(true, Ordering::Release);
        self.inner.wake.notify_all();
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
        lock(&self.inner.tokens).clear();
        if notify {
            let time = unsafe { MFGetSystemTime() };
            // SAFETY: queues MEStreamStopped.
            unsafe {
                let _ = self.queue.0.QueueEventParamVar(
                    MEStreamStopped.0 as u32,
                    &GUID::zeroed(),
                    S_OK,
                    &pv_i64(time),
                );
            }
        }
    }

    fn shutdown(&self) {
        self.stop(false);
        *lock(&self.source) = None;
        if let Some(allocator) = lock(&self.inner.allocator).take() {
            // SAFETY: releases the allocator's samples.
            unsafe {
                let _ = allocator.0.UninitializeSampleAllocator();
            }
        }
        lock(&self.inner.frames).shared = None;
        // SAFETY: shuts our own queue down.
        unsafe {
            let _ = self.queue.0.Shutdown();
        }
    }
}

impl IMFMediaStream_Impl for MediaStream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        lock(&self.source)
            .as_ref()
            .map(|s| s.0.clone())
            .ok_or_else(|| MF_E_SHUTDOWN.into())
    }

    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        Ok(self.descriptor.0.clone())
    }

    fn RequestSample(&self, token: Ref<IUnknown>) -> Result<()> {
        if *lock(&self.inner.state) != MF_STREAM_STATE_RUNNING {
            return Err(MF_E_MEDIA_SOURCE_WRONGSTATE.into());
        }
        lock(&self.inner.tokens).push_back(token.as_ref().cloned().map(Agile));
        self.inner.wake.notify_one();
        Ok(())
    }
}

impl IMFMediaStream2_Impl for MediaStream_Impl {
    fn SetStreamState(&self, state: MF_STREAM_STATE) -> Result<()> {
        match state {
            MF_STREAM_STATE_STOPPED => self.stop(true),
            MF_STREAM_STATE_PAUSED => *lock(&self.inner.state) = MF_STREAM_STATE_PAUSED,
            MF_STREAM_STATE_RUNNING => {
                let (w, h) = *lock(&self.inner.format);
                let t = media_type(w, h)?;
                self.start(w, h, &t, unsafe { MFGetSystemTime() })?;
            }
            _ => return Err(E_INVALIDARG.into()),
        }
        Ok(())
    }

    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        Ok(*lock(&self.inner.state))
    }
}

/// Dark placeholder shown while Clutch isn't sending frames.
fn placeholder(picture: &mut [u8], w: u32, h: u32) {
    fill_nv12(picture, w, h, 28, 128, 128);
}

/// Delivery thread: answers sample requests no faster than the frame rate.
fn deliver(inner: Arc<StreamInner>) {
    // SAFETY: per-thread COM init (balanced below).
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let interval = Duration::from_nanos(1_000_000_000 / FPS as u64);
    let mut next = Instant::now();
    loop {
        let token = {
            let mut tokens = lock(&inner.tokens);
            loop {
                if inner.stop.load(Ordering::Acquire) {
                    break None;
                }
                if let Some(t) = tokens.pop_front() {
                    break Some(t);
                }
                tokens = inner
                    .wake
                    .wait_timeout(tokens, Duration::from_millis(100))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        };
        let Some(token) = token else { break };
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next = next.max(now) + interval;
        if inner.stop.load(Ordering::Acquire) {
            break;
        }
        match sample(&inner, token) {
            // SAFETY: queues the sample.
            Ok(sample) => unsafe {
                if inner
                    .queue
                    .0
                    .QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, &sample)
                    .is_err()
                {
                    break; // queue shut down
                }
            },
            Err(e) => debug(&format!("sample failed: {e}")),
        }
    }
    if com {
        // SAFETY: balances CoInitializeEx.
        unsafe { CoUninitialize() };
    }
}

/// Refreshes the picture from shared memory (or the placeholder).
fn refresh(inner: &StreamInner, w: u32, h: u32) -> std::sync::MutexGuard<'_, Frames> {
    let mut frames = lock(&inner.frames);
    let frames_ref = &mut *frames;
    if frames_ref.picture.len() != nv12_len(w, h) {
        frames_ref.picture = vec![0; nv12_len(w, h)];
        placeholder(&mut frames_ref.picture, w, h);
    }
    if frames_ref.shared.is_none() && frames_ref.retry_at.is_none_or(|t| Instant::now() >= t) {
        match Shared::create(&inner.mapping) {
            Ok(s) => {
                s.set_wanted(w, h);
                frames_ref.shared = Some(s);
            }
            Err(e) => {
                debug(&format!("shared memory unavailable: {e}"));
                frames_ref.retry_at = Some(Instant::now() + Duration::from_secs(2));
            }
        }
    }
    if let Some(shared) = &frames_ref.shared {
        shared.set_wanted(w, h);
        shared.touch_consumer();
        if let Some((fw, fh)) = shared.read_frame(&mut frames_ref.scratch, &mut frames_ref.last_seq)
        {
            fit_nv12(&frames_ref.scratch, fw, fh, &mut frames_ref.picture, w, h);
        }
        if !shared.producer_alive(2000) {
            placeholder(&mut frames_ref.picture, w, h);
            frames_ref.last_seq = 0;
        }
    }
    frames
}

/// Builds one sample with the current picture.
fn sample(inner: &StreamInner, token: Option<Agile<IUnknown>>) -> Result<IMFSample> {
    let (w, h) = *lock(&inner.format);
    let frames = refresh(inner, w, h);
    let allocator = lock(&inner.allocator).as_ref().map(|a| a.0.clone());
    // SAFETY: MF sample/buffer calls; writes stay within the locked buffer.
    unsafe {
        let sample = match allocator {
            Some(a) => a.AllocateSample()?,
            None => {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreate2DMediaBuffer(w, h, NV12, false)?)?;
                s
            }
        };
        let buffer = sample.GetBufferByIndex(0)?;
        write_nv12(&buffer, &frames.picture, w, h)?;
        drop(frames);
        sample.SetSampleTime(MFGetSystemTime())?;
        sample.SetSampleDuration(FRAME_100NS)?;
        if let Some(token) = token {
            sample.SetUnknown(&MFSampleExtension_Token, &token.0)?;
        }
        Ok(sample)
    }
}

/// Copies a tight NV12 picture into an MF buffer, honoring its pitch.
///
/// # Safety
/// `buffer` must be a valid video buffer of at least `w`×`h` NV12.
unsafe fn write_nv12(buffer: &IMFMediaBuffer, picture: &[u8], w: u32, h: u32) -> Result<()> {
    let (w, h) = (w as usize, h as usize);
    let rows = h + h / 2;
    // SAFETY: per the function contract; every write is bounded by the lock's pitch and rows.
    unsafe {
        if let Ok(b2) = buffer.cast::<IMF2DBuffer2>() {
            let mut scan0 = std::ptr::null_mut();
            let mut pitch = 0i32;
            let mut start = std::ptr::null_mut();
            let mut len = 0u32;
            b2.Lock2DSize(
                MF2DBuffer_LockFlags_Write,
                &mut scan0,
                &mut pitch,
                &mut start,
                &mut len,
            )?;
            if pitch > 0 && pitch as usize >= w && (pitch as usize * rows) <= len as usize {
                for r in 0..rows {
                    std::ptr::copy_nonoverlapping(
                        picture.as_ptr().add(r * w),
                        scan0.add(r * pitch as usize),
                        w,
                    );
                }
            }
            b2.Unlock2D()?;
        } else {
            let mut data = std::ptr::null_mut();
            let mut max = 0u32;
            buffer.Lock(&mut data, Some(&mut max), None)?;
            let n = (w * rows).min(max as usize);
            std::ptr::copy_nonoverlapping(picture.as_ptr(), data, n);
            buffer.Unlock()?;
            buffer.SetCurrentLength(n as u32)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- activator

/// What Frame Server creates from the CLSID: hands out the media source on demand.
#[implement(IMFActivate)]
pub(crate) struct Activator {
    attributes: Agile<IMFAttributes>,
    source: Mutex<Option<Agile<IMFMediaSource>>>,
    mapping: String,
    _live: Live,
}

impl Activator {
    pub(crate) fn new(mapping: &str) -> Result<Self> {
        Ok(Self {
            attributes: Agile(attributes(1)?),
            source: Mutex::new(None),
            mapping: mapping.to_owned(),
            _live: Live::new(),
        })
    }
}

impl IMFActivate_Impl for Activator_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        let mut source = lock(&self.source);
        if source.is_none() {
            *source = Some(Agile(create_source(&self.mapping)?));
            debug("source created");
        }
        let source = source.as_ref().ok_or(E_POINTER)?;
        // SAFETY: standard QueryInterface out-parameter.
        let result = unsafe { source.0.query(riid, ppv).ok() };
        // SAFETY: riid is valid per the COM contract.
        debug(&format!("ActivateObject {:?}: {result:?}", unsafe {
            *riid
        }));
        result
    }

    fn ShutdownObject(&self) -> Result<()> {
        if let Some(source) = lock(&self.source).take() {
            // SAFETY: shuts the source down.
            unsafe {
                let _ = source.0.Shutdown();
            }
        }
        Ok(())
    }

    fn DetachObject(&self) -> Result<()> {
        *lock(&self.source) = None;
        Ok(())
    }
}

/// `IMFAttributes` is answered by an inner attribute store. Every call forwards the caller's
/// pointers unchanged (SAFETY: they are valid per the COM contract).
impl IMFAttributes_Impl for Activator_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe {
            self.attributes
                .0
                .GetItem(key, (!value.is_null()).then_some(value))
        }
    }
    fn GetItemType(&self, key: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attributes.0.GetItemType(key) }
    }
    fn CompareItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<BOOL> {
        unsafe { self.attributes.0.CompareItem(key, value) }
    }
    fn Compare(&self, theirs: Ref<IMFAttributes>, kind: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        unsafe { self.attributes.0.Compare(theirs.as_ref(), kind) }
    }
    fn GetUINT32(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.0.GetUINT32(key) }
    }
    fn GetUINT64(&self, key: *const GUID) -> Result<u64> {
        unsafe { self.attributes.0.GetUINT64(key) }
    }
    fn GetDouble(&self, key: *const GUID) -> Result<f64> {
        unsafe { self.attributes.0.GetDouble(key) }
    }
    fn GetGUID(&self, key: *const GUID) -> Result<GUID> {
        unsafe { self.attributes.0.GetGUID(key) }
    }
    fn GetStringLength(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.0.GetStringLength(key) }
    }
    fn GetString(&self, key: *const GUID, value: PWSTR, size: u32, length: *mut u32) -> Result<()> {
        if value.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the caller provides `size` u16s at `value`.
        let buf = unsafe { std::slice::from_raw_parts_mut(value.0, size as usize) };
        unsafe {
            self.attributes
                .0
                .GetString(key, buf, (!length.is_null()).then_some(length))
        }
    }
    fn GetAllocatedString(
        &self,
        key: *const GUID,
        value: *mut PWSTR,
        length: *mut u32,
    ) -> Result<()> {
        unsafe { self.attributes.0.GetAllocatedString(key, value, length) }
    }
    fn GetBlobSize(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.0.GetBlobSize(key) }
    }
    fn GetBlob(&self, key: *const GUID, buf: *mut u8, size: u32, written: *mut u32) -> Result<()> {
        if buf.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the caller provides `size` bytes at `buf`.
        let slice = unsafe { std::slice::from_raw_parts_mut(buf, size as usize) };
        unsafe {
            self.attributes
                .0
                .GetBlob(key, slice, (!written.is_null()).then_some(written))
        }
    }
    fn GetAllocatedBlob(&self, key: *const GUID, buf: *mut *mut u8, size: *mut u32) -> Result<()> {
        unsafe { self.attributes.0.GetAllocatedBlob(key, buf, size) }
    }
    fn GetUnknown(
        &self,
        key: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        let unknown: IUnknown = unsafe { self.attributes.0.GetUnknown(key)? };
        // SAFETY: standard QueryInterface out-parameter.
        unsafe { unknown.query(riid, ppv).ok() }
    }
    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.0.SetItem(key, value) }
    }
    fn DeleteItem(&self, key: *const GUID) -> Result<()> {
        unsafe { self.attributes.0.DeleteItem(key) }
    }
    fn DeleteAllItems(&self) -> Result<()> {
        unsafe { self.attributes.0.DeleteAllItems() }
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> Result<()> {
        unsafe { self.attributes.0.SetUINT32(key, value) }
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> Result<()> {
        unsafe { self.attributes.0.SetUINT64(key, value) }
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> Result<()> {
        unsafe { self.attributes.0.SetDouble(key, value) }
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> Result<()> {
        unsafe { self.attributes.0.SetGUID(key, value) }
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> Result<()> {
        unsafe { self.attributes.0.SetString(key, *value) }
    }
    fn SetBlob(&self, key: *const GUID, buf: *const u8, size: u32) -> Result<()> {
        if buf.is_null() && size > 0 {
            return Err(E_POINTER.into());
        }
        // SAFETY: the caller provides `size` bytes at `buf`.
        let slice = if size == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(buf, size as usize) }
        };
        unsafe { self.attributes.0.SetBlob(key, slice) }
    }
    fn SetUnknown(&self, key: *const GUID, unknown: Ref<IUnknown>) -> Result<()> {
        unsafe { self.attributes.0.SetUnknown(key, unknown.as_ref()) }
    }
    fn LockStore(&self) -> Result<()> {
        unsafe { self.attributes.0.LockStore() }
    }
    fn UnlockStore(&self) -> Result<()> {
        unsafe { self.attributes.0.UnlockStore() }
    }
    fn GetCount(&self) -> Result<u32> {
        unsafe { self.attributes.0.GetCount() }
    }
    fn GetItemByIndex(&self, index: u32, key: *mut GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe {
            self.attributes
                .0
                .GetItemByIndex(index, key, (!value.is_null()).then_some(value))
        }
    }
    fn CopyAllItems(&self, dest: Ref<IMFAttributes>) -> Result<()> {
        unsafe { self.attributes.0.CopyAllItems(dest.as_ref()) }
    }
}
