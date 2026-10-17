//! Diagnostics: creates the virtual camera, lists video capture devices and asks Frame Server
//! for the camera's media source.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(windows)]
fn main() {
    use estream_vcam::control::{ComApartment, VirtualCamera};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::CoTaskMemFree;

    let registered = estream_vcam::control::registered_path();
    println!(
        "registered: {registered:?} exists {:?}; registration {:?}; tag {}",
        registered.as_ref().map(|p| p.exists()),
        estream_vcam::control::registration(),
        estream_vcam::build_tag()
    );
    if std::env::args().any(|a| a == "--registration") {
        return;
    }
    let _com = ComApartment::multithreaded();
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
    let names = || -> Vec<String> {
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 1).unwrap();
            let attrs = attrs.unwrap();
            attrs
                .SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                )
                .unwrap();
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFEnumDeviceSources(&attrs, &mut list, &mut count).unwrap();
            let mut out = Vec::new();
            for i in 0..count as usize {
                if let Some(a) = (*list.add(i)).take() {
                    let mut name = windows::core::PWSTR::null();
                    let mut len = 0;
                    if a.GetAllocatedString(
                        &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                        &mut name,
                        &mut len,
                    )
                    .is_ok()
                    {
                        out.push(name.to_string().unwrap_or_default());
                        CoTaskMemFree(Some(name.0 as *const _));
                    }
                }
            }
            CoTaskMemFree(Some(list as *const _));
            out
        }
    };
    println!("before: {:?}", names());
    let camera = match VirtualCamera::start() {
        Ok(c) => c,
        Err(e) => {
            println!("start failed: {e} ({:?})", e.code());
            return;
        }
    };
    println!("camera started");
    for i in 0..6 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        println!("{i}s: {:?}", names());
    }
    match camera.media_source() {
        Ok(_) => println!("GetMediaSource: ok"),
        Err(e) => println!("GetMediaSource failed: {e} ({:?})", e.code()),
    }
}

#[cfg(not(windows))]
fn main() {}
