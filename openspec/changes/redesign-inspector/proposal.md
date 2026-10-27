# Change: Inspector in categories, with sliders

## Why
The inspector is one long scroll: name, quick placement, transform, crop, appearance, the whole
audio block (volume, hearing it, voice, balance, mono, all scenes, sync, tracks, filters), the
source's own settings, the page, media playback and video filters, one after another. A
microphone's device picker sits under a dozen audio controls; a camera's chroma key is at the
very bottom. Most values are typed into number boxes: volume in dB, every filter parameter,
the sync offset, a chat overlay's text size. Users asked for it to be grouped the way Blender's
properties editor is, and for sliders where a number box is hard to use.

## What Changes
- **Categories:** the inspector shows the selection's settings in tabs, each only when it
  applies:
  - **Source:** the source's own settings (device, window, file, text…), the browser page and
    media playback.
  - **Layout:** quick placement, position and size, rotation, size mode, and crop.
  - **Look:** opacity, blend mode and video filters.
  - **Sound:** volume and balance, who hears it, voice effects, audio filters, and the rest
    (mono, all scenes, sync offset, tracks) under Advanced.
  The tab strip is hidden when only one tab applies (a microphone without settings). The last
  chosen tab is kept when the selection changes, when the new selection has it.
- **Collapsible panels:** within a tab, related settings sit in panels with a header that
  folds them, as in Blender. Rarely used panels (Crop, Advanced) start folded. Each audio or
  video filter is its own panel with its on/off switch, move and remove buttons in the header.
- **Kept between runs:** the chosen tab and which panels are folded are saved with the window
  layout.
- **Sliders:** bounded values become value sliders: a bar with the name on the left, the value
  on the right, and the bar filled up to the value. Dragging moves the value with the pointer,
  Shift-drag moves it finely, a click without dragging lets the user type an exact value, and
  arrow keys step it. This covers volume (with a mark at 0 dB and -∞ at the bottom), balance,
  rotation, opacity, sync offset, every audio and video filter parameter, and source settings
  with a small range (frame rate, text size, corner rounding, alert volume). Values with very
  large ranges (width, height, crop, frequency, durations) keep number fields.
- **Clearer choices:**
  - Who hears a sound becomes one three-way choice: Stream only, Stream + me, Only me.
  - Voice presets become buttons instead of a dropdown.
  - Filter choices with two or three options (noise suppression Light/Strong) become a
    segmented choice.
  - An on/off filter parameter (luma key "Use brightness") becomes a switch.
  - Quick placement becomes a small canvas picture with the corners, center and full-screen
    spots to click.
- **Header:** the name, type, and Visible/Locked as eye and lock buttons stay above the tabs,
  so they are always in reach.
- The Live deck's sound settings use the same Sound panels.

## Impact
- Affected specs: studio-ui.
- Affected code: `apps/estream/ui/widgets.slint` (value slider, panel, segmented choice),
  `apps/estream/ui/panels.slint` (`InspectorPanel`, `PropEditor`), `apps/estream/ui/sound.slint`
  (`AudioSettings`), `apps/estream/ui/deck.slint`, `apps/estream/ui/app.slint` (tab and fold
  state), `apps/estream/src/settings.rs` and `apps/estream/src/app.rs` (saving them).
- No settings move or change meaning; nothing is migrated. Older settings files load with the
  default tab and panel state.
