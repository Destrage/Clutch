//! DirectShow virtual camera source filter for Windows 10.
//!
//! This implements the minimum DirectShow interfaces (`IBaseFilter`, `IPin`,
//! `IAMStreamConfig`, `IKsPropertySet`) needed for apps to
//! recognise a video capture device. Frames come from the same shared-memory protocol
//! used by the Media Foundation source ([`crate::shared`]), under
//! [`crate::shared::DSHOW_MAPPING_NAME`] since the filter runs inside the user's app.
//!
//! The filter is registered under `CLSID_VideoInputDeviceCategory` so that apps such as
//! Zoom, Teams, Discord, Meet etc. discover it alongside real cameras. Only on Windows 10:
//! on Windows 11 the Media Foundation camera is listed instead, and both would show twice.

use crate::shared::{Shared, fill_nv12, fit_nv12, nv12_len};
use crate::source::FORMATS;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::BITMAPINFOHEADER;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::IReferenceClock;
use windows::Win32::Media::KernelStreaming::{IKsPropertySet, IKsPropertySet_Impl};
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, CLSID_MemoryAllocator, VIDEOINFOHEADER,
};
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::core::*;

// ── Agile Wrapper for COM Pointers ───────────────────────────────────────────

struct Agile<T>(T);
unsafe impl<T> Send for Agile<T> {}
unsafe impl<T> Sync for Agile<T> {}

// ── GUIDs ────────────────────────────────────────────────────────────────────

/// CLSID for the DirectShow filter — distinct from the MF source's CLSID.
pub const DSHOW_CLSID: GUID = GUID::from_u128(0x7e3a1f92_5b4c_4d8e_a1c6_9f0e2d7b3a84);
pub const DSHOW_CLSID_STRING: &str = "{7E3A1F92-5B4C-4D8E-A1C6-9F0E2D7B3A84}";

// NV12 fourcc
const NV12_FOURCC: u32 = u32::from_le_bytes(*b"NV12");
const FPS: u32 = 30;
const FRAME_DURATION_100NS: i64 = 10_000_000 / FPS as i64;

/// DirectShow `MEDIATYPE_Video`.
const MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
/// NV12 media subtype.
const MEDIASUBTYPE_NV12: GUID = GUID::from_u128(0x3231564e_0000_0010_8000_00aa00389b71);
/// `FORMAT_VideoInfo`.
const FORMAT_VIDEOINFO: GUID = GUID::from_u128(0x05589f80_c356_11ce_bf01_00aa0055595a);

// ── Registration ─────────────────────────────────────────────────────────────

fn dshow_class_key() -> String {
    format!("SOFTWARE\\Classes\\CLSID\\{DSHOW_CLSID_STRING}")
}

fn filter_category_key() -> String {
    // CLSID_VideoInputDeviceCategory = {860BB310-5D01-11D0-BD3B-00A0C911CE86}
    format!(
        "SOFTWARE\\Classes\\CLSID\\{{860BB310-5D01-11D0-BD3B-00A0C911CE86}}\\Instance\\{DSHOW_CLSID_STRING}"
    )
}

/// Registers the DirectShow filter under the video capture device category.
/// Called from `DllRegisterServer` after the MF source is registered.
pub fn register(dll_path: &str) -> bool {
    let class = create_key(&dshow_class_key());
    let class = match class {
        Some(k) => k,
        None => return false,
    };
    let mut ok = set_string(class, None, crate::FRIENDLY_NAME);
    // SAFETY: closes the key we opened.
    unsafe {
        let _ = RegCloseKey(class);
    }

    let server = create_key(&format!("{}\\InprocServer32", dshow_class_key()));
    let server = match server {
        Some(k) => k,
        None => return false,
    };
    ok = ok
        && set_string(server, None, dll_path)
        && set_string(server, Some("ThreadingModel"), "Both");
    // SAFETY: closes the key we opened.
    unsafe {
        let _ = RegCloseKey(server);
    }

    // Register under CLSID_VideoInputDeviceCategory so apps find us.
    let cat = create_key(&filter_category_key());
    let cat = match cat {
        Some(k) => k,
        None => return ok,
    };
    ok = ok
        && set_string(cat, Some("CLSID"), DSHOW_CLSID_STRING)
        && set_string(cat, Some("FriendlyName"), crate::FRIENDLY_NAME);
    // SAFETY: closes the key we opened.
    unsafe {
        let _ = RegCloseKey(cat);
    }

    ok
}

