# Tasks: simplify-first-run-and-go-live

## 1. Streaming presets (`estream-media`)
- [x] 1.1 `Service::recommended_kbps(width, height, fps)` from each platform's guidance
- [x] 1.2 `Service::key_page()` (where to find the key) and `Service::from_key()` (detect by format)
- [x] 1.3 Unit tests

## 2. First run
- [x] 2.1 Purposes (gaming, camera, recording, vertical) → canvas and scene layout
- [x] 2.2 Device detection summary (screen, webcam, mic, encoder) in the wizard
- [x] 2.3 Scene built from detected devices; mic noise suppression on; presets behind a toggle

## 3. Go Live
- [x] 3.1 Go Live box: platform tiles, key field with reveal, "where's my key", auto settings line
- [x] 3.2 Key detection and mistake guards; Go Live saves and starts; More options → Settings
- [x] 3.3 Opens instead of Settings when there's no destination; "Add platform" in the live panel

## 4. Verification
- [x] 4.1 Unit tests (bitrates, key detection, purpose scenes)
- [x] 4.2 In-app: fresh first run for each purpose; Go Live box from an empty setup
