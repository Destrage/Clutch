# Tasks: add-video-sources

## 1. Framework
- [x] 1.1 `estream-capture` crate; `SourceType` registry (name, description, schema, factory); `PropertySchema` + validation in the engine (cheap validation schemas never enumerate devices on the control thread)
- [x] 1.2 Generic inspector rendering of `PropertySchema` (bool, int, float, text, multiline, color, path + Browse…, select; refresh button for device/window lists)
- [x] 1.3 Source state: `Active`, `Unavailable(reason)`, `Error(reason)`, read live and re-published by the video thread every 250 ms; shown as a badge in the list and a banner in the inspector

## 2. Windows GPU interop
- [x] 2.1 D3D11 device on wgpu's adapter (matched by LUID), multithread-protected, one per wgpu device
- [x] 2.2 Shared NT-handle textures opened in wgpu's D3D12 device (`texture_from_raw` + `create_texture_from_hal`); 3-slot ring, D3D11 event query before publishing (no keyed mutex needed)
- [ ] 2.3 CPU-staging fallback path when wgpu is not on D3D12 (sources currently report an error instead)

## 3. Sources
- [x] 3.1 Display capture (WGC) + monitor enumeration + cursor toggle; disconnected monitors report "not connected" and retry every second
- [ ] 3.2 DXGI Desktop Duplication fallback (for systems without Windows.Graphics.Capture)
- [x] 3.3 Window capture with match rules (exe+class+title → exe+class → exe+title → exe) and automatic re-acquire
- [ ] 3.4 Game capture (WGC-based; "any fullscreen app" or a specific window) — implemented, not yet verified with a real game
- [x] 3.5 Video capture device (Media Foundation): device list, resolution/FPS/format, MJPEG/YUY2 → NV12 via the source reader, NV12 → RGB on the GPU with BT.601/709 and limited/full range
- [x] 3.6 Image source (PNG/JPEG/WebP/BMP, animated GIF/WebP), decoded on a worker thread, reloads on file change
- [x] 3.7 Color source
- [x] 3.8 Text source (cosmic-text shaping + fallback fonts + color emoji, outline/shadow/gradient, alignment, wrap, read from file with change watch)

## 4. Verification
- [ ] 4.1 Display capture CPU cost test: 1440p60 capture < 1% CPU (not measured yet)
- [ ] 4.2 Automated window re-acquire test (close and relaunch a target app); manually verified across an eStream restart
- [ ] 4.3 Camera hot-unplug/replug test (needs hardware in the loop)
- [x] 4.4 Text rendering tests (Latin, Arabic, CJK, color emoji; outline/shadow pixels) — pixel assertions rather than golden images
- [x] 4.5 Display capture end-to-end test (native resolution frames through the compositor), interop test, NV12 color test, opt-in webcam test
