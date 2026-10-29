# Tasks: add-video-filters

## 1. Model and engine
- [x] 1.1 `FilterDef` on sources (alias for audio), serde skip when empty
- [x] 1.2 `Command::SetSourceFilters` (undoable, coalesced per source, unknown kinds and custom
      code validated)
- [x] 1.3 Filter chains in `VideoCore`: sync, per-frame passes after sources tick, compositor
      draws the chain output
- [x] 1.4 LUT / mask loading on a helper thread into a cache; steps remember which file they hold,
      so parameter changes never reload files

## 2. GPU
- [x] 2.1 Filter catalog (kinds, params, files, code)
- [x] 2.2 Filter renderer: bind groups shared with the compositor's source layout, uber shader,
      blur two-pass, crop/pad sizing
- [x] 2.3 `.cube` parser and 3D LUT upload; image mask upload
- [x] 2.4 Custom shader filters (shared naga validation with transitions). The user function is
      `effect(uv)` because `filter` is a reserved word in WGSL.

## 3. Studio
- [x] 3.1 Inspector Video filters section (add, toggle, reorder, remove, sliders)
- [x] 3.2 File browse for LUT / mask (asked right after adding); shader editor in filter mode

## 4. Verification
- [x] 4.1 GPU pixel tests per kind; custom filter; broken code passes through
- [x] 4.2 `.cube` parser tests
- [x] 4.3 Engine tests: keyed source on canvas; disable restores; crop/pad on canvas; LUT file
      loads off-thread; undo and slider coalescing; validation
- [x] 4.4 In-app check: hue-shift color correction and blur on the test pattern, inspector section

## Known issues / follow-ups
- Chroma key picks the key by hue; an eyedropper on the preview would be friendlier.
- A filter whose file fails to load is skipped silently except for a log line; the inspector
  doesn't show the error yet.
- Every filter pass has its own render target (simple and stable sizes, more memory for long
  chains).
- Filters run for every source with filters, even when it isn't visible in any scene.
