## 1. Purpose
- [x] 1.1 `Purpose::Basic` first in `Purpose::ALL`: label, detail, 1080p30 preset, not a live purpose
- [x] 1.2 Part defaults for Basic: webcam off, microphone and desktop audio on, chat, alerts and break scenes off; no game capture line
- [x] 1.3 `build_project` branch: "Main" scene with the screen (and the webcam in the corner when ticked)
- [x] 1.4 Tests: Basic builds only screen, microphone and desktop audio by default; ticked parts are added; the other purposes are unchanged

## 2. Wizard
- [x] 2.1 Five cards: Basic setup across the first row, the others two per row
- [x] 2.2 The wizard opens with Basic setup selected

## 3. Verify
- [x] 3.1 `cargo clippy` and tests for `estream`
- [x] 3.2 In-app check of the wizard with a fresh `CLUTCH_HOME`: card layout, summary, the scene it builds
