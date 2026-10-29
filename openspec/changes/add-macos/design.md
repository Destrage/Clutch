# Design: add-macos

## Context
Clutch's platform code follows one pattern. A trait or a small function in the shared crate has
a `#[cfg(windows)]` and a `#[cfg(target_os = "linux")]` implementation, plus a fallback for
"other systems" that today returns nothing or an error. macOS currently falls into that
fallback everywhere. This change gives each of those places a `#[cfg(target_os = "macos")]`
implementation. The Linux port (`openspec/changes/add-linux/`) is the model to follow: read its
proposal, tasks and code before starting, because most macOS work mirrors a Linux task.

Where macOS currently lands in a fallback (search for `target_os = "linux"` and
`not(windows)` to find them all):

| Area | File(s) | Today on macOS |
|---|---|---|
| GPU backend | `crates/estream-gpu/src/context.rs` | non-Windows path; check that Metal is picked |
| Shared memory | `crates/estream-shm/src/lib.rs` | error ("other platforms report an error") |
| Capture registry | `crates/estream-capture/src/lib.rs`, `devices.rs` | no display, window, camera or app audio |
| Audio devices | `crates/estream-audio/src/devices.rs` | `cpal` default host (CoreAudio), untested |
| Encoders | `crates/estream-media/src/encode.rs` | software only; no VideoToolbox entries |
| Virtual camera | `crates/estream-vcam/src/lib.rs`, `crates/estream-media/src/vcam.rs` | "not available" |
| Hotkeys | `crates/estream-automation/src/hotkeys.rs` | in-app only |
| Focus trigger | `crates/estream-automation/src/focus.rs` | unavailable |
| Foreground game | `crates/estream-clips/src/platform.rs` | `None` |
| Steam library | `crates/estream-clips/src/integrations/steam.rs` | non-Windows path (Linux paths) |
| System stats | `crates/estream-health/src/system.rs` | none |
| Stream keys | `apps/estream/src/secret.rs` | session only |
| Appearance | `apps/estream/src/settings.rs` (`system_prefers_dark`) | always dark |
| Sounds | `apps/estream/src/clips.rs`, `apps/estream/src/livechat/mod.rs` | silent |
| Open files/URLs | `apps/estream/src/shell.rs` | `open` already handled |
| Updater | `apps/estream/src/update.rs` | non-Windows path (AppImage/package logic) |
| Browser helper | `apps/estream-browser/src/main.rs` | CEF not set up for macOS |
| Packaging | `xtask/src/main.rs` | none |

## Goals / Non-goals
- Goals: parity with 0.7.0 on Apple Silicon running macOS 13+, with no change to Windows or
  Linux behaviour, and a .dmg a tester can install.
- Non-goals: Intel Macs, the Mac App Store, and features added to `main` after 0.7.0.

## Development setup (macOS)
1. Xcode Command Line Tools (`xcode-select --install`), Homebrew, and Rust via `rustup`. The repo
   pins the stable channel in `rust-toolchain.toml`.
2. Build tools: `brew install pkg-config cmake ninja nasm llvm`. `bindgen` needs libclang: set
   `LIBCLANG_PATH="$(brew --prefix llvm)/lib"`.
3. **FFmpeg 9, LGPL, shared, built from source** into `third_party/ffmpeg` (the same layout as on
   Windows: `include/`, `lib/`). Task 1.2 turns this into `tools/macos-build/ffmpeg.sh`.
   - Do **not** use Homebrew's `ffmpeg`. It is a different major version (wrong headers: codec
     ids come out wrong, which happened on Linux with FFmpeg 8), and it is built with GPL parts,
     so it can never be bundled.
   - Configure with `--enable-shared --disable-static --disable-programs --disable-doc` and no
     `--enable-gpl` / `--enable-nonfree`. Add `--enable-videotoolbox --enable-audiotoolbox`, and
     enable the external libraries the Windows build relies on (at least `libopenh264`,
     `libsvtav1`, `libopus`). Compare `ffmpeg -encoders` from the BtbN Windows build when in
     doubt.
   - Set each dylib's install name to `@rpath/<name>.dylib`, so the app can find them in
     `Contents/Frameworks`.
   - Export `FFMPEG_DIR=$PWD/third_party/ffmpeg` for builds.
4. CEF for macOS arm64 into `third_party/cef` (only needed from task group 6). The `cef` crate
   version in `apps/estream-browser/Cargo.toml` fixes which CEF build matches.
5. First build: `cargo build -p estream`, then `cargo test --workspace`. Fix compile errors
   first (task 1.1). Expect `windows`-only items that are reached without a `cfg`.

**Permissions during development:** a binary started from a terminal gets its privacy
permissions (Screen Recording, Camera, Microphone) through the terminal app. The prompt names
Terminal, iTerm or VS Code, not Clutch. Grant them to the terminal you use. Only the packaged
`Clutch.app` asks for itself.

