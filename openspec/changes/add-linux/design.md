# Design: Linux support

## Context
Windows code lives in `#[cfg(windows)]` blocks scattered across nine crates, most with a
`not(windows)` fallback that disables the feature. The engine talks to sources only through
`SourceType` / `VideoSource` / `AudioSource`, so Linux backends register the same way
`estream_capture::windows::register` does today.

## Goals / Non-goals
- Goals: every feature with a reasonable Linux API works; nothing panics or silently misbehaves
  where a feature is missing; one codebase, no Linux fork of the UI; a single-file install.
- Non-goals: macOS (next change); game capture (hook injection) on Linux; Flatpak, Snap or
  distro packages; arm64 or 32-bit builds.

## Decisions

### FFmpeg
Linux uses the same FFmpeg 9 LGPL shared build as Windows (BtbN `linux64-lgpl-shared`) in
`third_party/ffmpeg`, not the distro's: distros ship other major versions (Ubuntu 26.04 has 8)
and often GPL builds. `estream-media/build.rs` copies the `.so.<major>` files next to the
binaries and `.cargo/config.toml` adds an `$ORIGIN` rpath, mirroring how the DLLs are found on
Windows; the AppImage reuses the same files. The distro's `libav*-dev` headers in
`/usr/include` can shadow `FFMPEG_DIR`'s in bindgen (codec ids then don't match the library),
so the build warns when they are present and `estream_media::init` logs an error on a mismatch.

### Module layout
`estream-capture/src/linux/` with a `register(&mut SourceRegistry)`. Source type ids stay the
same as on Windows (`display_capture`, `window_capture`, `camera`, `app_audio`, …), so a project
made on Windows opens on Linux and only the device/window selection needs picking again. Ids
only Windows has (`game_capture`) are kept and the source shows "Not available on this system".
Linux-only code sits behind `#[cfg(target_os = "linux")]` so a later macOS change doesn't pick it
up by accident.

### Frames into the compositor
PipeWire streams negotiate DMA-BUF when the desktop offers it; the first version accepts SHM
(memfd) buffers and uploads them through the existing upload path (`media/upload.rs`). DMA-BUF
import is a follow-up. V4L2 cameras give YUYV/MJPEG/NV12, which go through the same upload the
Windows camera uses. One CPU copy per frame (~500 MB/s at 1080p60) is within budget and keeps the
`unsafe` surface small.

### Screen capture: portal only
The ScreenCast portal works on GNOME, KDE, wlroots and on X11 sessions with the portal installed,
and it is the only option on Wayland. A direct X11 path would be a second backend for a shrinking
audience, so it is not included. The portal's restore token is saved in the source settings so
the picker isn't shown at every start.

### Audio
cpal already covers microphones and output devices (ALSA, which PipeWire serves). Desktop audio
comes from the PipeWire monitor of the default sink; per-app audio links to the app's output
node. Both feed the existing `StereoConsumer` ring buffers, so the mixer is unchanged.

### Hotkeys and focus
A small `GlobalHotkeys` trait in `estream-automation`, with the current Windows `RegisterHotKey`
listener as one implementation. Linux uses the GlobalShortcuts portal where available (KDE,
GNOME 48+), else `XGrabKey` on X11; on Wayland without the portal hotkeys work only while Clutch
has focus and the settings page says so. The focus trigger reads `_NET_ACTIVE_WINDOW` on X11 and
reports "unavailable" on Wayland, where no portable API exists.

### Secrets
`keyring-core` with the zbus Secret Service store (pure Rust, no libdbus). The Secret Service
holds one random 256-bit key (`Clutch` / `secrets`); each value is sealed with it
(ChaCha20-Poly1305 via `ring`, random nonce) and stored as `keyring:<hex>` next to the existing
`dpapi:` values. Like DPAPI this keeps `protect`/`unprotect` stateless: no entry per destination
to name, rename or clean up, and the settings file never holds plain text. Without a Secret
Service the key lives for the session only, and the Go Live box and destination settings say so.

### Shared memory
POSIX `shm_open`/`mmap` behind the same `Shm::create`/`open` API. Names are prefixed `/clutch-`
and unlinked by the creator on drop.

### AppImage
`cargo xtask package` on Linux:
1. `cargo build --release` for `clutch` and `clutch-browser`.
2. Stage an AppDir: `usr/bin/clutch`, `usr/bin/clutch-browser`, the CEF files next to it,
   `clutch.desktop`, the icon (from `xtask/src/icon.rs`) and an `AppRun`.
3. Run `linuxdeploy` (downloaded into `target/xtask-tools` like WiX on Windows) to copy the
   FFmpeg libraries and other non-system dependencies into `usr/lib` and fix their rpaths.
   glibc, the GPU driver (Vulkan/GL), PipeWire, D-Bus and the portals come from the system and
   are excluded, as the AppImage guidelines require.
4. Produce `target/package/Clutch-<version>-x86_64.AppImage` with `appimagetool`.
The AppImage is built on the oldest supported Ubuntu (22.04 in CI) so its glibc floor is low.

## Risks / Trade-offs
- **Desktop diversity.** Portal support differs across desktops. Mitigation: feature checks at
  start, clear "not available" states, test on GNOME and KDE; WSLg only covers basic runs.
- **FFmpeg licensing.** The distro's FFmpeg may be built with GPL parts. The AppImage bundles
  an LGPL FFmpeg build instead (the same build flags as the Windows package), fetched by xtask.
- **CEF size.** CEF adds ~200 MB to the AppImage; acceptable, same as the Windows package.

## Migration
None. Projects and settings load unchanged; Windows behaviour is unchanged.
