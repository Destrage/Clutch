# eStream

A modern streaming and recording studio written from scratch in Rust. It is an alternative to OBS: a wgpu compositor, a Slint UI and FFmpeg encoding.

**Features so far:**
- scenes and sources: display, window and camera capture, media files and playlists, browser pages, test patterns;
- audio mixer with filters;
- recording and replay buffer;
- streaming to several platforms at once;
- studio mode with transitions, including stingers and custom shaders;
- video filters;
- multiple canvases (for example horizontal and vertical at the same time);
- a virtual camera for Zoom, Teams, Discord, Meet and other apps (Windows 11);
- automation: macros started by global hotkeys, a window coming to the front, timers, scenes or outputs;

Windows 10/11 (x64) is the supported platform today.

## Requirements

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
.\target\release\estream.exe
```

You can also use `cargo run --release -p estream`. Build the other two at least once first: `cargo run` only builds the studio, and browser sources and the virtual camera need `estream-browser.exe` and `estream_vcam.dll` next to `estream.exe`.

The first build takes a while because it compiles CEF's wrapper and many dependencies. Later builds are incremental.

Debug builds (`cargo build -p estream -p estream-browser -p estream-vcam`, then `.\target\debug\estream.exe`) work but run more slowly. Use them for development.

### Where your data goes

Settings, scene collections and logs are stored in `%APPDATA%\eStream`. To use a separate folder, for example for testing, set `ESTREAM_HOME`:

```powershell
$env:ESTREAM_HOME = "D:\estream-test"; .\target\release\estream.exe
```

Stream keys are encrypted with Windows DPAPI and never written to project files.

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

Click the camera button in the top bar, then pick **eStream Virtual Camera** in your call app. Windows lists it as "eStream Virtual Camera (Windows Virtual Camera)". It needs Windows 11.

- **First start:** Windows asks once for permission to install the camera component, so expect a UAC prompt. eStream copies the component to `%ProgramData%\eStream\VirtualCamera` and registers it there.
- **Choosing what it shows:** pick the canvas in **Settings → Virtual camera**.
- **While no app is using it:** eStream does no extra rendering for it.
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
openspec/               specs and change proposals (start with openspec/project.md)
```

## Contributing

eStream is spec-driven. Read `openspec/AGENTS.md` and `openspec/project.md` first. A new feature starts as a change proposal in `openspec/changes/`, and its `tasks.md` is ticked off as the work is done.
