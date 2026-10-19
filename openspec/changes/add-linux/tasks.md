# Tasks: add-linux

## 1. Build on Linux
- [x] 1.1 Workspace compiles on Linux (WSL Ubuntu): fix `windows`-only items reached without
      `cfg`; FFmpeg 9 LGPL in `third_party/ffmpeg` as on Windows, `.so` files copied next to
      the binaries with an `$ORIGIN` rpath; header/library mismatch detected
- [x] 1.2 CI: an ubuntu job (fmt, clippy, test) next to windows; GPU tests use lavapipe
- [x] 1.3 `estream-gpu`: Vulkan, then GL
- [x] 1.4 Opening files, folders and URLs (`xdg-open`) in one helper used by the app

## 2. Portable building blocks
- [x] 2.1 `estream-shm`: POSIX `shm_open` + `mmap` with the same API; tests run on Linux
- [x] 2.2 Secrets: `keyring` backend with the `keyring:` prefix; session-only without one
- [x] 2.3 `estream-health`: CPU, memory and GPU stats on Linux
- [x] 2.4 `estream-clips`: Steam library paths on Linux
- [x] 2.5 Unknown source kinds load as "Not available on this system" instead of failing

## 3. Capture
- [x] 3.1 ScreenCast portal session (display and window) with restore token in source settings
- [x] 3.2 PipeWire video stream → upload → compositor texture (SHM buffers)
- [x] 3.3 V4L2 camera source: enumeration, formats, YUYV/MJPEG/NV12 upload
- [x] 3.4 PipeWire desktop audio (default sink monitor) and per-app audio
- [x] 3.5 Register Linux source types

## 4. Automation
- [ ] 4.1 `GlobalHotkeys` trait; current Windows listener behind it
- [ ] 4.2 GlobalShortcuts portal, X11 `XGrabKey` fallback, in-app only otherwise
- [ ] 4.3 Focus trigger on X11; "unavailable" on Wayland shown in the macro editor

## 5. Virtual camera and browser source
- [ ] 5.1 Virtual camera via `v4l2loopback` (detects the module, explains how to load it)
- [ ] 5.2 `clutch-browser` (CEF) builds and runs on Linux

## 6. AppImage
- [ ] 6.1 xtask: AppDir staging (binaries, CEF, desktop entry, icon, AppRun)
- [ ] 6.2 xtask: LGPL FFmpeg fetch, `linuxdeploy` + `appimagetool` download, AppImage output
- [ ] 6.3 CI: build the AppImage on ubuntu-22.04 and upload it as an artifact
- [ ] 6.4 README: Linux install (AppImage), build from source (apt/dnf/pacman packages), runtime
      requirements (PipeWire, portals, v4l2loopback)

## 7. Verify
- [ ] 7.1 WSLg and a GNOME/KDE session: preview, record, stream, hotkeys
- [ ] 7.2 AppImage runs on a clean Ubuntu without FFmpeg installed
- [ ] 7.3 Windows: full test suite and a recording unchanged
