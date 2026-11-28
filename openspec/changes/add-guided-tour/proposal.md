# Change: A guided tour for first-time users

## Why
The first-run wizard builds a working setup, but new users then face the studio without
knowing what a setup, a canvas, a scene or a source is, or how they relate. Testers asked for a
walkthrough that starts from the top level (one setup per game, for example "Apex") and goes
down to canvases, scenes and sources, and that people who already know their way around can
skip.

## What Changes
- **Tour** that highlights each real part of the studio in turn: the rest is dimmed, a card next
  to the highlighted area explains it, with **Back**, **Next**, a step count ("3 of 9") and
  **Skip tour**. Esc also skips. Steps, top level first:
  1. **Setups** (top-bar pill): "A setup holds everything for one kind of stream: scenes,
     sources and audio. Make one per game, like *Apex* with its capture and overlays, and switch
     between them here."
  2. **Canvases** (canvas tabs): "A canvas is one output picture: 16:9 for Twitch or YouTube,
     9:16 for TikTok and Shorts. Each canvas has its own scenes, and both can go live at once."
  3. **Scenes** (scene strip): "Scenes are layouts you switch between: Gameplay, Starting soon,
     Be right back."
  4. **Sources** (sources panel): "Sources are what a scene shows and plays: the game, your
     webcam, chat, alerts. The top of the list is drawn in front. Add more with +."
  5. **Preview**: "Drag to move, pull the corners to resize. Items snap to edges and to each
     other."
  6. **Inspector**: "Settings for whatever is selected: what it captures, its look, filters."
  7. **Sound** (mixer): "Click a knob to mute, drag to set the volume, click the name for its
     settings."
  8. **Record and clip** (deck): "Record, keep a replay buffer and save the last moments as a
     clip with one key."
  9. **Go Live**: "Pick where to stream and go live. Settings → Performance tunes Clutch for this
     PC, and Ctrl+K finds any action."
- **When it runs:** after a new user finishes (or skips) the first-run wizard, a card asks
  **Show me around** / **Skip, I know my way around**. Finishing or skipping is remembered, so it
  never starts on its own again. Users updating from an older version are not interrupted.
  **Take the tour** in the command palette and the help menu starts it any time.
- **Follows the real layout:** a step whose area is hidden (for example a collapsed panel) opens
  it first; a step whose area doesn't exist (single canvas, no deck) is skipped. Steps on the
  setup or canvas use the user's own names ("You're in *My setup*").
- **Never in the way:** the tour doesn't start, and closes, when streaming or recording starts.
  Nothing in the studio changes because of the tour.
- **Accessible:** keyboard only (Tab/Enter/arrow keys), screen-reader labels on the card, and
  it follows interface zoom and text size.

## Out of scope
- Interactive tasks inside the tour ("now add a source"), and video tutorials.
- Tours for individual dialogs (Settings, Go Live).

## Impact
- Affected specs: `onboarding` (ADDED: Guided Tour, Skipping and Restarting the Tour).
- Affected code: `apps/estream/ui/tour.slint` (overlay with spotlight and card),
  `apps/estream/ui/app.slint` (targets report their absolute position and size to the overlay),
  `apps/estream/src/app.rs` (steps, state, open hidden panels), `apps/estream/src/settings.rs`
  (`tour_done`), command palette entry.
- Shared work: branch `add-guided-tour`.
