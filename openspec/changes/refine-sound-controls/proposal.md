# Change: Sound knobs and a closable inspector

## Why
Media audio reaches the stream but the streamer can't hear it, because every source starts with
monitoring off, and turning it on makes Desktop Audio capture it a second time.
The Live deck's tall faders take a lot of room for what they do, and the Stage only offers mute
buttons, so changing a volume means opening the item's settings. The sound buttons are already
round: making each one a knob gives volume, mute and settings in one compact control, the same in
both views. The item settings panel also has no way to close it other than clicking elsewhere.

## What Changes
- **Sound knobs** replace the Live deck faders and the Stage mute buttons:
  - Click: mute or unmute. Drag up/down (or scroll): volume. Double-click: back to 0 dB.
  - The ring shows the volume; the live level glows inside it; muted knobs are dimmed with a
    crossed-out speaker.
  - The value (in dB) shows while dragging; the name sits under the knob.
- **Name opens the settings:** clicking a source's name under its knob opens that source's audio
  settings (volume, filters, monitoring) — in the Stage's settings panel, and in a panel over the
  Live deck while live.
- **Close button** on the settings panel (Stage and Live deck); Escape closes it too.
- **Hearing your own media:** media files and browser sources start with monitoring on ("Monitor
  and output"), so the streamer hears what viewers hear; other sources keep it off. Each sound's
  settings get a plain "Hear on my speakers" toggle.
- **No double audio:** Desktop Audio leaves out Clutch's own playback (its process tree), so
  monitored sounds aren't captured again. Windows can leave out only one app at a time; while a
  game capture is leaving out the game, a monitored source shows a warning that Desktop Audio
  will pick it up.

## Impact
- Affected specs: studio-ui.
- Affected code: `apps/estream/ui` (new `SoundKnob` widget, `deck.slint`, `stage.slint`,
  inspector header), `apps/estream/src/app.rs` (open audio settings from the deck, close).
- Knobs use the existing volume, mute and commit commands (one undo step per drag).
- `estream-capture` (Desktop Audio leaves out Clutch's own process when no game is left out),
  source defaults for media and browser sources.
