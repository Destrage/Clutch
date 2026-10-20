# Change: Settings in pages

## Why
Settings is one long scrolling list: stream destinations, recording, the encoder, audio, the
replay buffer, clips and auto-clip, the virtual camera, remote control and appearance, one after
another. Finding a setting means scrolling past everything else, and testers found it hard to
use. Messages around the app already point at "Settings → Stream" or "Settings → Clips" as if
those were places, but there's nothing to click.

## What Changes
- **Pages:** Settings gets a list of pages on the left and shows one page at a time:
  - **Stream:** destinations, each one's editor and connected accounts.
  - **Recording:** folder, file name, canvas, format, remux, the video encoder and audio.
  - **Clips:** replay buffer length, clip mode, the clip hotkey, the clip library, auto-clip and
    hype detection.
  - **Virtual camera:** canvas and the camera component.
  - **Remote control:** the obs-websocket server, port and password.
  - **Appearance:** theme, and the FFmpeg version at the bottom.
- Each page scrolls by itself; the window keeps its size between pages.
- **Opening at the right page:** places that send the user to stream settings (Go Live's
  "Manage…", a missing destination) open the Stream page. Otherwise Settings opens at the page
  that was shown last (the first page the first time).
- Up/Down arrows move between pages when the page list has focus; every page is reachable
  with the keyboard and named for screen readers.

## Impact
- Affected specs: studio-ui.
- Affected code: `apps/estream/ui/overlays.slint` (`SettingsDialog`), `apps/estream/ui/app.slint`
  (the page property), `apps/estream/src/app.rs` (open at the Stream page).
- No settings move or change meaning; nothing is migrated.
