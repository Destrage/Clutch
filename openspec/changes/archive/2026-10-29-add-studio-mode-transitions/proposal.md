# Change: Studio mode and scene transitions

## Why
Switching scenes is an instant cut today, and every edit happens live on the program. Producers
need to prepare the next scene off-air (studio mode) and change scenes with smooth transitions.
eStream also makes custom transitions a first-class feature: a short
WGSL shader instead of a plugin.

## What Changes
- **transitions** capability (new):
  - Transition types: Cut, Fade (crossfade), Fade to color, Slide (push), Swipe (new scene over
    the old one), Wipe (soft edge) and **Custom shader** (a user WGSL function, validated with
    readable compiler errors).
  - Each type has a direction, color and softness where it applies, plus a duration.
  - The default transition is saved in the scene collection and is undoable.
  - While a transition runs, audio from both scenes is mixed.
- **studio-mode** capability (new):
  - A Preview canvas, rendered only while studio mode is on, is edited off-air next to the
    Program.
  - A Transition button and quick Cut / Fade buttons send Preview to Program with the chosen
    transition.
  - Clicking a scene in studio mode changes the Preview, not the Program.
- Engine:
  - The control thread starts transitions from the engine clock.
  - The video thread renders both scenes and blends them in a transition pass, timed by each
    frame's own timestamp. Results are deterministic and testable.
  - Scratch targets are allocated lazily.
- Studio:
  - a studio mode toggle;
  - side-by-side Preview / Program with a transition column;
  - a transition picker and duration (also usable outside studio mode, where scene clicks
    transition);
  - a custom shader editor with error display;
  - palette actions and shortcuts.

## Out of scope
- Stinger transitions (video with alpha). They need media file playback and come with
  `add-media-sources`.
- Per-scene transition overrides and audio crossfade curves (follow-up).

## Impact
- Affected specs: `transitions` (new), `studio-mode` (new)
- Affected code: `estream-project` (transition model), `estream-engine` (view state, graph,
  transition rendering, preview output), `estream-gpu` (transition renderer, shader validation),
  `apps/estream` (studio UI)
