# Tasks: fix-gpu-contention

## 1. Video Thread Watchdog
- [x] 1.1 Add `VideoProgress` (stage, stage start, ticked source, done) to `estream-engine::video` and update it through the video loop and `tick_sources`
- [x] 1.2 Start a detached watchdog thread next to the video thread that logs stalls over 2 s (stage + source name) and recoveries, and exits when the video thread finishes
- [x] 1.3 Unit-test stall and recovery detection with a fake progress source

## 2. GPU Scheduling Priority
- [ ] 2.1 Set the process GPU scheduling class on Windows (HIGH with HAGS, REALTIME otherwise; skipped on Intel; debug log on failure)
- [ ] 2.2 Set `SetGPUThreadPriority` on the D3D11 capture interop device

## 3. Separate Engine Device (Windows)
- [ ] 3.1 Create the UI and engine `GpuContext`s on the same adapter (LUID match); hand the UI one to Slint and the engine one to the main engine
- [ ] 3.2 Add shared preview textures on the engine device, opened on the UI device
- [ ] 3.3 Add the shared fence pair: engine signals after preview writes; UI waits before sampling and signals after presenting
- [ ] 3.4 Switch the canvas preview and program images to bridge textures
- [ ] 3.5 Switch scene thumbnails to bridge textures with the copy recorded on the engine device
- [ ] 3.6 Fall back to one shared device when sharing fails, and on Linux/macOS
- [ ] 3.7 Audit every other use of the app's `GpuContext` for the right device

## 4. Verification
- [ ] 4.1 Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`
- [ ] 4.2 Resize, maximize and restore the window while streaming: no `GpuWaitTimeout` in the log and the UI is drawn at the right size
- [ ] 4.3 Run a fullscreen game with Game capture for 10+ minutes on the Windows 10 test machine: output fps holds, no watchdog stall, clean "engine stopped" on exit