/// Unregisters the DirectShow filter.
pub fn unregister() {
    // SAFETY: deletes our class key and category instance.
    unsafe {
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(dshow_class_key()));
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(filter_category_key()));
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn set_string(key: HKEY, name: Option<&str>, value: &str) -> bool {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the byte view covers the UTF-16 string including its terminator.
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let name = name.map(HSTRING::from);
    // SAFETY: valid open key and data.
    unsafe {
        RegSetValueExW(
            key,
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            None,
            REG_SZ,
            Some(bytes),
        ) == ERROR_SUCCESS
    }
}

fn create_key(path: &str) -> Option<HKEY> {
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under HKLM.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    (status == ERROR_SUCCESS).then_some(key)
}

fn debug(message: &str) {
    let text = HSTRING::from(format!("estream-vcam-dshow: {message}\n"));
    // SAFETY: valid null-terminated string.
    unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(&text) };
    crate::log(message);
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ── Shared filter state ──────────────────────────────────────────────────────

/// State shared between the filter, pin, and delivery thread.
struct FilterInner {
    state: Mutex<FILTER_STATE>,
    /// The filter itself, for `QueryPinInfo` (weak: the filter owns the pin).
    filter: Mutex<Weak<IBaseFilter>>,
    graph: Mutex<Option<Agile<IFilterGraph>>>,
    pin: Mutex<Option<Agile<IPin>>>,
    allocator: Mutex<Option<Agile<IMemAllocator>>>,
    format: Mutex<(u32, u32)>,
    mapping_name: String,
    frames: Mutex<Frames>,
    stop: AtomicBool,
    wake: Condvar,
    ref_time_base: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct Frames {
    shared: Option<Shared>,
    retry_at: Option<Instant>,
    picture: Vec<u8>,
    scratch: Vec<u8>,
    last_seq: u64,
}

// ── DShowFilter ──────────────────────────────────────────────────────────────

/// The DirectShow source filter. Implements `IBaseFilter`.
#[implement(IBaseFilter)]
pub struct DShowFilter {
    inner: Arc<FilterInner>,
    /// The one output pin: apps compare pin pointers, so it keeps a single identity.
    pin: IPin,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// DirectShow COM objects alive (filters and pins), so the DLL stays loaded while apps use them.
static LIVE: AtomicU32 = AtomicU32::new(0);

impl DShowFilter {
    /// Creates the filter as a COM object, wired so its pin can report it.
    pub fn create(mapping: &str) -> IBaseFilter {
        let object = ComObject::new(Self::new(mapping));
        let filter: IBaseFilter = object.to_interface();
        if let Ok(weak) = filter.downgrade() {
            *lock(&object.inner.filter) = weak;
        }
        filter
    }

