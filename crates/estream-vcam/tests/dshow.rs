//! DirectShow filter unit and integration tests.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_vcam::dshow::{DSHOW_CLSID, DShowFilter};
use estream_vcam::shared::{DSHOW_MAPPING_NAME, Shared, fill_nv12};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::System::Com::*;
use windows::core::{Interface, w};

#[test]
fn dshow_filter_initialization_and_pins() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mapping = format!("Local\\estream-dshow-test-{}", std::process::id());
    let base_filter = DShowFilter::create(&mapping);

    // Verify CLSID
    let persist: IPersist = base_filter.cast().unwrap();
    let clsid = unsafe { persist.GetClassID() }.unwrap();
    assert_eq!(clsid, DSHOW_CLSID);

    // Verify pins enumeration
    let pins = unsafe { base_filter.EnumPins() }.unwrap();
    let mut pin_buf = [None];
    let mut fetched = 0;
    let hr = unsafe { pins.Next(&mut pin_buf, Some(&mut fetched)) };
    assert!(hr.is_ok());
    assert_eq!(fetched, 1);
    let pin = pin_buf[0]
        .clone()
        .expect("DirectShow pin should be present");

    // The same pin object every time (apps compare pin pointers).
    let found = unsafe { base_filter.FindPin(w!("Capture")) }.unwrap();
    assert_eq!(found, pin);

    // Verify pin info: direction, and the owning filter
    let mut pin_info = PIN_INFO::default();
    unsafe { pin.QueryPinInfo(&mut pin_info) }.unwrap();
    assert_eq!(pin_info.dir, PINDIR_OUTPUT);
    let owner = core::mem::ManuallyDrop::into_inner(pin_info.pFilter);
    assert_eq!(owner.as_ref(), Some(&base_filter));
    drop(owner);

    // Verify media formats enumeration
    let mt_enum = unsafe { pin.EnumMediaTypes() }.unwrap();
    let mut mt_buf = [std::ptr::null_mut()];
    let mut mt_fetched = 0;
    let mt_hr = unsafe { mt_enum.Next(&mut mt_buf, Some(&mut mt_fetched)) };
    assert!(mt_hr.is_ok());
    assert_eq!(mt_fetched, 1);
    let mt_ptr = mt_buf[0];
    assert!(!mt_ptr.is_null());
    unsafe { CoTaskMemFree(Some(mt_ptr.cast())) };

    // The pin outlives the filter without keeping it alive.
    drop((base_filter, persist));
    let mut pin_info = PIN_INFO::default();
    unsafe { pin.QueryPinInfo(&mut pin_info) }.unwrap();
    assert!(core::mem::ManuallyDrop::into_inner(pin_info.pFilter).is_none());
}

/// Plays Clutch on Windows 10: an app captures the camera through DirectShow (FFmpeg here) and
/// must get the frames Clutch writes, not the placeholder.
#[test]
#[ignore = "needs the DirectShow filter registered (per user is enough) and ffmpeg on PATH"]
fn dshow_camera_delivers_clutch_frames() {
    let stop = Arc::new(AtomicBool::new(false));
    let producer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut shared = None;
            while !stop.load(Ordering::Acquire) {
                if shared.is_none() {
                    shared = Shared::open(DSHOW_MAPPING_NAME).ok();
                }
                if let Some(s) = &shared {
                    let (w, h) = s.wanted();
                    if w > 0 && h > 0 {
                        s.write_frame(w, h, |f| fill_nv12(f, w, h, 105, 212, 234));
                    }
                }
                std::thread::sleep(Duration::from_millis(33));
            }
        })
    };

    // Skip the first second: the filter creates the mapping on its first frame.
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "dshow", "-i"])
        .arg(format!("video={}", estream_vcam::FRIENDLY_NAME))
        .args(["-vf", r"select=gte(n\,30)", "-frames:v", "1"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("run ffmpeg");
    stop.store(true, Ordering::Release);
    producer.join().unwrap();

    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mid = output.stdout.len() / 2 / 3 * 3;
    let (r, g, b) = (
        output.stdout[mid],
        output.stdout[mid + 1],
        output.stdout[mid + 2],
    );
    assert!(r > 200 && g < 80 && b > 200, "got rgb({r}, {g}, {b})");
}
