# Change: Linux support

## Why
Clutch only runs on Windows. The core (compositor, mixer, encoders, project, UI) is portable,
but everything that touches the OS is Windows-only: screen, window and camera capture, per-app
audio, global hotkeys, focus triggers, the virtual camera, shared memory for the browser source,
stored stream keys, system stats and the installer. On Linux those parts are missing or fail,
and the workspace has never been built or tested there. macOS follows in a later change.

## What Changes
- **Builds and runs on Linux** (x86_64, Wayland and X11). CI builds, lints and tests on Linux
  next to Windows.
- **Portable building blocks**:
  - Shared memory (`estream-shm`) on POSIX (`shm_open` + `mmap`), so the browser source works.
  - Stream keys stored through the Secret Service (GNOME Keyring, KWallet) instead of not being
    saved.
  - GPU backend: Vulkan, with GL as fallback.
  - System stats (CPU, memory, GPU where available) and Steam library lookup on Linux.
- **Capture backends**: the xdg-desktop-portal ScreenCast portal over PipeWire for displays and
  windows (Wayland and X11); PipeWire for desktop and per-app audio; V4L2 for cameras. Game
  capture stays Windows-only; window capture covers it on Linux.
- **Automation**: global hotkeys through the GlobalShortcuts portal, with an X11 key grab
  fallback; the "window focused" trigger on X11 (reported as unavailable on Wayland).
- **Virtual camera** through `v4l2loopback`.
- **AppImage**: `cargo xtask package` on Linux builds `Clutch-<version>-x86_64.AppImage` with
  FFmpeg and the browser-source helper bundled, plus a desktop entry and icon. Users download
  one file, make it executable and run it. Building from source also works and is documented.

## Impact
- Affected specs: new `platform-support` capability.
- Affected code: `estream-capture` (new `linux` module next to `windows`), `estream-shm`,
  `estream-automation` (hotkeys, focus), `estream-vcam` / `estream-media` (vcam feed),
  `estream-health` (system stats), `estream-clips` (Steam paths), `estream-gpu` (backend
  choice), `apps/estream` (secrets, opening files and folders), `apps/estream-browser` (CEF on
  Linux), `xtask` (AppImage), `README.md`, `.github/workflows/ci.yml`.
- New Linux-only dependencies: `ashpd`, `pipewire`, `v4l`, `keyring`, `x11rb`.
- No change to the project file format. A source whose kind doesn't exist on Linux (game
  capture) is kept and shown as unavailable rather than dropped.
