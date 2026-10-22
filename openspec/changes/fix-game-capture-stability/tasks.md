# Tasks: fix-game-capture-stability

## 1. Process Priority
- [ ] 1.1 Set Above Normal priority class on Windows during process initialization in `apps/estream/src/main.rs`

## 2. Game Capture Target Latching
- [ ] 2.1 Keep acquired game target in `CaptureSource` across focus changes while the target window/process remains alive
- [ ] 2.2 Re-acquire foreground target when the latched window is closed or its process terminates

## 3. Device Removal Diagnostics
- [ ] 3.1 Report detailed removal reason code when a DXGI device removed error occurs

## 4. Verification
- [ ] 4.1 Run unit tests with `cargo test --workspace`
- [ ] 4.2 Run linter with `cargo clippy --workspace --all-targets`
