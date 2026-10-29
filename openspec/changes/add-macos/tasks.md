# Tasks: add-macos

Work in the order below, one PR per group (branches in `design.md`, "PR plan"). Tick a task only
when it is done and tested on a Mac; write down in the PR what was tested. Every PR must leave
the Windows and Linux CI jobs green.

## 1. Build on macOS
- [ ] 1.1 Workspace compiles on macOS arm64: give every `windows`-only item reached without a
      `cfg` a gate, and route the "other systems" fallbacks so they compile (behaviour comes in
      later groups)
- [ ] 1.2 `tools/macos-build/ffmpeg.sh`: builds LGPL FFmpeg 9 (shared, VideoToolbox,
      AudioToolbox, OpenH264, SVT-AV1, Opus) into `third_party/ffmpeg`, with `@rpath`
      install names; README and `design.md` point to it
- [ ] 1.3 `estream-gpu`: Metal backend chosen on macOS; engine and UI share one device
- [ ] 1.4 `cargo test --workspace` passes on the Mac (GPU tests included, on the real GPU)
- [ ] 1.5 CI: `rust-macos` job on `macos-15` (cached FFmpeg build, fmt, clippy, test); GPU tests
      skip with a log line only if the runner has no Metal adapter
- [ ] 1.6 App starts: main window, live preview of an empty scene, color and image sources,
      record an empty scene to MP4 with a software encoder, settings saved under
      `~/Library/Application Support/Clutch`

## 2. Portable building blocks
- [ ] 2.1 `estream-shm` on macOS (POSIX shm), names within 31 characters, one-time sizing; tests
      run on macOS
- [ ] 2.2 Stream keys in the Keychain (`keyring-core` Keychain store, same `keyring:` prefix)
- [ ] 2.3 `estream-health`: CPU, memory and GPU load on macOS
- [ ] 2.4 `estream-clips`: Steam library under `~/Library/Application Support/Steam`
- [ ] 2.5 `system_prefers_dark` follows the macOS appearance
- [ ] 2.6 Clip-saved sound and chat message beep play on macOS
- [ ] 2.7 Windows-only source kinds (game capture) load as "Not available on this system" and
      survive a save, as on Linux

## 3. Capture
- [ ] 3.1 Screen Recording permission: check before capture, request once, explain and open
      System Settings when denied
- [ ] 3.2 ScreenCaptureKit display capture (list displays, stream, upload to the compositor),
      cursor on/off
- [ ] 3.3 ScreenCaptureKit window capture (list windows by app and title, follow a window that
      moves or resizes)
- [ ] 3.4 Desktop audio and per-app audio through ScreenCaptureKit, excluding Clutch itself
- [ ] 3.5 AVFoundation cameras: enumeration, formats, NV12/BGRA upload, Camera permission
- [ ] 3.6 Microphones through CoreAudio (`cpal`): device list, default device changes, Microphone
      permission
- [ ] 3.7 Register the macOS source types; device detection in the first-run wizard finds the Mac's
      display, camera and microphone

## 4. Encoding
- [ ] 4.1 `h264_videotoolbox` and `hevc_videotoolbox` in the encoder table (family
      `videotoolbox`, NV12 input, rate control mapped)
- [ ] 4.2 Default encoder on macOS is VideoToolbox H.264; the encoder list hides NVENC/AMF/QSV
- [ ] 4.3 Record 10 minutes at 1080p60 with display capture, camera and mic, and check the file
      with `ffprobe` (A/V sync, no dropped frames)
- [ ] 4.4 Stream to a test RTMP server and over SRT with VideoToolbox; multistream to two
      destinations

## 5. Automation
- [ ] 5.1 Global hotkeys via `RegisterEventHotKey` behind `GlobalHotkeys`; combinations macOS
      refuses are reported in the hotkey editor
- [ ] 5.2 "Window focused" trigger via `NSWorkspace` (app) and window titles when Screen
      Recording is granted; the macro editor says when it matches by app only
- [ ] 5.3 `estream-clips` foreground app and window on macOS, so game-aware clipping works

## 6. Browser source
- [ ] 6.1 `clutch-browser` builds on macOS and loads the CEF framework at runtime
- [ ] 6.2 Helper app bundles (`Helper`, `GPU`, `Renderer`, `Plugin`, `Alerts`) created for
      development runs (script) and packaging
- [ ] 6.3 Browser source, interactive browser, chat overlay and alerts render with transparency
      and audio

## 7. Virtual camera (after the maintainer decides on Developer ID signing)
- [ ] 7.1 Until then: the virtual camera output reports that it isn't available on this Mac
- [ ] 7.2 CoreMediaIO Camera Extension fed through `estream-vcam`'s shared-memory protocol
- [ ] 7.3 Installed and activated from `Clutch.app`; FaceTime, Zoom and a browser see "Clutch
      Virtual Camera"

## 8. Packaging and updates
- [ ] 8.1 `cargo xtask package` on macOS: `Clutch.app` (binaries, FFmpeg dylibs, CEF framework
      and helpers, `Info.plist` with usage strings, `.icns` icon), ad-hoc signed
- [ ] 8.2 `Clutch-<version>-arm64.dmg` with an Applications link
- [ ] 8.3 The app runs on a Mac without Homebrew or FFmpeg installed (check with `otool -L` that
      nothing links outside the bundle or `/System` and `/usr/lib`)
- [ ] 8.4 Updater: picks the `-arm64.dmg` asset, verifies the digest, replaces the app and
      relaunches; falls back to opening the release page when the bundle isn't writable
- [ ] 8.5 README: macOS install (first-launch approval in Privacy & Security, permissions), build
      from source, what differs on macOS

## 9. Verify
- [ ] 9.1 Windows and Linux CI green on every PR, and Windows behaviour unchanged (maintainer
      checks a recording on Windows)
- [ ] 9.2 Walk the "0.7.0 parity checklist" in `design.md` on a Mac and note any gaps in the
      final PR
- [ ] 9.3 A project made on Windows opens on the Mac and back without losing sources
