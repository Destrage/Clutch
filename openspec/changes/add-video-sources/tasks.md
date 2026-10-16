# Tasks: add-video-sources

## 1. Framework
- [ ] 1.1 `estream-capture` crate; `SourceFactory` registry; `PropertySchema` + validation
- [ ] 1.2 Generic inspector rendering of `PropertySchema` (in `apps/estream`)
- [ ] 1.3 Source state: `Active`, `Unavailable(reason)`, `Error(reason)`, surfaced in UI

## 2. Windows GPU interop
- [ ] 2.1 D3D11 device on wgpu's adapter (match LUID)
- [ ] 2.2 Shared NT-handle texture pool + keyed mutex; wrap as `wgpu::Texture`
- [ ] 2.3 CPU-staging fallback path

## 3. Sources
- [ ] 3.1 Display capture (WGC) + monitor enumeration/hot-plug + cursor toggle
- [ ] 3.2 DXGI Desktop Duplication fallback
- [ ] 3.3 Window capture with match rules (exe/class/title priority) + re-acquire
- [ ] 3.4 Game capture (WGC-based) with game-window picker
- [ ] 3.5 Video capture device (Media Foundation): device list, resolution/FPS/format, YUV shader
- [ ] 3.6 Image source (PNG/JPEG/WebP, animated GIF/WebP)
- [ ] 3.7 Color source
- [ ] 3.8 Text source (cosmic-text, outline/shadow/gradient, read from file with change watch)

## 4. Verification
- [ ] 4.1 Display capture CPU cost test: 1440p60 capture < 1% CPU
- [ ] 4.2 Window re-acquire test (close and relaunch target app)
- [ ] 4.3 Camera hot-unplug/replug test
- [ ] 4.4 Text golden-image tests (Latin, CJK, emoji, RTL)
