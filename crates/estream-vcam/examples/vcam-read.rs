//! Reads "Clutch Virtual Camera" like a call app would and saves the last frame as a PNG.
//!
//! `cargo run -p estream-vcam --example vcam-read -- <out.png> [seconds]`
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(windows)]
fn main() {
    use estream_vcam::FRIENDLY_NAME;
    use estream_vcam::control::ComApartment;
    use std::time::{Duration, Instant};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::CoTaskMemFree;

    let out = std::env::args().nth(1).unwrap_or_else(|| "vcam.png".into());
    let secs: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let _com = ComApartment::multithreaded();
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };

    let activate = unsafe {
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
        let mut found = None;
        for i in 0..count as usize {
            let Some(a) = (*list.add(i)).take() else {
                continue;
            };
            let mut name = windows::core::PWSTR::null();
            let mut len = 0;
            if a.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut name, &mut len)
                .is_ok()
            {
                if name
                    .to_string()
                    .unwrap_or_default()
                    .starts_with(FRIENDLY_NAME)
                {
                    found = Some(a);
                }
                CoTaskMemFree(Some(name.0 as *const _));
            }
        }
        CoTaskMemFree(Some(list as *const _));
        found.expect("the virtual camera is on")
    };
    let source: IMFMediaSource = unsafe { activate.ActivateObject() }.unwrap();
    let reader = unsafe { MFCreateSourceReaderFromMediaSource(&source, None) }.unwrap();
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let (w, h) = (1280usize, 720usize);
    for i in 0.. {
        let Ok(t) = (unsafe { reader.GetNativeMediaType(stream, i) }) else {
            break;
        };
        if unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap() == ((w as u64) << 32) | h as u64 {
            unsafe { reader.SetCurrentMediaType(stream, None, &t) }.unwrap();
            break;
        }
    }
    let until = Instant::now() + Duration::from_secs(secs);
    let mut frames = 0;
    let mut last = Vec::new();
    while Instant::now() < until {
        let mut sample = None;
        let mut flags = 0u32;
        unsafe { reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .unwrap();
        let Some(sample) = sample else { continue };
        frames += 1;
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().unwrap();
            let mut data = std::ptr::null_mut();
            let mut len = 0;
            buffer.Lock(&mut data, None, Some(&mut len)).unwrap();
            last = std::slice::from_raw_parts(data, len as usize).to_vec();
            buffer.Unlock().unwrap();
        }
    }
    println!("{frames} frames in {secs} s");
    // NV12 (BT.709, limited) to RGB.
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let luma = last[y * w + x] as f32 - 16.0;
            let c = w * h + (y / 2) * w + (x / 2) * 2;
            let (u, v) = (last[c] as f32 - 128.0, last[c + 1] as f32 - 128.0);
            let px = |f: f32| (f * 1.164).clamp(0.0, 255.0) as u8;
            let i = (y * w + x) * 3;
            rgb[i] = px(luma + 1.793 * v / 1.164);
            rgb[i + 1] = px(luma - (0.213 * v + 0.533 * u) / 1.164);
            rgb[i + 2] = px(luma + 2.112 * u / 1.164);
        }
    }
    let file = std::fs::File::create(&out).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgb)
        .unwrap();
    println!("saved {out}");
    unsafe {
        let _ = source.Shutdown();
    }
}

#[cfg(not(windows))]
fn main() {}
