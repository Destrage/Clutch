//! Drives the camera media source in-process, the way Frame Server does.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_vcam::shared::{Shared, fill_nv12};
use estream_vcam::source::{FORMATS, create_source};
use std::time::{Duration, Instant};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{IUnknown, Interface};

fn next_event(generator: &IMFMediaEventGenerator) -> (MF_EVENT_TYPE, PROPVARIANT) {
    let event = unsafe { generator.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) }.unwrap();
    let kind = MF_EVENT_TYPE(unsafe { event.GetType() }.unwrap() as i32);
    assert!(
        unsafe { event.GetStatus() }.unwrap().is_ok(),
        "{kind:?} failed"
    );
    (kind, unsafe { event.GetValue() }.unwrap())
}

fn unknown(value: &PROPVARIANT) -> IUnknown {
    unsafe { (*value.Anonymous.Anonymous.Anonymous.punkVal).clone() }.expect("an object")
}

/// Y of the first pixel and (U, V) of the first chroma pair of a sample.
fn first_pixel(sample: &IMFSample, w: usize, h: usize) -> (u8, u8, u8) {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().unwrap();
        let mut data = std::ptr::null_mut();
        let mut len = 0;
        buffer.Lock(&mut data, None, Some(&mut len)).unwrap();
        assert!(len as usize >= w * h * 3 / 2, "buffer {len} too small");
        let bytes = std::slice::from_raw_parts(data, len as usize);
        let pixel = (bytes[0], bytes[w * h], bytes[w * h + 1]);
        buffer.Unlock().unwrap();
        pixel
    }
}

fn wait_sample(stream: &IMFMediaEventGenerator) -> IMFSample {
    loop {
        let (kind, value) = next_event(stream);
        if kind == MEMediaSample {
            return unknown(&value).cast().unwrap();
        }
    }
}

#[test]
fn source_negotiates_streams_and_delivers_frames() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();
    }
    let mapping = format!("Local\\estream-vcam-it-{}", std::process::id());
    let source = create_source(&mapping).unwrap();
    let generator: IMFMediaEventGenerator = source.cast().unwrap();
    assert_eq!(
        unsafe { source.GetCharacteristics() }.unwrap(),
        MFMEDIASOURCE_IS_LIVE.0 as u32
    );

    // One stream offering every format; pick 1280×720 like most call apps.
    let pd = unsafe { source.CreatePresentationDescriptor() }.unwrap();
    assert_eq!(unsafe { pd.GetStreamDescriptorCount() }.unwrap(), 1);
    let mut selected = Default::default();
    let mut sd = None;
    unsafe { pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd) }.unwrap();
    let handler = unsafe { sd.unwrap().GetMediaTypeHandler() }.unwrap();
    assert_eq!(
        unsafe { handler.GetMediaTypeCount() }.unwrap(),
        FORMATS.len() as u32
    );
    let hd = unsafe { handler.GetMediaTypeByIndex(1) }.unwrap();
    let subtype = unsafe { hd.GetGUID(&MF_MT_SUBTYPE) }.unwrap();
    assert_eq!(subtype, MFVideoFormat_NV12);
    let matrix = unsafe { hd.GetUINT32(&MF_MT_YUV_MATRIX) }.unwrap();
    assert_eq!(
        matrix, MFVideoTransferMatrix_BT709.0 as u32,
        "colors are declared"
    );
    unsafe { handler.SetCurrentMediaType(&hd) }.unwrap();
    let (w, h) = (1280usize, 720usize);

    unsafe { source.Start(&pd, std::ptr::null(), &PROPVARIANT::default()) }.unwrap();
    let (kind, value) = next_event(&generator);
    assert_eq!(kind, MENewStream);
    let stream: IMFMediaStream = unknown(&value).cast().unwrap();
    assert_eq!(next_event(&generator).0, MESourceStarted);
    let stream_events: IMFMediaEventGenerator = stream.cast().unwrap();
    assert_eq!(next_event(&stream_events).0, MEStreamStarted);

    // Nobody is sending frames yet: the dark placeholder.
    unsafe { stream.RequestSample(None) }.unwrap();
    let sample = wait_sample(&stream_events);
    assert_eq!(first_pixel(&sample, w, h), (28, 128, 128));
    assert!(unsafe { sample.GetSampleDuration() }.unwrap() > 0);

    // eStream's side opens the mapping, sees the request size and sends a red frame.
    let producer = Shared::open(&mapping).expect("the source created the mapping");
    assert_eq!(producer.wanted(), (1280, 720));
    assert!(producer.consumer_alive(2000));
    producer.write_frame(1920, 1080, |f| fill_nv12(f, 1920, 1080, 81, 90, 240));
    unsafe { stream.RequestSample(None) }.unwrap();
    let sample = wait_sample(&stream_events);
    assert_eq!(
        first_pixel(&sample, w, h),
        (81, 90, 240),
        "scaled to 1280×720"
    );

    // Samples are paced at 30 fps even when requested all at once.
    let started = Instant::now();
    for _ in 0..10 {
        unsafe { stream.RequestSample(None) }.unwrap();
    }
    for _ in 0..10 {
        wait_sample(&stream_events);
    }
    assert!(
        started.elapsed() >= Duration::from_millis(280),
        "{:?}",
        started.elapsed()
    );

    unsafe { source.Stop() }.unwrap();
    assert_eq!(next_event(&stream_events).0, MEStreamStopped);
    assert_eq!(next_event(&generator).0, MESourceStopped);
    assert!(
        unsafe { stream.RequestSample(None) }.is_err(),
        "stopped streams refuse requests"
    );
    unsafe { source.Shutdown() }.unwrap();
    assert!(unsafe { source.CreatePresentationDescriptor() }.is_err());
}
