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
audience, so it is not included. The system's picker chooses the display or window; the
source's "Choose again…" button (a new `PropKind::Button`) reopens it. The portal's restore
token skips the picker at later starts. Tokens are single-use (each start returns the next
one) and mean nothing on another machine, so they are not kept in the source settings, where
every start would mark the project unsaved and add an undo step: they live in
`portal-tokens.json` in Clutch's settings folder, keyed by source id. Sources learn their id
from `SourceContext::source`.

### Audio
cpal's PipeWire host (its `pipewire` feature, Linux only) covers microphones and desktop audio:
opening an input stream on a sink captures what it plays, as WASAPI loopback does, so the
existing `audio_output` type works unchanged. Its device lists are filtered to sources
(microphones) and sinks (speakers, headphones), leaving out cpal's virtual "default" entries and
apps' playback streams; the real defaults come from PipeWire's `default` metadata, and default
desktop audio uses the default sink itself (it follows default changes). Per-app audio opens a
capture stream on each playback stream of the chosen app (`target.object`, not reconnecting
when the app's stream ends) and mixes them on the audio thread. Apps are matched by
`application.process.binary`, ignoring case and `.exe`, so Windows projects carry over. Both
feed the existing `StereoConsumer` ring buffers, so the mixer is unchanged.

### Hotkeys and focus
A small `GlobalHotkeys` trait in `estream-automation` (register, failed, try_press), with the
Windows `RegisterHotKey` listener as one implementation. Linux uses the GlobalShortcuts portal
where available (KDE, GNOME 48+): one session for the whole process, shared by macros and the
clip hotkey so the user is asked once, with the chord text as the stable shortcut id. Binding
can wait on a desktop dialog, so failures are reported asynchronously. Without the portal,
X11 sessions use `XGrabKey` (Caps/Num Lock variants included; another client's grab shows as
a conflict). Elsewhere hotkeys work only while Clutch has focus (the app passes its own key
presses to the runner and the clip hotkey) and the settings say so. `CLUTCH_HOTKEYS=portal|x11|off`
overrides the choice.

The portal ties permissions to an app id, which host apps register through
`org.freedesktop.host.portal.Registry`; it only accepts an id with a desktop entry whose `Exec`
exists. Clutch (`io.github.destrage.Clutch`) therefore keeps a hidden (`NoDisplay`) entry in
`~/.local/share/applications` pointing at the running binary or `$APPIMAGE`, marked
`X-Clutch-Generated` and never overwriting an entry it didn't write.

The focus trigger reads EWMH `_NET_ACTIVE_WINDOW` on X11 through the clip detector's probe
(which also needs the window's pid and fullscreen state), plugged into the runner by the app;
on Wayland, where no portable API exists, the macro editor says the trigger is unavailable.

### Virtual camera
A `v4l2loopback` device (the one labelled "Clutch" if there are several), found in sysfs as a
`video4linux` device with no hardware behind it. Clutch sets a YUYV format (what webcam apps
read most widely) sized to the canvas, fitted within 1080p, and writes every canvas frame with
`write()`. The loopback device can't tell whether an app is reading, so frames are written
while the camera is on. Nothing is installed with Clutch: without the module, starting the
camera fails with how to install and load it.

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
2. Stage an AppDir: `usr/bin` holds `clutch`, `clutch-browser`, the CEF files and the FFmpeg
   libraries side by side (stripped: CEF ships with ~1.2 GB of symbols), plus the desktop entry
   `io.github.destrage.Clutch.desktop`, the icon (from `xtask/src/icon.rs`), AppStream metainfo
   and an `AppRun` script.
3. Produce `target/package/Clutch-<version>-x86_64.AppImage` with `appimagetool` (pinned,
   downloaded into `third_party/appimage` like WiX on Windows; run extract-and-run, so building
   needs no FUSE).

`linuxdeploy` isn't needed: the binaries find FFmpeg and CEF through their `$ORIGIN` RPATH,
and what they need beyond that comes from the system, as the AppImage guidelines require:
glibc, the GPU driver (Vulkan/GL), PipeWire, ALSA, fontconfig, D-Bus, the portals, and the
libraries Chromium uses (NSS, ATK, GBM, cups, X11), which every desktop that runs Chrome has.
CEF runs without its setuid sandbox (`chrome-sandbox` can't work from an AppImage); the helper is
isolated as a separate process. The window reports the same app id (`slint::set_xdg_app_id`),
so the desktop shows the right icon. The AppImage is built on the oldest supported Ubuntu
(22.04 in CI) so its glibc floor is low.

## Risks / Trade-offs
- **Desktop diversity.** Portal support differs across desktops. Mitigation: feature checks at
  start, clear "not available" states, test on GNOME and KDE; WSLg only covers basic runs.
- **FFmpeg licensing.** The distro's FFmpeg may be built with GPL parts. The AppImage bundles
  an LGPL FFmpeg build instead (the same build flags as the Windows package), fetched by xtask.
- **CEF size.** CEF adds ~200 MB to the AppImage; acceptable, same as the Windows package.

## Migration
None. Projects and settings load unchanged; Windows behaviour is unchanged.
