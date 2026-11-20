# Tasks: add-item-transitions

## 1. Model
- [x] 1.1 `MoveSettings` on `Transition`, `ItemAnimations` / `ItemAnimation` / `AnimationKind` / `Ease` on `SceneItem` (serde defaults, skipped when default); old collections load unchanged
- [x] 1.2 Edits through the transition settings command and an `animations` field on `UpdateItem` with undo and coalescing; validation (durations 50..=5000 ms); persistence round-trip test
- [x] 1.3 Remote API: item animations and moves readable and settable; existing obs-websocket requests keep their shape

## 2. Engine
- [x] 2.1 Item matching when a transition starts (by source, list order, opt-out), moving items removed from the background lists; unit tests for pairing, duplicates, groups and opt-out
- [x] 2.2 Ease curves and transform interpolation (decompose, short-way rotation, log scale, arc path, crop, opacity) with unit tests
- [x] 2.3 Show and hide animations as view state: eye toggles start them, the graph keeps a hiding item until it ends, a mid-animation toggle reverses without a jump
- [x] 2.4 Exit and entry animations for unpaired items during scene transitions

## 3. Rendering
- [x] 3.1 Moving pass: draw moving items over the blended background with a load pass; stinger order (under the stinger)
- [x] 3.2 SDF bake with the mask (R16F, 256 px long side), analytic rectangle for unmasked items; bake ahead for every masked item of canvases with moves on
- [x] 3.3 Compositor morph mask mode (two SDFs, weight, feather) and effect layer crossfade
- [x] 3.4 Show and hide animation draws (Fade, Zoom, Slide, Pop, Wipe)
- [x] 3.5 Rendered-frame tests (pixel checks): a camera halfway between two places, rectangle → circle at 0 / 0.5 / 1, masked → unmasked, Zoom and Wipe at half progress

## 4. UI
- [x] 4.1 Transition settings: "Move matching items", ease curve, curved path
- [x] 4.2 Inspector Transitions panel: Show, Hide (kind, direction, duration, ease), "Move between scenes"
- [x] 4.3 Keyboard access and screen-reader labels for the new controls

## 5. Verification
- [x] 5.1 `cargo test` and `cargo clippy --all-targets -- -D warnings` for the touched crates
- [ ] 5.2 Face camera with a rounded mask in "Gameplay" and a circle mask large in "Just Chatting": switch both ways with Fade and Slide while streaming and recording at 1080p60; the camera moves and morphs smoothly in preview, stream and recording, with no extra dropped frames
- [ ] 5.3 Show and hide an overlay with Pop and Slide from the eye button, a hotkey and the remote API
