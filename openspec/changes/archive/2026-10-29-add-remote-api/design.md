# Design: add-remote-api

## Threads
- **Server thread:**
  - A current-thread `tokio` runtime: a listener, then one task per client
    (`tokio-tungstenite`).
  - It does I/O only. It never touches the engine or the UI.
- **App (UI thread):**
  - Owns the request handlers, because outputs, studio mode and the preview scene live in the
    app.
  - The app tick (every 4 ms) drains pending requests, handles them, and sends the replies back
    through a oneshot channel.
- **Real-time threads** are untouched. Meter levels are read from the existing lock-free
  `Meter`s.

```
client ⇄ ws task ── PendingRequest ──▶ app tick: Api::process(&mut dyn Studio)
                 ◀── oneshot reply ───┘        │
       ◀──────── broadcast(Event) ◀──── Api::tick: snapshot diff, output/studio polling
```

## Crate `estream-api`
- **`protocol`:**
  - OpCodes, close codes, request status codes and event subscription bits;
  - `Encoding` (JSON / MessagePack);
  - `auth_response(password, salt, challenge)` = base64(sha256(base64(sha256(password + salt)) + challenge)).
- **`server`** — `Server::start(port, password)`:
  - binds `0.0.0.0:port` (start errors are reported to the app);
  - `pending()` lists the queued requests;
  - `broadcast(event)` sends an event to each client whose subscription includes its intent;
  - `wants(intent)` reports whether any client subscribed to that intent;
  - `client_count()`;
  - `shutdown()` sends `ExitStarted`, then closes with 1001.
- **Session rules:**
  - Any message other than Identify before identification → close 4007.
  - A second Identify → 4008.
  - A wrong password → 4009.
  - An RPC version other than 1 → 4010.
  - Malformed JSON → 4002.
  - An unknown op → 4006.
- **Batches:**
  - Executed in order on the app thread (`SerialRealtime`, `SerialFrame`).
  - `Sleep` is handled in the session task: `sleepMillis`, or `sleepFrames` × the frame time.
  - `haltOnFailure` is honoured.
  - `Parallel` runs serially; `Sleep` is rejected in a parallel batch.
- **`handlers`:**
  - `handle(&mut Ctx, &dyn Studio, type, data) -> Result<Value, RequestError>`, with
    `RequestError { code, comment }`.
  - Shared field helpers (`req_str`, `req_bool`, …) produce the matching OBS status codes:
    - 300 MissingRequestField;
    - 401 InvalidRequestFieldType;
    - 402 RequestFieldOutOfRange;
    - 600 ResourceNotFound.
- **`events`:**
  - `Translator` keeps the previous `Arc<Project>` and diffs it against the new snapshot when the
    revision changes. Every origin (UI, undo, remote) produces the same events.
  - **Scenes:**
    - `SceneCreated`, `SceneRemoved`, `SceneNameChanged`, `SceneListChanged`;
    - `CurrentProgramSceneChanged` (main canvas).
  - **Inputs:**
    - `InputCreated`, `InputRemoved`, `InputNameChanged`, `InputSettingsChanged`;
    - `InputMuteStateChanged`, `InputVolumeChanged`, `InputAudioBalanceChanged`;
    - `InputAudioSyncOffsetChanged`, `InputAudioTracksChanged`, `InputAudioMonitorTypeChanged`.
  - **Scene items:**
    - `SceneItemCreated`, `SceneItemRemoved`, `SceneItemListReindexed`;
    - `SceneItemEnableStateChanged`, `SceneItemLockStateChanged`;
    - `SceneItemTransformChanged` (high volume).
  - **Transitions and filters:**
    - `CurrentSceneTransitionChanged`, `CurrentSceneTransitionDurationChanged`;
    - `SourceFilterEnableStateChanged`.
  - **App state**, polled every tick:
    - studio mode → `StudioModeStateChanged`;
    - preview scene → `CurrentPreviewSceneChanged`;
    - output active flags → `StreamStateChanged`, `RecordStateChanged`,
      `ReplayBufferStateChanged`;
    - replay saves → `ReplayBufferSaved`.
  - `InputVolumeMeters`: every 50 ms, only while some client subscribes to it.
    `inputLevelsMul` = `[rms, peak, peak]` per channel.

