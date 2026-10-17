# Change: Browser source (out-of-process CEF)

## Why
Alerts, chat boxes, donation goals, scoreboards and most stream widgets are web pages. OBS embeds
CEF in its own process, so a misbehaving page can freeze or crash the stream. eStream renders
pages in a separate helper process. If it crashes, the stream keeps going and the pages come
back.

## What Changes
- **browser-source** capability (new): a `browser` source type.
  - Settings: URL or local HTML file, width, height, frame rate, custom CSS (default: transparent
    page background).
  - Transparent pages composite over the scene. Page audio goes to the mixer.
  - A refresh action reloads the page.
- **Helper process** `estream-browser.exe`:
  - Chromium Embedded Framework (CEF 154, offscreen rendering).
  - One process for all browser sources; it exits when eStream closes it or eStream exits.
  - Frames are shared through named shared memory, and page audio through a shared ring buffer.
  - Commands go over stdin and events come back over stdout.
- **Crash isolation:** if the helper dies, browser sources show "Browser restarting…". The host
  restarts it after a short delay and re-creates every page. The rest of the studio and every
  output keep running.
- **Build:**
  - CEF binaries are downloaded once into the git-ignored `third_party/cef` (`CEF_PATH`).
  - Only the helper links CEF.
  - The runtime files are copied next to the executables.

## Out of scope
- Interacting with pages (clicks, typing) from the studio.
- Hardware-accelerated shared-texture paint; frames are copied through shared memory.
- macOS and Linux.

## Impact
- Affected specs: `browser-source` (new)
- Affected code:
  - new `apps/estream-browser` (helper);
  - `estream-capture` (browser source, host manager, shared memory);
  - `.cargo/config.toml` (`CEF_PATH`).
