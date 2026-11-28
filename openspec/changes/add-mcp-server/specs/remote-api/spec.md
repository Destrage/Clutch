## ADDED Requirements

### Requirement: MCP Server
The system SHALL provide an optional Model Context Protocol server, off by default, reachable
only from the local machine over Streamable HTTP and protected by a bearer token. The system
SHALL also offer a stdio bridge mode that relays to the running app. Settings SHALL offer a way
to copy the connection details.

#### Scenario: Assistant connects
- **WHEN** MCP is enabled and a client connects to the local endpoint with the right token
- **THEN** the client can list Clutch's tools and the status bar shows that an AI assistant is connected

#### Scenario: Wrong or missing token
- **WHEN** a client connects without the token or with a wrong one
- **THEN** the request is rejected with HTTP 401 and no tool runs

#### Scenario: Stdio bridge without the app
- **WHEN** `clutch --mcp-stdio` starts while Clutch is not running or MCP is off
- **THEN** it exits with an error that says to open Clutch and enable AI assistants

### Requirement: MCP Tools
The MCP server SHALL offer tools to read status, setups, canvases, scenes, sources, stats and
health alerts, and to switch scenes, show or hide sources, set text, set volume and mute, save
a replay clip, add a marker, and start or stop recording and streaming. Every tool that changes
state SHALL go through the command bus, as the same action does in the UI.

#### Scenario: Switch scene
- **WHEN** an assistant calls `switch_scene` with the name "BRB"
- **THEN** the program scene switches to BRB and remote clients receive the same scene-changed event as for a UI switch

#### Scenario: Unknown scene
- **WHEN** an assistant calls `switch_scene` with a name that doesn't exist
- **THEN** the tool returns an error listing the scene names that do exist

### Requirement: MCP Safety
Starting or stopping a stream and stopping a recording through MCP SHALL be refused unless
the user has allowed it in Settings (off by default). No MCP tool SHALL return stream keys,
account tokens or passwords. Every MCP action SHALL be logged with the client's name.

#### Scenario: Going live not allowed
- **WHEN** an assistant calls `start_streaming` and going live from MCP is not allowed
- **THEN** nothing starts and the tool returns an error saying where to allow it

#### Scenario: Secrets stay hidden
- **WHEN** an assistant reads status, stats or any list
- **THEN** no stream key, token or password appears in the result