    fn new(mapping: &str) -> Self {
        LIVE.fetch_add(1, Ordering::AcqRel);
        let inner = Arc::new(FilterInner {
            state: Mutex::new(FILTER_STATE(0)), // State_Stopped
            filter: Mutex::new(Weak::new()),
            graph: Mutex::new(None),
            pin: Mutex::new(None),
            allocator: Mutex::new(None),
            format: Mutex::new(FORMATS[0]),
            mapping_name: mapping.to_owned(),
            frames: Mutex::new(Frames::default()),
            stop: AtomicBool::new(true),
            wake: Condvar::new(),
            ref_time_base: Mutex::new(None),
        });
        LIVE.fetch_add(1, Ordering::AcqRel);
        let pin = ComObject::new(DShowPin {
            inner: inner.clone(),
        })
        .into_interface();
        Self {
            inner,
            pin,
            worker: Mutex::new(None),
        }
    }
}

impl Drop for DShowFilter {
    fn drop(&mut self) {
        // An app may release the filter without stopping it: end the delivery thread before
        // the DLL can unload.
        self.inner.stop.store(true, Ordering::Release);
        if let Some(w) = lock(&self.worker).take() {
            let _ = w.join();
        }
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn dshow_objects_alive() -> bool {
    LIVE.load(Ordering::Acquire) > 0
}

// ── IBaseFilter & IMediaFilter ───────────────────────────────────────────────

impl IPersist_Impl for DShowFilter_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        Ok(DSHOW_CLSID)
    }
}

impl IMediaFilter_Impl for DShowFilter_Impl {
    fn Stop(&self) -> Result<()> {
        debug("DShow filter Stop");
        *lock(&self.inner.state) = FILTER_STATE(0);
        self.inner.stop.store(true, Ordering::Release);
        self.inner.wake.notify_all();
        if let Some(w) = lock(&self.worker).take() {
            let _ = w.join();
        }
        Ok(())
    }

    fn Pause(&self) -> Result<()> {
        debug("DShow filter Pause");
        *lock(&self.inner.state) = FILTER_STATE(1);
        Ok(())
    }

    fn Run(&self, _start: i64) -> Result<()> {
        debug("DShow filter Run");
        *lock(&self.inner.state) = FILTER_STATE(2);
        *lock(&self.inner.ref_time_base) = Some(Instant::now());
        self.inner.stop.store(false, Ordering::Release);

        let mut worker = lock(&self.worker);
        if worker.as_ref().is_none_or(|w| w.is_finished()) {
            let inner = self.inner.clone();
            *worker = std::thread::Builder::new()
                .name("estream-vcam-dshow".into())
                .spawn(move || deliver_loop(inner))
                .ok();
        }
        Ok(())
    }

    fn GetState(&self, _timeout_ms: u32) -> Result<FILTER_STATE> {
        Ok(*lock(&self.inner.state))
    }

    fn SetSyncSource(&self, _clock: Ref<'_, IReferenceClock>) -> Result<()> {
        Ok(())
    }

    fn GetSyncSource(&self) -> Result<IReferenceClock> {
        Err(windows::core::Error::from(E_NOINTERFACE))
    }
}

impl IBaseFilter_Impl for DShowFilter_Impl {
    fn EnumPins(&self) -> Result<IEnumPins> {
        Ok(ComObject::new(PinEnum {
            pin: self.pin.clone(),
            pos: AtomicU32::new(0),
        })
        .into_interface())
    }

    fn FindPin(&self, id: &PCWSTR) -> Result<IPin> {
        let name = unsafe { id.to_string() }.unwrap_or_default();
        if name == "Output" || name == "Capture" {
            Ok(self.pin.clone())
        } else {
            Err(VFW_E_NOT_FOUND.into())
        }
    }

    #[allow(
        clippy::not_unsafe_ptr_arg_deref,
        reason = "COM signature, checked for null"
    )]
    fn QueryFilterInfo(&self, info: *mut FILTER_INFO) -> Result<()> {
        // SAFETY: COM passes a valid structure or null.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return Err(E_POINTER.into());
        };
        let name: Vec<u16> = crate::FRIENDLY_NAME.encode_utf16().chain(Some(0)).collect();
        let len = name.len().min(128);
        info.achName[..len].copy_from_slice(&name[..len]);
        info.pGraph =
            core::mem::ManuallyDrop::new(lock(&self.inner.graph).as_ref().map(|g| g.0.clone()));
        Ok(())
    }

    fn JoinFilterGraph(&self, graph: Ref<'_, IFilterGraph>, _name: &PCWSTR) -> Result<()> {
        *lock(&self.inner.graph) = graph.cloned().map(Agile);
        Ok(())
    }

    fn QueryVendorInfo(&self) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
}

