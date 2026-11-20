# Change: Polish for 0.10.0

## Why
The 0.10.0 review (PR #23) fixed what was broken and left a list of things that work but read or
behave worse than they should. 0.10.0 is meant to be a clean release, so these go in first.

## What Changes
- **Filter values as percentages.** Video filter settings that are fractions (opacity,
  similarity, smoothness, spill, low/high, LUT strength, brightness, contrast, sharpen) show and
  take percentages ("100%", "-25%") like the rest of the app, instead of "1.00". Stored values
  are unchanged.
- **Settings that depend on a switch.** A source setting can name the switch it belongs to; it
  is shown only while that switch is on (Text: gradient end color under Gradient, outline color
  and width under Outline, the shadow settings under Drop shadow).
- **Borders follow a morphing outline.** While a moving item changes shape, its border is drawn
  from the in-between outline every frame, so it hugs the shape the whole way instead of fading
  out and back in. Glow and shadow keep handing over (they are soft).
- **The preview isn't hidden by its panels.** With the scene items panel or the inspector open,
  the canvas is fitted into the space beside them (as long as that leaves it a useful size).
- **Lists keep their rows across refreshes.** Lists whose rows hold controls and that background
  work refreshes (the settings form's lists, the Mask panel's shapes) keep their models and change only rows that
  differ, so an open dropdown or a dragged slider survives a refresh caused by something else
  (the remote API, a macro, a scene switch).
- **Transition card** says when "Move matching items" is on.

## Impact
- Affected specs: `video-filters` (value display), `studio-ui` (dependent settings, preview fit,
  stable lists, transition card), `transitions` (border during a morph)
- Affected code: `estream-gpu` (filter parameter percent flag, compositor border mode),
  `estream-engine` (property visibility condition, morph border draw), `estream-capture` (text
  source schema), `apps/estream` (parameter editor, preview fit, kept models, transition card)
- No change to saved data.
