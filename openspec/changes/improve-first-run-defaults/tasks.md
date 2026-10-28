# Tasks: improve-first-run-defaults

## 1. Setup parts (`presets.rs`)
- [x] 1.1 Optional parts with per-purpose defaults; choices that remember what the user changed
- [x] 1.2 Summary lines carry their part and whether it's on
- [x] 1.3 Chat overlay, alerts, and "Starting soon" / "Be right back" scenes in the built setup
- [x] 1.4 Left-out parts are not added; an unticked webcam falls back to the screen

## 2. Wizard
- [x] 2.1 Checkboxes on optional lines; toggling updates the list
- [x] 2.2 Choices reset when the wizard opens and are used by Get started

## 3. Verification
- [x] 3.1 Unit tests (defaults per purpose, left-out parts, extra scenes)
- [x] 3.2 In-app: each purpose with boxes ticked and unticked