// ── DShowPin ─────────────────────────────────────────────────────────────────

/// The single output pin of our DirectShow source filter.
#[implement(IPin, IAMStreamConfig, IKsPropertySet)]
pub(crate) struct DShowPin {
    inner: Arc<FilterInner>,
}

impl Drop for DShowPin {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

impl IPin_Impl for DShowPin_Impl {
    fn Connect(&self, receive_pin: Ref<'_, IPin>, mt: *const AM_MEDIA_TYPE) -> Result<()> {
        let receive_pin = receive_pin.ok()?;

        if !mt.is_null() && unsafe { (*mt).majortype } != MEDIATYPE_VIDEO {
            return Err(VFW_E_TYPE_NOT_ACCEPTED.into());
        }
        let (w, h) = *lock(&self.inner.format);
        let proposed = build_media_type_for_size(w, h);
        let received =
            unsafe { receive_pin.ReceiveConnection(&self.to_interface::<IPin>(), &proposed) };
        // SAFETY: the downstream pin keeps its own copy of the format block.
        unsafe { CoTaskMemFree(Some(proposed.pbFormat.cast())) };
        received?;

        *lock(&self.inner.pin) = Some(Agile(receive_pin.clone()));

        let input: IMemInputPin = receive_pin.cast()?;
        let allocator: Option<IMemAllocator> = unsafe {
            input.GetAllocator().ok().or_else(|| {
                CoCreateInstance::<Option<&IUnknown>, IMemAllocator>(
                    &CLSID_MemoryAllocator,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .ok()
            })
        };

        if let Some(ref alloc) = allocator {
            let (w, h) = *lock(&self.inner.format);
            let buf_size = nv12_len(w, h);
            let props = ALLOCATOR_PROPERTIES {
                cBuffers: 4,
                cbBuffer: buf_size as i32,
                cbAlign: 1,
                cbPrefix: 0,
            };
            unsafe {
                let _ = alloc.SetProperties(&props)?;
                input.NotifyAllocator(alloc, false)?;
            }
        }
        *lock(&self.inner.allocator) = allocator.map(Agile);

        Ok(())
    }

    fn ReceiveConnection(&self, _pin: Ref<'_, IPin>, _mt: *const AM_MEDIA_TYPE) -> Result<()> {
        Err(E_UNEXPECTED.into())
    }

    fn Disconnect(&self) -> Result<()> {
        *lock(&self.inner.pin) = None;
        *lock(&self.inner.allocator) = None;
        Ok(())
    }

    fn ConnectedTo(&self) -> Result<IPin> {
        lock(&self.inner.pin)
            .as_ref()
            .map(|p| p.0.clone())
            .ok_or(VFW_E_NOT_CONNECTED.into())
    }

    fn ConnectionMediaType(&self, mt: *mut AM_MEDIA_TYPE) -> Result<()> {
        if mt.is_null() {
            return Err(E_POINTER.into());
        }
        let (w, h) = *lock(&self.inner.format);
        unsafe { *mt = build_media_type_for_size(w, h) };
        Ok(())
    }

    fn QueryPinInfo(&self, info: *mut PIN_INFO) -> Result<()> {
        // SAFETY: COM passes a valid structure or null.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return Err(E_POINTER.into());
        };
        info.dir = PINDIR_OUTPUT;
        let name: Vec<u16> = "Capture\0".encode_utf16().collect();
        let len = name.len().min(128);
        info.achName[..len].copy_from_slice(&name[..len]);
        // An owned reference to the filter, as `QueryPinInfo` requires.
        info.pFilter = core::mem::ManuallyDrop::new(lock(&self.inner.filter).upgrade());
        Ok(())
    }

    fn QueryDirection(&self) -> Result<PIN_DIRECTION> {
        Ok(PINDIR_OUTPUT)
    }

