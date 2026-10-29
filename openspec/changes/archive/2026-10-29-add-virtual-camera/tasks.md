# Tasks: add-virtual-camera

## 1. Camera source (`crates/estream-vcam`)
- [x] 1.1 Shared memory protocol (header, seqlock, global mapping with DACL) and `fit_nv12`
- [x] 1.2 COM media source: activator, source, stream, allocator, events
- [x] 1.3 DLL exports: class factory, register/unregister (HKLM)
- [x] 1.4 Control: registration check, elevated install, `MFCreateVirtualCamera` session camera

## 2. Output (`estream-media`)
- [x] 2.1 Feed thread: heartbeat-driven consumer, scaling, status

## 3. Studio
- [x] 3.1 Top bar button, palette action, toasts and first-run install
- [x] 3.2 Settings: canvas, install state, reinstall
- [x] 3.3 Remote API: virtual cam requests and `VirtualcamStateChanged`

## 4. Verification
- [x] 4.1 Unit tests (scaling, seqlock)
- [x] 4.2 In-process media source test (events, samples, frames)
- [x] 4.3 System test through Frame Server with an `IMFSourceReader` (after one UAC approval)
- [x] 4.4 In-app check
