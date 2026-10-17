//! The installed virtual camera, end to end through Windows' Frame Server.
//!
//! Needs the camera source registered once (`regsvr32 target\debug\estream_vcam.dll` from an
//! elevated prompt, or Start virtual camera in eStream). Run with
//! `cargo test -p estream-vcam --test system -- --ignored`.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_vcam::FRIENDLY_NAME;
use estream_vcam::control::{self, ComApartment, Registration, VirtualCamera};
use estream_vcam::shared::{MAPPING_NAME, Shared, fill_nv12};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

/// The camera's activation object, once Windows lists it.
fn find_camera() -> Option<IMFActivate> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).ok()?;
        let attrs = attrs?;
        attrs
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .ok()?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attrs, &mut list, &mut count).ok()?;
        let mut found = None;
        for i in 0..count as usize {
            let Some(activate) = (*list.add(i)).take() else {
                continue;
            };
            let mut name = windows::core::PWSTR::null();
            let mut len = 0;
            if activate
                .GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut name, &mut len)
                .is_ok()
            {
                let text = name.to_string().unwrap_or_default();
                CoTaskMemFree(Some(name.0 as *const _));
                // Windows appends " (Windows Virtual Camera)".
                if text.starts_with(FRIENDLY_NAME) {
                    found = Some(activate);
                }
            }
        }
        CoTaskMemFree(Some(list as *const _));
        found
    }
}

#[test]
#[ignore = "needs the camera source installed (one-time elevated regsvr32)"]
fn camera_streams_through_frame_server() {
    let _com = ComApartment::multithreaded();
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
    assert_eq!(
        control::registration(),
        Registration::Installed,
        "register target\\debug\\estream_vcam.dll first (elevated regsvr32)"
    );

    let camera = VirtualCamera::start().expect("MFCreateVirtualCamera");

    // Play eStream: send magenta-ish frames at whatever size the source asks for.
    let stop = Arc::new(AtomicBool::new(false));
    let producer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut shared = None;
            while !stop.load(Ordering::Acquire) {
                if shared.is_none() {
                    shared = Shared::open(MAPPING_NAME).ok();
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

    // The device shows up asynchronously.
    let deadline = Instant::now() + Duration::from_secs(15);
    let activate = loop {
        if let Some(a) = find_camera() {
            break a;
        }
        assert!(
            Instant::now() < deadline,
            "“{FRIENDLY_NAME}” never appeared"
        );
        std::thread::sleep(Duration::from_millis(250));
    };

    let source: IMFMediaSource = unsafe { activate.ActivateObject() }.expect("open the camera");
    let reader = unsafe { MFCreateSourceReaderFromMediaSource(&source, None) }.unwrap();
    // Ask for 1280×720 like most call apps.
    let mut chosen = None;
    for i in 0.. {
        let Ok(t) = (unsafe { reader.GetNativeMediaType(STREAM, i) }) else {
            break;
        };
        let size = unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap();
        if size == (1280u64 << 32) | 720 {
            chosen = Some(t);
            break;
        }
    }
    let chosen = chosen.expect("a 1280×720 format");
    unsafe { reader.SetCurrentMediaType(STREAM, None, &chosen) }.unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut frames, mut matched) = (0, false);
    while Instant::now() < deadline && !matched {
        let mut flags = 0u32;
        let mut sample = None;
        unsafe { reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .unwrap();
        let Some(sample) = sample else { continue };
        frames += 1;
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().unwrap();
            let mut data = std::ptr::null_mut();
            let mut len = 0;
            buffer.Lock(&mut data, None, Some(&mut len)).unwrap();
            let y = *data;
            buffer.Unlock().unwrap();
            matched = y == 105;
        }
    }
    stop.store(true, Ordering::Release);
    producer.join().unwrap();
    assert!(
        matched,
        "{frames} frames read, none carried eStream's picture"
    );

    drop(reader);
    unsafe {
        let _ = source.Shutdown();
        let _ = activate.ShutdownObject();
    }
    drop(camera);
    assert!(
        (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(250));
            find_camera().is_none()
        }),
        "the camera is removed with eStream"
    );
}