    fn QueryId(&self) -> Result<PWSTR> {
        let id = "Capture\0";
        let wide: Vec<u16> = id.encode_utf16().collect();
        let ptr = unsafe { CoTaskMemAlloc(wide.len() * 2) } as *mut u16;
        if ptr.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        unsafe { std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len()) };
        Ok(PWSTR(ptr))
    }

    fn QueryAccept(&self, mt: *const AM_MEDIA_TYPE) -> HRESULT {
        if mt.is_null() {
            return E_POINTER;
        }
        let mt = unsafe { &*mt };
        if mt.majortype == MEDIATYPE_VIDEO {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn EnumMediaTypes(&self) -> Result<IEnumMediaTypes> {
        Ok(ComObject::new(MediaTypeEnum {
            format: *lock(&self.inner.format),
            pos: AtomicU32::new(0),
        })
        .into_interface())
    }

    fn QueryInternalConnections(&self, _pins: OutRef<'_, IPin>, count: *mut u32) -> Result<()> {
        if count.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *count = 0 };
        Ok(())
    }

    fn EndOfStream(&self) -> Result<()> {
        Ok(())
    }

    fn BeginFlush(&self) -> Result<()> {
        Ok(())
    }

    fn EndFlush(&self) -> Result<()> {
        Ok(())
    }

    fn NewSegment(&self, _start: i64, _stop: i64, _rate: f64) -> Result<()> {
        Ok(())
    }
}

// ── IAMStreamConfig ──────────────────────────────────────────────────────────

impl IAMStreamConfig_Impl for DShowPin_Impl {
    fn SetFormat(&self, mt: *const AM_MEDIA_TYPE) -> Result<()> {
        if mt.is_null() {
            return Err(E_POINTER.into());
        }
        let mt = unsafe { &*mt };
        if mt.majortype != MEDIATYPE_VIDEO
            || mt.cbFormat < std::mem::size_of::<VIDEOINFOHEADER>() as u32
        {
            return Err(VFW_E_INVALIDMEDIATYPE.into());
        }
        let vih = unsafe { &*(mt.pbFormat as *const VIDEOINFOHEADER) };
        let w = vih.bmiHeader.biWidth as u32;
        let h = vih.bmiHeader.biHeight.unsigned_abs();
        if FORMATS.contains(&(w, h)) {
            *lock(&self.inner.format) = (w, h);
            Ok(())
        } else {
            Err(VFW_E_INVALIDMEDIATYPE.into())
        }
    }

    fn GetFormat(&self) -> Result<*mut AM_MEDIA_TYPE> {
        let (w, h) = *lock(&self.inner.format);
        let mt = build_media_type_for_size(w, h);
        let ptr =
            unsafe { CoTaskMemAlloc(std::mem::size_of::<AM_MEDIA_TYPE>()) } as *mut AM_MEDIA_TYPE;
        if ptr.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        unsafe { *ptr = mt };
        Ok(ptr)
    }

    fn GetNumberOfCapabilities(&self, picount: *mut i32, pisize: *mut i32) -> Result<()> {
        if picount.is_null() || pisize.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *picount = FORMATS.len() as i32;
            *pisize = std::mem::size_of::<VIDEO_STREAM_CONFIG_CAPS>() as i32;
        }
        Ok(())
    }