## The `Studio` trait (implemented by the app)

```rust
pub trait Studio {
    fn engine(&self) -> Option<&Engine>;
    fn studio_mode(&self) -> bool;
    fn set_studio_mode(&mut self, on: bool);
    fn preview_scene(&self) -> Option<SceneId>;
    fn set_preview_scene(&mut self, scene: SceneId) -> Result<(), String>;
    fn trigger_studio_transition(&mut self) -> Result<(), String>;
    fn output_status(&self, output: Output) -> OutputStatus;
    fn set_output_active(&mut self, output: Output, active: bool) -> Result<(), String>;
    fn save_replay(&mut self) -> Result<(), String>;
    fn last_replay(&self) -> Option<PathBuf>;
    fn record_directory(&self) -> PathBuf;
    fn collection_name(&self) -> String;
}
```

Scene, input, item, transition and filter requests use `engine()` and Commands directly, so
handlers stay testable with a headless engine and a fake `Studio`.

## Identifiers
- **Scenes:** `sceneName` is the scene name, and `sceneUuid` is the `SceneId`. Either can be
  given; the UUID wins.
- **Inputs:** `inputName` / `inputUuid` refer to a `SourceDef` (any kind).
- **Scene items:**
  - `sceneItemId` is an integer, assigned the first time an `ItemId` is seen and kept for the
    session. Groups are treated as items.
  - `sceneItemIndex` counts from the bottom, as in OBS. eStream also stores items bottom-first,
    so the index is the position in the list.
- **Transforms:**
  - `positionX/Y`, `rotation`, `scaleX/Y`, `width`/`height` (scaled), `sourceWidth`/
    `sourceHeight`, `boundsType` (`OBS_BOUNDS_NONE` / `OBS_BOUNDS_SCALE_INNER` /
    `OBS_BOUNDS_STRETCH`), `boundsWidth/Height`, `crop*` and `alignment` 5 (top-left).
  - eStream's bounds modes map onto these.
- **Volume:**
  - `inputVolumeDb` is `volume_db`, and `inputVolumeMul` = 10^(dB/20). Muted is separate.
  - Range checks match OBS: dB −100…26, mul 0…20.
- **Transitions:**
  - Names are the kind labels (Cut, Fade, …).
  - Setting one switches the kind and keeps the duration. Kinds that need settings (Stinger,
    Shader) keep their stored settings when they are the current kind; otherwise they're
    rejected.
- **Input kinds:** eStream's source kind ids (`display_capture`, `browser`, `media`, …). OBS
  kind names (`browser_source`, `ffmpeg_source`, `image_source`, `color_source_v3`,
  `text_gdiplus_v3`, …) are accepted as aliases in `CreateInput` and `GetInputList`.

## Settings

```rust
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    pub auth: bool,
    pub password_protected: String,
}
```

The default is: disabled, port 4455, auth on, and a random 16-character password generated
the first time the server is enabled. The UI shows the password masked, with a reveal button. A
setting change restarts the server; connected clients get close 4011 (session invalidated).

## Testing
- Protocol unit tests: the auth vector, codes, and Hello/Identify round trips.
- Handler tests on a headless engine (no video/audio threads) with a fake `Studio`: scenes,
  inputs, items, transforms and volume.
- Event diff tests: renames, mute and item visibility produce the right events.
- An end-to-end test with a `tungstenite` client against the real server:
  - auth success and failure (4009);
  - a request round trip;
  - batch with `Sleep`;
  - an event received after a change;
  - MessagePack subprotocol.
- In-app check: enable the server in Settings, then drive it with a scripted client (scene
  switch, mute, record start/stop) and watch the UI follow.
