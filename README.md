# Clutch

Stream, record and clip your best moments, then cut them into beat-synced montages. Clutch is a modern studio written from scratch in Rust, with a wgpu compositor, a Slint UI and FFmpeg encoding.

**Features so far:**
- scenes and sources: display, window and camera capture, media files and playlists, browser pages, test patterns;
- audio mixer with filters;
- recording and replay buffer;
- streaming to several platforms at once;
- studio mode with transitions, including stingers and custom shaders;
- video filters;
- multiple canvases (for example horizontal and vertical at the same time);
- a virtual camera for Zoom, Teams, Discord, Meet and other apps (Windows 11, or Linux with v4l2loopback);
- automation: macros started by global hotkeys, a window coming to the front, timers, scenes or outputs;
- stats and health: live numbers with 5 minutes of history, and plain-language alerts (rendering lag, encoder overload, network trouble, CPU/GPU load, low disk) with advice;

Clutch runs on Windows 10/11 (x64) and on Linux (x86_64, Wayland or X11). See [Linux](#linux) for what differs there.

## Getting started

1. **First run.** Pick what you'll use Clutch for: streaming games, talking to camera, recording videos, or vertical video. Clutch finds your screen, webcam, microphone and encoder, then builds a ready scene. Noise suppression is on for the mic. "Choose the resolution myself" shows every canvas preset.
2. **Go live.** Click **Go Live**, pick Twitch, YouTube, Kick or Facebook Live, and paste your stream key. "Where do I find it?" opens the platform's key page. Clutch recognizes the platform from the key, catches a pasted server address, and picks the server, the encoder and the platform's recommended bitrate. Use **More options** for custom servers, SRT/RIST/WHIP, or your own encoder settings. To stream to another platform as well, use **Add platform** in the menu next to Go Live.

## Requirements

These are for building on Windows; for Linux see [Build from source on Linux](#build-from-source-on-linux).

| Tool | Why | Install |
|---|---|---|
| Rust (stable, 1.90+) | the compiler; `rust-toolchain.toml` selects it | <https://rustup.rs> |
| Visual Studio 2022 Build Tools, "Desktop development with C++" | MSVC linker and Windows SDK | `winget install Microsoft.VisualStudio.2022.BuildTools` |
| LLVM / Clang | bindgen (FFmpeg and CEF bindings) | `winget install LLVM.LLVM` |
| CMake and Ninja | build CEF's C++ wrapper | `winget install Kitware.CMake Ninja-build.Ninja` |
| FFmpeg 9, shared LGPL build | encoding and decoding | see below |

LLVM must be installed at `C:\Program Files\LLVM`. If it is somewhere else, change `LIBCLANG_PATH` in `.cargo/config.toml`.

### FFmpeg

1. Download a **win64 LGPL shared** build of FFmpeg 9, for example `ffmpeg-n9.*-win64-lgpl-shared` from <https://github.com/BtbN/FFmpeg-Builds/releases>.
2. Extract it so that the folders look like this:

```
third_party/ffmpeg/bin/       avcodec-63.dll, avformat-63.dll, ...
third_party/ffmpeg/include/
third_party/ffmpeg/lib/
```

`third_party/` is git-ignored. The build copies the DLLs next to the executables, so you don't need to change `PATH`.

### CEF (browser source)

You don't need to do anything for CEF. On the first build, the Chromium Embedded Framework (about 150 MB) downloads into `third_party/cef/` automatically. Its runtime files (`libcef.dll`, `.pak` files, `locales/`) are copied next to the executables.

## Build and run

Use **PowerShell** or a Developer Command Prompt. Git Bash puts its own `link.exe` ahead of MSVC's linker and breaks the build.

```powershell
# Build the studio, the browser helper and the virtual camera component
cargo build --release -p estream -p estream-browser -p estream-vcam

# Run it
.\target\release\clutch.exe
```

You can also use `cargo run --release -p estream`. Build the other two at least once first: `cargo run` only builds the studio, and browser sources and the virtual camera need `clutch-browser.exe` and `estream_vcam.dll` next to `clutch.exe`.

The first build takes a while because it compiles CEF's wrapper and many dependencies. Later builds are incremental.

Debug builds (`cargo build -p estream -p estream-browser -p estream-vcam`, then `.\target\debug\clutch.exe`) work but run more slowly. Use them for development.

### Windows installer

```powershell
cargo xtask package
```

This produces `target\installer\Clutch-<version>-x64.msi`. The command:
- builds release binaries with the C runtime linked in, so the VC++ redistributable isn't needed;
- stages them with FFmpeg, the CEF runtime and the license notices;
- builds the MSI with WiX 3.14, which downloads once into `third_party/wix`.

The installer:
- installs to `C:\Program Files\Clutch`, with optional desktop and Start menu shortcuts (both unchecked by default; an upgrade keeps the earlier choice);
- registers the virtual camera for all users, so Clutch never has to ask for that permission later;
- upgrades older versions in place.

Uninstall it from Windows Settings → Apps.

`cargo xtask icon` regenerates the app icon (`apps/estream/assets/clutch.ico`).

### Where your data goes

Settings, scene collections and logs are stored in `%APPDATA%\Clutch`. On its first start, Clutch moves an existing `%APPDATA%\eStream` folder there (the app was called eStream up to 0.2). To use a separate folder, for example for testing, set `CLUTCH_HOME` (`ESTREAM_HOME` also works):

```powershell
$env:CLUTCH_HOME = "D:\clutch-test"; .\target\release\clutch.exe
```

Stream keys are encrypted with Windows DPAPI and never written to project files.

On Linux the folder is `~/.config/Clutch`, and `CLUTCH_HOME` works the same way.

## Linux

### Install

Download `Clutch-<version>-x86_64.AppImage`, make it executable and run it:

```sh
chmod +x Clutch-*-x86_64.AppImage
./Clutch-*-x86_64.AppImage
```

It runs on distributions from 2024 on (glibc 2.39 or newer: Ubuntu 24.04, Debian 13, Fedora 40, current Arch), and contains FFmpeg and the browser source's Chromium runtime. Everything else comes from your desktop:

| What | Needed for | Usually |
|---|---|---|
| PipeWire | audio, desktop and app audio, screen capture | the default on current distributions |
| xdg-desktop-portal with your desktop's backend (GNOME, KDE, wlroots) | screen and window capture, global hotkeys | installed with the desktop |
| a Secret Service (GNOME Keyring, KWallet) | remembering stream keys | installed with GNOME and KDE |
| a Vulkan or OpenGL driver | the preview and compositing | installed with the desktop |
| `v4l2loopback` | the virtual camera | `sudo apt install v4l2loopback-dkms` |

On first start Clutch writes a hidden desktop entry (`~/.local/share/applications/io.github.destrage.Clutch.desktop`, not shown in menus) pointing at the AppImage. Desktops only give permissions such as global hotkeys to an app they can find an entry for. Clutch never changes an entry it didn't write.

### Build from source on Linux

Install the Rust toolchain from <https://rustup.rs> and these packages:

```sh
# Ubuntu, Debian
sudo apt install build-essential pkg-config clang libclang-dev cmake ninja-build \
  libasound2-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev \
  libxi-dev libfontconfig-dev libssl-dev libdbus-1-dev libudev-dev libpipewire-0.3-dev

# Fedora
sudo dnf install gcc gcc-c++ pkgconf-pkg-config clang clang-devel cmake ninja-build \
  alsa-lib-devel libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel \
  libXi-devel fontconfig-devel openssl-devel dbus-devel systemd-devel pipewire-devel

# Arch
sudo pacman -S base-devel clang cmake ninja alsa-lib libxkbcommon wayland libx11 libxcursor \
  libxrandr libxi fontconfig openssl dbus systemd-libs pipewire
```

Then put the **linux64 LGPL shared** build of FFmpeg 9 (for example `ffmpeg-n9.*-linux64-lgpl-shared` from <https://github.com/BtbN/FFmpeg-Builds/releases>) in `third_party/ffmpeg`, so that `third_party/ffmpeg/lib` holds the `.so` files. Don't install your distribution's `libav*-dev` packages: their headers can take precedence over the bundled ones, and the build warns if they do. CEF downloads by itself, as on Windows.

```sh
cargo build --release -p estream -p estream-browser
./target/release/clutch
```

The FFmpeg and CEF libraries are copied next to the binaries, which find them there. `cargo xtask package` builds the AppImage into `target/package/`; it downloads `appimagetool` once into `third_party/appimage`.

### What differs on Linux

- **Screen and window capture** go through your desktop's picker. Clutch remembers the choice, so the picker only shows the first time; **Choose again…** in the source's settings opens it again.
- **Game capture** isn't available; use window or display capture. Projects made on Windows keep their game capture sources, shown as not available.
- **App audio** lists the apps that are currently playing sound and reattaches when the app plays again.
- **Global hotkeys** go through your desktop's shortcut settings (KDE, GNOME 48 and later) and may be changed there; your desktop may ask once. On X11 desktops without that, Clutch grabs the keys itself. Elsewhere hotkeys work while Clutch is in front, and the settings say so. Set `CLUTCH_HOTKEYS` to `portal`, `x11` or `off` to choose.
- **The "window comes to the front" trigger** works on X11. Wayland doesn't tell apps which window is in front, so there the trigger is shown as unavailable.
- **The virtual camera** writes to a `v4l2loopback` device. Load the module with a label so call apps show a recognisable name:

  ```sh
  sudo modprobe v4l2loopback card_label="Clutch" exclusive_caps=1
  ```

  To load it at every start, put `v4l2loopback` in `/etc/modules-load.d/v4l2loopback.conf` and `options v4l2loopback card_label="Clutch" exclusive_caps=1` in `/etc/modprobe.d/v4l2loopback.conf`.
- **Stream keys** are kept with your desktop's keyring (Secret Service). Without one, a key lasts until Clutch closes, and the Go Live box says so.

## Remote control

Turn it on in **Settings → Remote control**. Then point Stream Deck, Touch Portal, Streamer.bot or any obs-websocket v5 client at the address shown there (port 4455 by default), using the password from the same section.

To try requests by hand:

```powershell
cargo run -p estream-api --example obs-cli -- --password <password> GetSceneList SetCurrentProgramScene '{"sceneName":"Scene"}'
```

## Automation

Click the lightning button in the top bar. A **macro** is a list of steps, such as:
- switch scene;
- show, hide or mute a source;
- start or stop an output;
- save a replay;
- run any command palette action;
- wait.

It runs when one of its **triggers** fires:
- a global hotkey, which works while a game has focus;
- a window coming to the front (match on program name or title, e.g. `valorant`);
- every N seconds;
- a scene going live;
- an output starting or stopping.

Macros are saved with the scene collection, and every change they make can be undone. Remote controls see macros as obs-websocket hotkeys (`GetHotkeyList`, `TriggerHotkeyByName`).

## Virtual camera

Click the camera button in the top bar, then pick **Clutch Virtual Camera** in your call app. Windows lists it as "Clutch Virtual Camera (Windows Virtual Camera)". It needs Windows 11. On Linux it uses `v4l2loopback`; see [Linux](#what-differs-on-linux).

- **First start:** Windows asks once for permission to install the camera component, so expect a UAC prompt. Clutch copies the component to `%ProgramData%\Clutch\VirtualCamera` and registers it there.
- **Choosing what it shows:** pick the canvas in **Settings → Virtual camera**.
- **While no app is using it:** Clutch does no extra rendering for it.
- **Development:** after changing `crates/estream-vcam`, use **Settings → Virtual camera → Reinstall** so Windows loads the new build.
- **Problems:** the camera component writes `vcam.log` in the same `%ProgramData%` folder.

The end-to-end test runs through Windows' camera service and needs the component installed:

```powershell
cargo test -p estream-vcam --test system -- --ignored
```

## Tests

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

Some tests use real hardware: the GPU, hardware encoders, screen capture and the browser helper. Build `estream-browser` before running the browser test.

## Project layout

```
apps/estream            the studio app (Slint UI)
apps/estream-browser    CEF helper process for browser sources
crates/estream-core     ids, time, shared types
crates/estream-project  the scene collection model (serde)
crates/estream-engine   command bus, undo, render graph, real-time video/audio threads
crates/estream-gpu      wgpu compositor, transitions, filters
crates/estream-capture  display/window/camera capture, browser source host
crates/estream-audio    audio devices, mixer, audio filters
crates/estream-media    FFmpeg encoding, recording, streaming, media player
crates/estream-shm      shared-memory frames and audio between processes
crates/estream-api      remote control server (obs-websocket v5 protocol)
crates/estream-vcam     virtual camera: the Media Foundation camera source DLL and its controls
crates/estream-automation  macros, triggers and global hotkeys
crates/estream-health   stats history, health alerts and the system sampler
openspec/               specs and change proposals (start with openspec/project.md)
```

## Contributing

Clutch is spec-driven. Read `openspec/AGENTS.md` and `openspec/project.md` first. A new feature starts as a change proposal in `openspec/changes/`, and its `tasks.md` is ticked off as the work is done.