## Decisions

### Platform bindings: `objc2` crates
Use the `objc2-*` framework crates (`objc2-foundation`, `objc2-app-kit`,
`objc2-screen-capture-kit`, `objc2-av-foundation`, `objc2-core-media`, `objc2-core-video`,
`objc2-core-graphics`, `objc2-io-surface`). They are generated from Apple's headers, cover every
framework this change needs, and keep the bindings consistent. `unsafe` stays inside
`platform`/`macos` modules, and each block gets a `// SAFETY:` comment, as everywhere else.
Declare macOS-only dependencies under `[target.'cfg(target_os = "macos")'.dependencies]`.

### Capture: ScreenCaptureKit, CPU upload first
- `SCShareableContent` lists displays and windows; `SCStream` delivers `CMSampleBuffer`s backed
  by `IOSurface` (BGRA, or NV12 for lower bandwidth).
- **First version:** lock the pixel buffer and upload it into a wgpu texture, as the Linux
  PipeWire SHM path does.
- **Later perf task:** import the `IOSurface` as a Metal texture and wrap it with wgpu's HAL
  interop, so no CPU copy is made.
- Audio comes from the same `SCStream`. With `capturesAudio`, filtering by display gives desktop
  audio, and filtering by application gives per-app audio. Excluding Clutch's own process
  avoids feedback.
- Capture callbacks arrive on a dispatch queue. Hand frames to the engine through the same
  lock-free paths the other backends use; never block the video or audio thread.

### Cameras: AVFoundation
`AVCaptureDevice` discovery (built-in, external and Continuity cameras), then
`AVCaptureVideoDataOutput` with NV12 or BGRA, uploaded like the V4L2 path.

### Encoders: VideoToolbox through FFmpeg
- Add `h264_videotoolbox` and `hevc_videotoolbox` to the encoder table in `encode.rs`, with a
  `videotoolbox` family that takes NV12.
- Map the rate-control settings to the options VideoToolbox supports: bitrate, constant quality
  through `-q:v` where FFmpeg exposes it, and real-time mode on.
- Apple Silicon has no hardware AV1 encoder, so AV1 stays on SVT-AV1.
- Passing GPU frames (`CVPixelBuffer` from the compositor) is a later perf task, like Windows'
  D3D11 path.

### Global hotkeys: Carbon `RegisterEventHotKey`
`RegisterEventHotKey` needs no Accessibility or Input Monitoring permission, unlike a
`CGEventTap`. macOS 15 refuses combinations whose only modifiers are Option and/or Shift. The
hotkey editor must report that rather than silently doing nothing. Implement it behind the
existing `GlobalHotkeys` trait.

### Focus trigger and foreground game
- `NSWorkspace.frontmostApplication` gives the app and needs no permission.
- Window titles from `CGWindowListCopyWindowInfo` need Screen Recording permission. Without it,
  match by app only, and say so in the macro editor.

### Stream keys: Keychain
Use the same `keyring-core` API as Linux, with its macOS Keychain store and the same `keyring:`
prefix, so the settings format doesn't change.

### Shared memory
POSIX `shm_open` + `mmap` already exists for Linux. Enable it for macOS too, with one
difference: macOS limits names to 31 characters (`PSHMNAMLEN`). Shorten the mapped names (for
example, hash the long part) only on macOS, and add a test that the longest name Clutch creates
fits. macOS also does not allow `ftruncate` to resize a region a second time. Size each region
once.

### Browser source: CEF app bundles
On macOS, CEF requires:
- the "Chromium Embedded Framework.framework" in the bundle's `Contents/Frameworks`;
- helper apps next to it (`<name> Helper.app`, plus `(GPU)`, `(Renderer)`, `(Plugin)` and
  `(Alerts)` variants), each with its own `Info.plist`;
- loading the framework at runtime through the `cef` crate's library loader.

`clutch-browser` stays a separate process. The packaging task (group 8) creates these bundles,
and a development script does the same for `cargo run`. This is the most fiddly part of the
port; leave it until capture and encoding work.

### Virtual camera: CoreMediaIO Camera Extension (last, gated)
- A Camera Extension is a system extension inside
  `Clutch.app/Contents/Library/SystemExtensions`, activated with `OSSystemExtensionRequest`.
- Activation needs:
  - an app in `/Applications`;
  - signing with a Developer ID;
  - the system-extension entitlement and a provisioning profile.
- During development, `systemextensionsctl developer on` (with SIP adjusted) avoids signing.
- Shipping it needs an Apple Developer Program membership. That decision belongs to the
  maintainer. Until it is made, the output fails with "The virtual camera isn't available on
  this Mac yet".
- The frame transport reuses `estream-vcam`'s shared-memory protocol.

