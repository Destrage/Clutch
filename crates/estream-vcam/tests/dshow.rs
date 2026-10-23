//! DirectShow filter unit and integration tests.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_vcam::dshow::{DShowFilter, DSHOW_CLSID};
use windows::Win32::Media::DirectShow::*;
use windows::Win32::System::Com::*;
use windows::core::{ComObject, Interface};

#[test]
fn dshow_filter_initialization_and_pins() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mapping = format!("Local\\estream-dshow-test-{}", std::process::id());
    let filter_obj = DShowFilter::new(&mapping);
    let base_filter: IBaseFilter = ComObject::new(filter_obj).into_interface();

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
    let pin = pin_buf[0].clone().expect("DirectShow pin should be present");

    // Verify pin info
    let mut pin_info = PIN_INFO::default();
    unsafe { pin.QueryPinInfo(&mut pin_info) }.unwrap();
    assert_eq!(pin_info.dir, PINDIR_OUTPUT);

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
}
