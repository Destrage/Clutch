# remote-api Specification

## Purpose
Remote control over the obs-websocket v5 protocol, so Stream Deck, Touch Portal and similar tools can drive Clutch: requests, events and settings.

## Requirements

### Requirement: obs-websocket v5 Compatible Server
The system SHALL provide an optional WebSocket server that implements obs-websocket protocol
v5 (RPC version 1). This covers the Hello/Identify handshake, password authentication, event
subscriptions, requests, request batches, and the JSON and MessagePack encodings, so that
existing obs-websocket clients can control eStream without modification.

#### Scenario: Authenticated connection
- **WHEN** the server has a password and a client answers the Hello challenge correctly
- **THEN** the client receives Identified with negotiated RPC version 1

#### Scenario: Wrong password
- **WHEN** a client identifies with a wrong authentication string
- **THEN** the connection closes with code 4009

#### Scenario: Request before identification
- **WHEN** a client sends a Request before Identify
- **THEN** the connection closes with code 4007

### Requirement: Remote Requests
The server SHALL support the requests commonly used by controllers: scenes, program and
preview, studio mode, inputs (including mute and volume), transitions, scene items, stream,
record, replay buffer, media inputs and filter enable state. It SHALL return OBS request status
codes. Every change SHALL go through the command bus, so it appears in the studio and can be
undone.

#### Scenario: Switch scene remotely
- **WHEN** a client sends `SetCurrentProgramScene` with an existing scene name
- **THEN** that scene goes on program using the current transition, and the studio shows it

#### Scenario: Unknown scene
- **WHEN** a client sends `SetCurrentProgramScene` with a name that doesn't exist
- **THEN** the response has `result: false` and code 600 (ResourceNotFound)

#### Scenario: Mute from a controller
- **WHEN** a client sends `ToggleInputMute` for the microphone
- **THEN** the microphone is muted in the mixer, and Undo unmutes it

### Requirement: Remote Events
The server SHALL send events to the clients subscribed to them. The events cover changes made
anywhere (the UI, undo, other clients): scenes, inputs, scene items, transitions, filters,
outputs and studio mode. Volume meters SHALL be sent only to clients that subscribe to the
high-volume `InputVolumeMeters` event.

#### Scenario: UI change reaches the controller
- **WHEN** the user switches scene in the studio
- **THEN** subscribed clients receive `CurrentProgramSceneChanged` with the new scene name

### Requirement: Remote Control Settings
Remote control SHALL be off by default and configurable in Settings: enabled, port (default
4455) and password. The password SHALL be stored encrypted. Settings SHALL show the address
clients connect to and how many are connected.

#### Scenario: Port in use
- **WHEN** the server is enabled on a port another program already uses
- **THEN** Settings shows that the port is unavailable, and the rest of the studio keeps working
