# Change: Remote control API (obs-websocket v5 compatible)

## Why
Stream Deck, Touch Portal, Streamer.bot, chat bots and many web dashboards speak
the obs-websocket v5 protocol. If eStream speaks the same protocol, these tools work without
changes, and people keep the control setups they already have.

## What Changes
- **remote-api** capability (new): a WebSocket server that speaks obs-websocket protocol v5
  (RPC version 1). It is off by default, uses port 4455, and supports an optional password.
  - **Protocol:**
    - Hello, Identify, Identified and Reidentify.
    - Password authentication with SHA-256 challenges.
    - Event subscriptions.
    - Requests and request batches (serial, plus `Sleep`).
    - JSON and MessagePack encodings, chosen by WebSocket subprotocol.
    - OBS close codes and request status codes.
  - **Requests** — the set used by common controllers:
    - general and stats;
    - scenes and program/preview;
    - studio mode;
    - inputs (settings, mute, volume, balance, sync, monitoring, tracks);
    - transitions;
    - scene items (list, id, enabled, locked, index, transform);
    - stream, record and replay buffer;
    - media inputs;
    - source filters (list, enable).
  - **Events:**
    - scene, input, scene-item, transition and filter changes, from every origin (UI, undo, remote);
    - output state;
    - studio mode;
    - `InputVolumeMeters` (high volume, 50 ms), plus `ExitStarted` on shutdown.
- **Mapping to eStream's model:**
  - Scenes and inputs are addressed by name or UUID.
  - Scene item ids are integers, stable for the session.
  - The program is the first (main) canvas. Setting a scene on another canvas switches that
    canvas.
- **All mutations go through the command bus**, so remote edits can be undone and appear in the
  UI. Outputs and studio mode go through the same app actions as the buttons.
- **Settings:**
  - a "Remote control" section to enable the server and set the port and password;
  - the password is stored encrypted (DPAPI), like stream keys;
  - a status line shows the address and the number of connected clients.

## Out of scope
- `GetSourceScreenshot` / `SaveSourceScreenshot`, virtual camera, hotkeys, profiles and switching
  scene collections, vendor requests, and filter creation and editing. These requests return
  "unknown request type" (or "not supported" when they are known) until the features exist.
- Canvas selection in requests (obs-websocket canvases).
- `GetStats` reports 0 for CPU use, memory use and free disk space.

## Impact
- Affected specs: `remote-api` (new)
- Affected code:
  - new `crates/estream-api` (protocol, server, request handlers, event translation);
  - `apps/estream` (settings section, wiring the server to the app tick and actions);
  - `apps/estream/src/settings.rs`.
