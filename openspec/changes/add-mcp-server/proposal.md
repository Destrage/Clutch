# Change: Control Clutch from AI assistants (MCP server)

## Why
People want to drive Clutch from AI assistants and agents ("switch to my BRB scene", "clip the
last 30 seconds", "why are frames dropping?"). Those clients speak the Model Context Protocol
(MCP), not obs-websocket. Clutch already answers remote requests through the `Studio` trait and
the command bus (`estream-api`), so an MCP endpoint can reuse the same paths and stay
consistent with Stream Deck, hotkeys and macros.

## What Changes
- **MCP server** built into the app, **off by default**, under Settings → Remote control →
  **AI assistants (MCP)**:
  - Streamable HTTP transport on `http://127.0.0.1:<port>/mcp` (localhost only, default port
    4466), with a bearer token generated on first enable. **Copy connection** puts a ready
    config snippet (URL + token) on the clipboard for Claude, Cursor and similar clients.
  - `clutch --mcp-stdio` runs a small stdio bridge that relays to the running app, for clients
    that only launch stdio servers. It exits with a clear error when Clutch isn't running or MCP
    is off.
- **Tools** (each one maps to an existing request or command, so undo and events behave the
  same as from the UI):
  - Read: `get_status` (live/recording/replay state, current setup, scene and canvas, dropped
    frames, CPU, encoder), `list_setups`, `list_canvases`, `list_scenes`, `list_sources`,
    `get_stats`, `get_health` (current health alerts and their hints).
  - Scene and sources: `switch_scene`, `set_source_visible`, `set_text` (text sources),
    `set_volume`, `set_mute`.
  - Clips: `save_replay` (clip the buffer), `add_marker`.
  - Outputs: `start_recording`, `stop_recording`, `start_streaming`, `stop_streaming`.
- **Safety:**
  - Starting/stopping streaming and stopping a recording are marked destructive (MCP tool
    annotations) and are refused unless **Allow going live and stopping outputs** is ticked
    (off by default). The error says where to turn it on.
  - Stream keys, OAuth tokens and the obs-websocket password are never returned by any tool.
  - Every MCP action is logged with the client's name, and a small "AI assistant connected"
    indicator shows in the status bar while a client is connected.
- Tool results are short plain-language JSON, shaped for a model to read (names, not ids alone).

## Out of scope
- Adding, removing or arranging sources and editing filters from MCP (later change; needs a
  stable, model-friendly description of source settings).
- Clutch acting as an MCP *client*.
- Remote (non-localhost) access.

## Impact
- Affected specs: `remote-api` (ADDED: MCP Server, MCP Tools, MCP Safety).
- Affected code: `crates/estream-api` (new `mcp` module on the API's I/O thread, using the
  official Rust MCP SDK `rmcp`, requests answered in `Api::tick` like obs-websocket),
  `apps/estream/src/settings.rs` (enable, port, token, allow-outputs),
  `apps/estream/src/main.rs` (`--mcp-stdio`), `apps/estream/ui/overlays.slint` (settings rows),
  status bar indicator.
- Shared work: branch `add-mcp-server`.