    fn GetStreamCaps(&self, index: i32, mt: *mut *mut AM_MEDIA_TYPE, scc: *mut u8) -> Result<()> {
        if index < 0 || index as usize >= FORMATS.len() {
            return Err(S_FALSE.into());
        }
        if mt.is_null() {
            return Err(E_POINTER.into());
        }
        let (w, h) = FORMATS[index as usize];

        let p =
            unsafe { CoTaskMemAlloc(std::mem::size_of::<AM_MEDIA_TYPE>()) } as *mut AM_MEDIA_TYPE;
        if p.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        unsafe { *p = build_media_type_for_size(w, h) };
        unsafe { *mt = p };

        if !scc.is_null() {
            let caps = VIDEO_STREAM_CONFIG_CAPS {
                guid: MEDIATYPE_VIDEO,
                VideoStandard: 0,
                InputSize: SIZE {
                    cx: w as i32,
                    cy: h as i32,
                },
                MinCroppingSize: SIZE {
                    cx: w as i32,
                    cy: h as i32,
                },
                MaxCroppingSize: SIZE {
                    cx: w as i32,
                    cy: h as i32,
                },
                CropGranularityX: 1,
                CropGranularityY: 1,
                CropAlignX: 1,
                CropAlignY: 1,
                MinOutputSize: SIZE {
                    cx: w as i32,
                    cy: h as i32,
                },
                MaxOutputSize: SIZE {
                    cx: w as i32,
                    cy: h as i32,
                },
                OutputGranularityX: 1,
                OutputGranularityY: 1,
                StretchTapsX: 0,
                StretchTapsY: 0,
                ShrinkTapsX: 0,
                ShrinkTapsY: 0,
                MinFrameInterval: FRAME_DURATION_100NS,
                MaxFrameInterval: FRAME_DURATION_100NS,
                MinBitsPerSecond: nv12_len(w, h) as i32 * 8 * FPS as i32,
                MaxBitsPerSecond: nv12_len(w, h) as i32 * 8 * FPS as i32,
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &caps as *const _ as *const u8,
                    scc,
                    std::mem::size_of::<VIDEO_STREAM_CONFIG_CAPS>(),
                );
            }
        }

        Ok(())
    }
}

// ── IKsPropertySet ───────────────────────────────────────────────────────────

impl IKsPropertySet_Impl for DShowPin_Impl {
    fn Set(
        &self,
        _guidpropset: *const GUID,
        _dwpropid: u32,
        _pinstancedata: *const core::ffi::c_void,
        _cbinstancedata: u32,
        _ppropdata: *const core::ffi::c_void,
        _cbpropdata: u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Get(
        &self,
        guidpropset: *const GUID,
        dwpropid: u32,
        _pinstancedata: *const core::ffi::c_void,
        _cbinstancedata: u32,
        ppropdata: *mut core::ffi::c_void,
        cbpropdata: u32,
        pcbreturned: *mut u32,
    ) -> Result<()> {
        const AMPROPSETID_PIN: GUID = GUID::from_u128(0x9b00f101_1567_11d1_b3f1_00aa003761c5);
        const PIN_CATEGORY_CAPTURE: GUID = GUID::from_u128(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);

        if guidpropset.is_null() {
            return Err(E_POINTER.into());
        }
        let set = unsafe { *guidpropset };
        if set == AMPROPSETID_PIN && dwpropid == 0 {
            if cbpropdata >= 16 && !ppropdata.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        &PIN_CATEGORY_CAPTURE as *const _ as *const u8,
                        ppropdata.cast::<u8>(),
                        16,
                    );
                }
                if !pcbreturned.is_null() {
                    unsafe { *pcbreturned = 16 };
                }
                Ok(())
            } else {
                Err(E_UNEXPECTED.into())
            }
        } else {
            Err(E_NOTIMPL.into())
        }
    }

    fn QuerySupported(&self, guidpropset: *const GUID, dwpropid: u32) -> Result<u32> {
        const AMPROPSETID_PIN: GUID = GUID::from_u128(0x9b00f101_1567_11d1_b3f1_00aa003761c5);
        const KSPROPERTY_SUPPORT_GET: u32 = 1;

        if guidpropset.is_null() {
            return Err(E_POINTER.into());
        }
        let set = unsafe { *guidpropset };
        if set == AMPROPSETID_PIN && dwpropid == 0 {
            Ok(KSPROPERTY_SUPPORT_GET)
        } else {
            Err(E_NOTIMPL.into())
        }
    }
}

// ── Media type helpers ───────────────────────────────────────────────────────

