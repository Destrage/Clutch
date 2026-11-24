## 1. Detection
- [x] 1.1 HDR state per monitor: color space and peak from `IDXGIOutput6::GetDesc1`, SDR white level from `DisplayConfigGetDeviceInfo` (with the 200-nit fallback)
- [x] 1.2 Resolve the monitor for window and game capture; re-check once a second off the frame thread
- [x] 1.3 Log what was detected when a capture starts and when it changes

## 2. Capture format
- [x] 2.1 Frame pool in `R16G16B16A16Float` on HDR monitors, `B8G8R8A8` otherwise; recreate on change
- [x] 2.2 Shared texture ring in `R16G16B16A16_FLOAT` / `Rgba16Float` for HDR captures
- [x] 2.3 "HDR screens" option on display, window and game capture (Automatic / As Windows gives them), saved with the source

## 3. Mapping
- [x] 3.1 `estream-gpu` HDR-to-SDR pass (WGSL) with the uniform set from detection
- [x] 3.2 Capture sources run the pass for HDR frames and hand the compositor the linear result
- [x] 3.3 Curve unit tests and the headless GPU test against the CPU reference

## 4. Verify
- [x] 4.1 `cargo clippy` and tests for `estream-capture`, `estream-gpu`, `estream`
- [x] 4.2 Headless end-to-end test: 16-bit capture of a real monitor through the mapping pass (forced HDR); SDR capture still works in the app
- [ ] 4.3 Check on an HDR monitor: desktop, an Auto HDR game, an HDR game; HDR toggled while capturing
