# Tasks: add-studio-mode-transitions

## 1. Model and engine
- [x] 1.1 `Transition` / `TransitionKind` / `Direction` in the project model (serde default Fade 300 ms)
- [x] 1.2 `Command::SetTransition` (undoable) with shader validation
- [x] 1.3 View state: preview scene per canvas, active transitions; `transition_to`,
      `set_preview_scene`; graph generation counter; wake-up at transition end. A plain scene
      change (command or undo) during a transition cuts it.
- [x] 1.4 Graph: transition from-items, preview items, audio union during transitions

## 2. GPU
- [x] 2.1 `TransitionRenderer` with built-in kinds (uber shader)
- [x] 2.2 Custom WGSL transitions: prelude/epilogue, naga validation with mapped line numbers,
      pipeline cache, fade fallback
- [x] 2.3 Video thread: scratch targets, transition pass, studio preview output

## 3. Studio
- [x] 3.1 Studio mode toggle; Preview / Program side by side; editing acts on the Preview
- [x] 3.2 Transition column (Transition, Cut, Fade, picker, duration) and transition bar outside
      studio mode (Fade to color is offered as "Fade to black"; other colors are model-only for now)
- [x] 3.3 Custom shader editor dialog with error display
- [x] 3.4 Palette actions and shortcuts (studio mode Ctrl+Shift+T; transition / cut / shader editor
      in the palette; no global Enter binding to avoid accidental takes)

## 4. Verification
- [x] 4.1 Model tests (defaults, old files, undo)
- [x] 4.2 GPU pixel tests per transition kind; custom shader runs; broken shader error names the line
- [x] 4.3 Engine tests: transition timing, audio union, preview output, cut on plain change
- [x] 4.4 In-app check: studio mode, 3 s fade captured mid-way, custom shader error and apply

## Known issues / follow-ups
- Stinger transitions wait for `add-media-sources`.
- Audio during a transition is a union of both scenes without a volume crossfade.
- Changing the transition duration with the number field records one undo step per change
  (no coalescing yet).
- The first frame after leaving studio mode can briefly show the last preview frame in the main
  view until the next program frame arrives (one frame).