fn build_media_type_for_size(w: u32, h: u32) -> AM_MEDIA_TYPE {
    let vih_size = std::mem::size_of::<VIDEOINFOHEADER>();
    let vih_ptr = unsafe { CoTaskMemAlloc(vih_size) } as *mut VIDEOINFOHEADER;
    if !vih_ptr.is_null() {
        unsafe {
            std::ptr::write_bytes(vih_ptr, 0, 1);
            let vih = &mut *vih_ptr;
            vih.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            vih.bmiHeader.biWidth = w as i32;
            vih.bmiHeader.biHeight = h as i32;
            vih.bmiHeader.biPlanes = 1;
            vih.bmiHeader.biBitCount = 12; // NV12 = 12 bits per pixel
            vih.bmiHeader.biCompression = NV12_FOURCC;
            vih.bmiHeader.biSizeImage = nv12_len(w, h) as u32;
            vih.AvgTimePerFrame = FRAME_DURATION_100NS;
            vih.rcSource = RECT {
                left: 0,
                top: 0,
                right: w as i32,
                bottom: h as i32,
            };
            vih.rcTarget = vih.rcSource;
        }
    }

    AM_MEDIA_TYPE {
        majortype: MEDIATYPE_VIDEO,
        subtype: MEDIASUBTYPE_NV12,
        bFixedSizeSamples: BOOL(1),
        bTemporalCompression: BOOL(0),
        lSampleSize: nv12_len(w, h) as u32,
        formattype: FORMAT_VIDEOINFO,
        pUnk: core::mem::ManuallyDrop::new(None),
        cbFormat: vih_size as u32,
        pbFormat: vih_ptr as *mut u8,
    }
}

// ── Delivery thread ──────────────────────────────────────────────────────────

fn deliver_loop(inner: Arc<FilterInner>) {
    let interval = Duration::from_nanos(1_000_000_000 / FPS as u64);
    let mut next = Instant::now();

    loop {
        if inner.stop.load(Ordering::Acquire) {
            break;
        }

        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next = next.max(Instant::now()) + interval;

        if inner.stop.load(Ordering::Acquire) {
            break;
        }

        let allocator = lock(&inner.allocator).as_ref().map(|a| a.0.clone());
        let downstream = lock(&inner.pin).as_ref().map(|p| p.0.clone());
        let (allocator, downstream) = match (allocator, downstream) {
            (Some(a), Some(d)) => (a, d),
            _ => continue,
        };

        let input: std::result::Result<IMemInputPin, _> = downstream.cast();
        let input = match input {
            Ok(i) => i,
            Err(_) => continue,
        };

        let _ = unsafe { allocator.Commit() };

        let mut sample_opt: Option<IMediaSample> = None;
        let res = unsafe { allocator.GetBuffer(&mut sample_opt, None, None, 0) };
        let Some(sample) = (if res.is_ok() { sample_opt } else { None }) else {
            continue;
        };

        let (w, h) = *lock(&inner.format);
        refresh_frame(&inner, w, h);

        let frames = lock(&inner.frames);
        unsafe {
            if let Ok(ptr) = sample.GetPointer()
                && !ptr.is_null()
            {
                let len = nv12_len(w, h);
                let buf_len = sample.GetSize();
                let copy_len = len.min(buf_len as usize);
                if frames.picture.len() >= copy_len {
                    std::ptr::copy_nonoverlapping(frames.picture.as_ptr(), ptr, copy_len);
                }
                let _ = sample.SetActualDataLength(copy_len as i32);
            }
        }
        drop(frames);

        let base = lock(&inner.ref_time_base).unwrap_or_else(Instant::now);
        let elapsed = Instant::now().duration_since(base);
        let time_100ns = elapsed.as_nanos() as i64 / 100;
        let end = time_100ns + FRAME_DURATION_100NS;
        unsafe {
            let _ = sample.SetTime(Some(&time_100ns), Some(&end));
            let _ = sample.SetSyncPoint(true);
        }

        let _ = unsafe { input.Receive(&sample) };
    }

    if let Some(alloc) = lock(&inner.allocator).as_ref() {
        let _ = unsafe { alloc.0.Decommit() };
    }
}

