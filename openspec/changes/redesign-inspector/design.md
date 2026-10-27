# Design: redesign-inspector

## Value slider interaction
Blender's number sliders and common audio software disagree on what a click does. A slider
that jumps to the click point makes a stray click change the volume of a live microphone, so
the value slider never jumps:

- **Drag:** the value moves by the distance dragged, scaled so that the bar's width is the
  whole range. The filled edge follows the pointer, so it feels like a slider. Shift divides
  the speed by ten.
- **Click without moving** (under 3 px): the bar becomes a text field with the value selected.
  Enter or leaving the field applies it, Escape cancels.
- **Keys:** with the slider focused, Left/Down and Right/Up step by `step` (Shift: ten steps),
  Enter starts typing.
- Each drag is one undo step: `changed` fires while dragging, `committed` on release, as with
  the existing `Slider` and `NumberField`.
- Values snap to `step` from `minimum`; typed values are clamped to the range.
- **Fill origin:** ranges that cross zero (balance, rotation, brightness, gain, pitch) fill
  from zero, so the bar shows the direction.

## Which numbers become sliders
A number is a slider when its range is known and small enough to drag precisely:
`(maximum - minimum) / step <= 1000`. That takes in every audio filter parameter, every video
filter parameter except Crop/Pad's ±4000 px edges, and source settings such as frame rate, text
size or corner rounding. Width, height, wrap width, tone frequency and durations measured in
thousands of seconds stay number fields (which can still be scrubbed by dragging their label).
Position and size of an item have no range at all and stay number fields.

A video filter parameter from 0 to 1 with step 1 and no choices is a switch.

## Volume
The volume slider runs from -60 dB to +26 dB (the range the engine accepts above silence),
linear in dB, with a tick at 0 dB. At the bottom it shows "-∞ dB" and sets -100 dB, matching
the mixer fader, which treats -60 dB and below as silence.

## State
Tabs and folds live on the window because the inspector is created and destroyed with the
selection: `inspector-tab` (a string) and `inspector-folds`, an `InspectorFolds` struct with
one bool per panel (`true` = folded). The inspector binds both two-way. A struct keeps it to one
property, and a new panel is one new field.

They are saved in `Settings.inspector` (`tab`, and the folded panels as a list of names) when
the app closes and applied on start, like the panel widths. Unknown names are ignored, so
renaming a panel only loses its fold.

Filter panels fold per filter while the inspector is open (local state); they are not saved.

## Tabs
`inspector-tab` holds the chosen tab's name. The shown tab is the chosen one if the selection
has it, otherwise the first one it has, so choosing Sound on a microphone and then clicking a
camera with sound keeps Sound, while clicking an image shows Source or Layout.
