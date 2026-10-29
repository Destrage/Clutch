# Change: macOS support

## Why
Clutch runs on Windows and Linux. The core (compositor, mixer, encoders, project, UI) is
portable, and everything that touches the OS sits behind `cfg` with a Windows and a Linux
backend. On macOS those backends are missing, so capture, hotkeys, stream keys, the browser
source, the virtual camera, updates and packaging don't exist there, and the workspace has never
been built on a Mac.

The goal is **parity with Clutch 0.7.0**: everything a 0.7.0 user can do on Windows also works
on a Mac. Where macOS can't do something, the app says so with a short reason (game capture is
the one known case). Features that land on `main` after 0.7.0 are follow-up work, not part of
this change.

## What Changes
- **Builds and runs on macOS 13 Ventura and later, Apple Silicon (arm64).** CI builds, lints and
  tests on macOS next to Windows and Linux.
- **GPU:** wgpu on Metal. The engine and UI share one device, as on Linux.
- **Portable building blocks:**
  - shared memory (`estream-shm`) on macOS, with its 31-character name limit;
  - stream keys in the macOS Keychain;
  - system stats (CPU, memory, GPU load);
  - the Steam library path;
  - following the system's dark or light appearance;
  - the clip-saved and chat sounds.
- **Capture:**
  - ScreenCaptureKit for displays, windows, desktop audio and per-app audio;
  - AVFoundation for cameras;
  - CoreAudio microphones through `cpal`.

  Game capture stays Windows-only. On macOS, window capture covers it.
- **Privacy permissions:**
  - Screen Recording, Camera and Microphone are requested when first needed;
  - a denied permission shows what it blocks and opens the right System Settings page;
  - the app bundle carries the usage strings macOS requires.
- **Encoding:**
  - VideoToolbox H.264 and HEVC hardware encoders;
  - the existing OpenH264 and SVT-AV1 software encoders;
  - an LGPL FFmpeg 9 built for macOS.
- **Automation:** global hotkeys, the "window focused" trigger, and game detection for clips on
  macOS.
- **Browser source:** the out-of-process CEF helper runs on macOS, with the helper app bundles
  CEF needs.
- **Virtual camera:** a CoreMediaIO Camera Extension. It needs a Developer ID–signed app, so it
  comes last and depends on the signing decision. Until then the output reports that it is not
  available on this Mac.
- **Packaging and updates:**
  - `cargo xtask package` on macOS builds `Clutch.app` and `Clutch-<version>-arm64.dmg`, with
    FFmpeg and CEF bundled;
  - the updater downloads the new .dmg, checks its digest and replaces the app.
- **Docs:** a macOS section in the README (install, build from source, permissions).

## Out of scope
- Intel Macs and universal binaries (possible later, after arm64 works).
- Mac App Store distribution (its sandbox doesn't fit the browser source or the virtual camera).
- Features that land on `main` after 0.7.0 (follow-up `mac/` changes).

## Impact
- Affected specs: `platform-support` (new macOS requirements next to the Linux ones from
  `add-linux`).
- Affected code:
  - `estream-gpu` (backend choice);
  - `estream-shm`;
  - `estream-capture` (new `macos` module next to `windows` and `linux`);
  - `estream-audio` (devices);
  - `estream-media` (VideoToolbox, vcam feed);
  - `estream-vcam`;
  - `estream-automation` (hotkeys, focus);
  - `estream-health` (stats);
  - `estream-clips` (foreground game, Steam);
  - `apps/estream` (secrets, appearance, sounds, updater, permissions);
  - `apps/estream-browser` (CEF on macOS);
  - `xtask` (app bundle, dmg);
  - `.github/workflows/ci.yml`;
  - `README.md`.
- New macOS-only dependencies: the `objc2` framework crates (Foundation, AppKit, AVFoundation,
  CoreMedia, CoreVideo, ScreenCaptureKit, CoreGraphics, IOSurface) and the Keychain store for
  `keyring-core`.
- No change to the project file format. A project made on Windows opens on a Mac, with game
  capture sources listed as unavailable and kept.
- Windows and Linux behaviour does not change. Every macOS addition is behind
  `cfg(target_os = "macos")`.
