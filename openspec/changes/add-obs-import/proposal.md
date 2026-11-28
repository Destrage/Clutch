# Change: Import scenes and settings from OBS Studio

## Why
Many people who try Clutch already have their scenes, overlays and output settings in OBS
Studio. Rebuilding them by hand is the biggest reason to give up on the first evening. The
roadmap already lists `add-obs-import`. OBS keeps scene collections as JSON and profiles as
INI files, which can be read as data (clean-room: no OBS code is used or translated).

## What Changes
- **Where:** the first-run wizard gets an **Import from OBS** link under the purpose cards; the
  setup menu in the top bar gets **Import from OBS…**; the command palette has the same action.
- **Finding OBS data:** the default OBS config folder per platform (Windows
  `%APPDATA%\obs-studio`, macOS `~/Library/Application Support/obs-studio`, Linux
  `~/.config/obs-studio` and the Flatpak path). The dialog lists the scene collections and
  profiles it finds. **Choose file…** accepts an exported scene collection `.json`.
- **Each imported scene collection becomes a new setup** with the same name (a suffix when the
  name is taken). Nothing existing is overwritten. Importing is one undoable step for setup
  creation, and the setup can be deleted afterwards like any other.
- **What is mapped:**
  - Canvas size and fps from the profile's video settings (base resolution, fps).
  - Scenes and their order; scene items with position, scale, rotation, crop, bounds, alignment,
    visibility and lock; groups.
  - Sources: display, window and game capture; video capture devices (camera); audio input and
    output capture; application audio; image; color; text; media file; browser source;
    image slideshow; nested scenes. Device and window choices are matched by name to the devices
    this PC has; unmatched ones are kept and flagged.
  - Audio: volume, mute, monitoring, sync offset and track routing.
  - Filters that Clutch has (color correction, chroma/color key, crop/pad, LUT, sharpen,
    image mask, scroll; gain, noise suppression, noise gate, compressor, limiter, expander).
  - Profile: recording folder and format, encoder (mapped to the matching Clutch encoder) and
    bitrate, stream service and server. The stream key is imported only when the user ticks
    **Import my stream key**, and is then stored the same way as other keys.
  - The obs-websocket server port and whether it was enabled (not the password).
- **Import report:** after import, a summary lists what came over and everything skipped or
  changed, with the reason (unknown source type, third-party plugin source or filter, missing
  file, device not found, hotkeys). Skipped sources become placeholders the user can replace
  or delete, so the scene layout stays intact.
- Importing is blocked while streaming or recording, like switching setups.

## Out of scope
- Hotkeys, transitions other than cut/fade/slide/stinger, scripts and third-party plugins
  (listed in the report).
- Streamlabs Desktop and other apps (can follow with the same report format).
- Keeping a setup in sync with OBS after the import.

## Impact
- Affected specs: `obs-import` (new capability).
- Affected code: `crates/estream-project` (new `import::obs` module: parsing, mapping and the
  report, with fixture-based tests), `apps/estream/src` (dialog, setup creation, profile
  mapping into settings), `apps/estream/ui/overlays.slint` and the first-run wizard.
- Shared work: branch `add-obs-import`.
