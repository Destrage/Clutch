# Tasks: add-vcam-windows10

## 1. DirectShow filter
- [x] 1.1 Source filter, output pin, `IAMStreamConfig` and `IKsPropertySet` in `estream-vcam/src/dshow.rs`
- [x] 1.2 Class factory and `DllGetClassObject` for the filter CLSID in `dll.rs`
- [x] 1.3 One pin identity; `QueryPinInfo` reports the owning filter (weak back-reference)
- [x] 1.4 Stop the delivery thread when the filter is released without `Stop`

## 2. Registration and shared memory
- [x] 2.1 Register the filter on Windows 10 only; always clear an earlier registration first
- [x] 2.2 Session-local shared memory (`Local\estream-vcam-dshow-v1`); Clutch writes there when the Media Foundation camera is unavailable

## 3. Verification
- [x] 3.1 Unit test: CLSID, pin enumeration, pin identity, owning filter
- [x] 3.2 Ignored end-to-end test: FFmpeg captures Clutch's frames through the filter (per-user registration)
- [ ] 3.3 Manual: Zoom or Discord on Windows 10 shows the canvas; Windows 11 lists the camera once
