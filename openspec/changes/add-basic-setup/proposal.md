# Change: Basic setup in the first-run wizard

## Why
The first-run wizard offers four purposes (streaming games, talking to camera, recording,
vertical video). Each one builds a fairly complete setup: game capture, a webcam in the corner,
and, for the live purposes, chat, alerts and break scenes. Testers asked for a plain starting
point that sets up the basics and lets them add the rest themselves. Nothing is selected for
someone who doesn't know yet what they want, so the wizard opens on "Streaming games" and the
heaviest setup.

## What Changes
- A fifth purpose, **Basic setup**: "Your screen, microphone and desktop audio. Add the rest
  later." It builds one scene named "Main" with the primary screen, and adds the microphone with
  noise suppression and desktop audio.
- Its optional parts default to: microphone and desktop audio on; webcam, chat overlay, alerts
  and break scenes off (game capture isn't offered). The user can tick any of them, as with
  the other purposes.
- Its canvas is 1080p · 30 fps, which is light for encoders and connections. The resolution
  list can still change it.
- **Basic setup is the first card and is selected when the wizard opens.** The purpose cards
  become five: Basic setup spans the first row and the other four keep their two-per-row grid.
- The performance profile and encoder are chosen as they are now. The basic setup has no
  special case there.

## Impact
- Affected specs: `onboarding` (MODIFIED: Purpose-Based First Run, Optional Setup Parts)
- Affected code: `apps/estream/src/presets.rs` (`Purpose::Basic`, its summary, defaults and
  `build_project` branch), `apps/estream/src/app.rs` (default selection),
  `apps/estream/ui/overlays.slint` (card layout)
- Shared work: branch `add-basic-setup`
