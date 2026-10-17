# Tasks: add-browser-source

## 1. Build
- [x] 1.1 `CEF_PATH` = `third_party/cef`; CEF binaries downloaded and git-ignored
- [x] 1.2 `apps/estream-browser` helper crate links CEF; runtime files next to executables

## 2. Helper
- [x] 2.1 CEF startup (subprocesses, windowless, own message-loop thread, cache path)
- [x] 2.2 Commands over stdin (create/update/reload/close), events over stdout
- [x] 2.3 Offscreen paint → video mapping (seqlock, un-premultiplied)
- [x] 2.4 Page audio → shared ring (48 kHz stereo)
- [x] 2.5 Custom CSS injection; load/error events; exit on stdin EOF

## 3. Host (`estream-capture`)
- [x] 3.1 Shared memory helpers (seqlock frame, audio ring) with tests
- [x] 3.2 `BrowserHost`: spawn helper, stdout reader, page state, restart with back-off
- [x] 3.3 `browser` source: video + audio sides, status, refresh

## 4. Studio
- [x] 4.1 Register the browser source; refresh action in the inspector

## 5. Verification
- [x] 5.1 Unit tests (seqlock, ring)
- [x] 5.2 Helper end-to-end test (frame size and colour; crash → restart)
- [x] 5.3 In-app check with a transparent animated overlay
