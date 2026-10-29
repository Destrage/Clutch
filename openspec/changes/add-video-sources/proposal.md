# Change: Add core video sources (Windows first)

## Why
A studio is only useful once it can capture something. These sources cover the most common streaming
setups: screen, window, game, webcam, images and text overlays.

## What Changes
- **video-sources** capability (new):
  - Display capture (Windows.Graphics.Capture, with DXGI Desktop Duplication as fallback), with a
    cursor toggle and a monitor picker that tracks monitors across hot-plug.
  - Window capture (Windows.Graphics.Capture) by window match rules (exe + class + title), so it
    automatically re-acquires the window when the app restarts.
  - Game capture via Windows.Graphics.Capture, with a borderless-fullscreen hint. (Hook-based
    capture is deferred to a later change; see design.)
  - Video capture device (webcam/capture card) via Media Foundation, with resolution/FPS/format selection.
  - Image (PNG/JPEG/WebP/GIF animated), Color, and Text (GPU-rendered, with font, outline, shadow,
    gradient, and read-from-file).
  - The source property schema lets the inspector render settings generically.
  - Zero-copy: captured D3D11 textures are shared to wgpu (D3D12) via shared NT handles.
- Platform abstraction traits, so macOS/Linux backends can plug in later.

## Impact
- Affected specs: `video-sources` (new)
- Affected code: new `crates/estream-capture` (`platform/windows/*`)
- Depends on: `add-engine-core`; UI integration via `add-studio-shell`
