# Tasks: fix-game-capture-stability

## 1. Process Priority
- [x] 1.1 Set Above Normal priority class on Windows during process initialization in `apps/estream/src/main.rs`

## 2. Game Capture Target Latching
- [x] 2.1 Keep acquired game target in `CaptureSource` across focus changes while the target window/process remains alive
- [x] 2.2 Re-acquire foreground target when the latched window is closed or its process terminates
- [x] 2.3 List minimized windows in the window pickers (fullscreen games minimize while another window has focus)

## 3. Device Removal Diagnostics
- [x] 3.1 Report detailed removal reason code when a DXGI device removed error occurs

## 4. Preview Viewport Scroll Control
- [x] 4.1 Add `scroll_pan` boolean property to `Settings` (default `false`) and expose it to the Slint UI
- [x] 4.2 In `preview.slint`, guard mouse wheel canvas panning behind `scroll-pan` so unmodified wheel does not pan by default
- [x] 4.3 Add a "Mouse wheel pans preview" toggle in the Settings Appearance page

## 5. Verification
- [x] 5.1 Run unit tests with `cargo test --workspace`
- [x] 5.2 Run linter with `cargo clippy --workspace --all-targets`