### Packaging, signing, updates
- `cargo xtask package` on macOS stages `Clutch.app`:
  - `clutch` and `clutch-browser` (plus its helper bundles);
  - the FFmpeg dylibs and the CEF framework in `Contents/Frameworks`;
  - an `Info.plist` with the bundle id `com.destrosphere.clutch`, `LSMinimumSystemVersion` 13.0
    and the camera and microphone usage strings;
  - the icon as `.icns`, generated from the existing icon source by `cargo xtask icon`.
- Then it builds `Clutch-<version>-arm64.dmg` with `hdiutil`.
- **Signing:** builds are ad-hoc signed (`codesign --sign -`) until there is a Developer ID.
  Without notarization, macOS blocks the first launch of a downloaded app, and the user must
  allow it in System Settings → Privacy & Security. Ad-hoc signing also changes the app's
  identity on every build, so privacy permissions are asked again after each update. Both go
  away with Developer ID signing plus notarization (`notarytool`), which the maintainer may
  add later. Keep the signing step a single function, so it can be swapped in.
- **Updater:**
  - pick the `-arm64.dmg` asset and verify its digest (existing code);
  - `hdiutil attach -nobrowse`, copy the app over the running one's bundle with `ditto`,
    detach, and relaunch;
  - if the bundle isn't writable, say so and open the release page instead.

### CI
- Add a `rust-macos` job on an Apple Silicon runner (`macos-15`): build FFmpeg with the script
  from task 1.2 (cached), then fmt, clippy and test.
- GitHub's macOS runners may expose no usable Metal adapter. If they don't, make the GPU tests
  skip with a clear log line on that job only. Windows and Linux keep running them on WARP and
  lavapipe.

## Risks / Trade-offs
- **CEF bundling:** complex. Isolate it in its own PR (group 6), so capture and recording can
  merge earlier.
- **Ad-hoc signing:** gives a worse first-run experience. It is documented in the README and
  fixed by Developer ID later.
- **ScreenCaptureKit on macOS 13:** lacks some features of 14+ (presenter overlay, some window
  filters). Use only what 13 has, or guard newer calls with an OS version check.

## PR plan
One PR per task group, each on its own branch from `main` and each leaving `main` working on
every platform:

| PR | Branch | Task groups |
|---|---|---|
| 1 | `mac/add-macos-build` | 1 (builds and passes tests, CI job, FFmpeg script) |
| 2 | `mac/add-macos-basics` | 2 (shm, Keychain, stats, Steam, appearance, sounds) |
| 3 | `mac/add-macos-capture` | 3 (ScreenCaptureKit video and audio, cameras, permissions) |
| 4 | `mac/add-macos-encode` | 4 (VideoToolbox) |
| 5 | `mac/add-macos-automation` | 5 (hotkeys, focus, clips) |
| 6 | `mac/add-macos-browser` | 6 (CEF) |
| 7 | `mac/add-macos-package` | 8 (app, dmg, updater, README) |
| 8 | `mac/add-macos-vcam` | 7 (virtual camera, after the signing decision) |

## 0.7.0 parity checklist
Task 9.2 walks through this list on a Mac. "Portable" means the code is shared and only needs
checking.

| Feature (0.7.0) | macOS work |
|---|---|
| Scenes, sources, transforms, undo/redo, command palette | portable |
| Multi-canvas (tabs, duplicate, copy scene, per-canvas Go Live) | portable |
| Inspector (tabs, folding panels, value sliders) | portable |
| First-run wizard (purpose, device detection, chat/alerts/break scenes) | device detection uses the macOS backends |
| Display and window capture | ScreenCaptureKit (group 3) |
| Game capture | unavailable, with a reason (window capture instead) |
| Camera | AVFoundation (group 3) |
| Microphone, desktop audio, per-app audio, capture audio | CoreAudio + ScreenCaptureKit audio (group 3) |
| Image, color, text, media, playlist, slideshow, music sources | portable |
| Video filters, transitions, studio mode | portable (Metal) |
| Sound controls, voice effects, noise suppression (RNNoise, DeepFilterNet) | portable |
| Recording (MKV/MP4/fMP4), remux, multi-track audio | portable + VideoToolbox (group 4) |
| Streaming (RTMP(S), Enhanced RTMP, SRT, WHIP), multistream | portable + VideoToolbox |
| Replay buffer, clips, auto-clip, clip library, clip player, recordings view | portable + foreground game (group 5) |
| Montage editor | portable |
| Browser source, interactive browser, chat overlay, alerts, unified chat panel | CEF (group 6) |
| Account sign-in and stream setup (Twitch, YouTube, Kick) | portable; app IDs exist only in maintainer builds |
| Hotkeys, macros, focus trigger | group 5 |
| Remote API (obs-websocket v5) | portable |
| Stats dock, health | system stats (group 2) |
| Performance profiles, setup switcher | portable |
| Virtual camera | group 7 (gated) |
| Updates | group 8 |
| Stream keys remembered | Keychain (group 2) |