fn refresh_frame(inner: &FilterInner, w: u32, h: u32) {
    let mut frames = lock(&inner.frames);
    let frames_ref = &mut *frames;
    if frames_ref.picture.len() != nv12_len(w, h) {
        frames_ref.picture = vec![0; nv12_len(w, h)];
        fill_nv12(&mut frames_ref.picture, w, h, 28, 128, 128);
    }
    if frames_ref.shared.is_none() && frames_ref.retry_at.is_none_or(|t| Instant::now() >= t) {
        match Shared::create(&inner.mapping_name) {
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
            fill_nv12(&mut frames_ref.picture, w, h, 28, 128, 128);
            frames_ref.last_seq = 0;
        }
    }
}

// ── Pin enumerator ───────────────────────────────────────────────────────────

#[implement(IEnumPins)]
struct PinEnum {
    pin: IPin,
    pos: AtomicU32,
}

impl IEnumPins_Impl for PinEnum_Impl {
    fn Next(&self, count: u32, pins: *mut Option<IPin>, fetched: *mut u32) -> HRESULT {
        let pos = self.pos.load(Ordering::Acquire);
        if pos >= 1 || count == 0 {
            if !fetched.is_null() {
                unsafe { *fetched = 0 };
            }
            return S_FALSE;
        }
        unsafe {
            *pins = Some(self.pin.clone());
        }
        self.pos.store(pos + 1, Ordering::Release);
        if !fetched.is_null() {
            unsafe { *fetched = 1 };
        }
        if count == 1 { S_OK } else { S_FALSE }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let old = self.pos.fetch_add(count, Ordering::AcqRel);
        if old + count <= 1 {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }

    fn Reset(&self) -> Result<()> {
        self.pos.store(0, Ordering::Release);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumPins> {
        Ok(ComObject::new(PinEnum {
            pin: self.pin.clone(),
            pos: AtomicU32::new(self.pos.load(Ordering::Acquire)),
        })
        .into_interface())
    }
}

// ── Media type enumerator ────────────────────────────────────────────────────

#[implement(IEnumMediaTypes)]
struct MediaTypeEnum {
    format: (u32, u32),
    pos: AtomicU32,
}

impl IEnumMediaTypes_Impl for MediaTypeEnum_Impl {
    fn Next(&self, count: u32, types: *mut *mut AM_MEDIA_TYPE, fetched: *mut u32) -> HRESULT {
        let pos = self.pos.load(Ordering::Acquire) as usize;
        if pos >= FORMATS.len() || count == 0 {
            if !fetched.is_null() {
                unsafe { *fetched = 0 };
            }
            return S_FALSE;
        }
        let available = (FORMATS.len() - pos).min(count as usize);
        for i in 0..available {
            let (w, h) = FORMATS[pos + i];
            let mt = build_media_type_for_size(w, h);
            let ptr = unsafe { CoTaskMemAlloc(std::mem::size_of::<AM_MEDIA_TYPE>()) }
                as *mut AM_MEDIA_TYPE;
            if !ptr.is_null() {
                unsafe { *ptr = mt };
            }
            unsafe { *types.add(i) = ptr };
        }
        self.pos.store((pos + available) as u32, Ordering::Release);
        if !fetched.is_null() {
            unsafe { *fetched = available as u32 };
        }
        if available as u32 == count {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let old = self.pos.fetch_add(count, Ordering::AcqRel);
        if (old + count) as usize <= FORMATS.len() {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }

    fn Reset(&self) -> Result<()> {
        self.pos.store(0, Ordering::Release);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumMediaTypes> {
        Ok(ComObject::new(MediaTypeEnum {
            format: self.format,
            pos: AtomicU32::new(self.pos.load(Ordering::Acquire)),
        })
        .into_interface())
    }
}
