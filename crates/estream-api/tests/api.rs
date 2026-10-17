//! Request handlers, event translation and the WebSocket server against a headless engine.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_api::events::{StatePoller, Translator};
use estream_api::handlers::{self, Ctx};
use estream_api::protocol::{self, close, op, status};
use estream_api::server::SessionCounters;
use estream_api::{Api, MediaAction, MediaStatus, Output, OutputStatus, Studio};
use estream_core::{Rational, SceneId, SourceId};
use estream_engine::{Command, Engine, EngineConfig};
use estream_project::{Project, Scene, SceneItem, SourceDef};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct FakeStudio {
    engine: Engine,
    studio: bool,
    preview: Option<SceneId>,
    active: [bool; 3],
    last_replay: Option<PathBuf>,
    pressed: Vec<String>,
}

fn index(o: Output) -> usize {
    match o {
        Output::Stream => 0,
        Output::Record => 1,
        Output::ReplayBuffer => 2,
    }
}

impl Studio for FakeStudio {
    fn engine(&self) -> Option<&Engine> {
        Some(&self.engine)
    }
    fn collection_name(&self) -> String {
        "Test".into()
    }
    fn record_directory(&self) -> PathBuf {
        PathBuf::from("C:/rec")
    }
    fn studio_mode(&self) -> bool {
        self.studio
    }
    fn set_studio_mode(&mut self, enabled: bool) {
        self.studio = enabled;
        self.preview = enabled
            .then(|| self.engine.snapshot().canvases[0].active_scene)
            .flatten();
    }
    fn preview_scene(&self) -> Option<SceneId> {
        self.preview
    }
    fn set_preview_scene(&mut self, scene: SceneId) -> Result<(), String> {
        self.preview = Some(scene);
        Ok(())
    }
    fn trigger_studio_transition(&mut self) -> Result<(), String> {
        let canvas = self.engine.snapshot().canvases[0].id;
        let scene = self.preview.ok_or("no preview")?;
        self.engine
            .transition_to(canvas, scene, None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn output_status(&self, output: Output) -> OutputStatus {
        OutputStatus {
            active: self.active[index(output)],
            ..Default::default()
        }
    }
    fn set_output_active(
        &mut self,
        output: Output,
        active: bool,
    ) -> Result<Option<PathBuf>, String> {
        self.active[index(output)] = active;
        Ok((!active && output == Output::Record).then(|| PathBuf::from("C:/rec/out.mkv")))
    }
    fn save_replay(&mut self) -> Result<PathBuf, String> {
        let path = PathBuf::from("C:/rec/replay.mkv");
        self.last_replay = Some(path.clone());
        Ok(path)
    }
    fn last_replay(&self) -> Option<PathBuf> {
        self.last_replay.clone()
    }
    fn last_recording(&self) -> Option<PathBuf> {
        None
    }
    fn media_status(&self, _: SourceId) -> Option<MediaStatus> {
        None
    }
    fn media_action(&mut self, _: SourceId, _: MediaAction) -> Result<(), String> {
        Err("no media".into())
    }
    fn media_seek(&mut self, _: SourceId, _: Duration) -> Result<(), String> {
        Err("no media".into())
    }
    fn press_input_button(&mut self, source: SourceId, property: &str) -> Result<(), String> {
        self.pressed.push(format!("{source}:{property}"));
        Ok(())
    }
}

/// Scenes "Scene" (on program, with a color box) and "Second"; inputs "Box" and "Mic".
fn studio() -> FakeStudio {
    let mut project = Project::new_default(320, 180, Rational::fps(30));
    let canvas = project.canvases[0].id;
    let boxed = SourceDef::new("Box", "color", json!({ "width": 64, "height": 32 }));
    let mic = SourceDef::new("Mic", "audio_test_tone", json!({}));
    project.scenes[0].items.push(SceneItem::source(boxed.id));
    project.scenes.push(Scene::new("Second", canvas));
    project.sources.push(boxed);
    project.sources.push(mic);
    let engine = Engine::start(
        project,
        EngineConfig {
            video: false,
            audio: false,
            ..Default::default()
        },
    )
    .unwrap();
    FakeStudio {
        engine,
        studio: false,
        preview: None,
        active: [false; 3],
        last_replay: None,
        pressed: Vec::new(),
    }
}

fn call(
    ctx: &mut Ctx,
    s: &mut FakeStudio,
    kind: &str,
    data: Value,
) -> Result<Value, (u16, String)> {
    handlers::handle(ctx, s, kind, &data, &SessionCounters::default())
        .map_err(|e| (e.code, e.comment.unwrap_or_default()))
}

#[test]
fn scenes_program_and_preview() {
    let mut s = studio();
    let mut ctx = Ctx::default();
    let list = call(&mut ctx, &mut s, "GetSceneList", json!({})).unwrap();
    assert_eq!(list["currentProgramSceneName"], "Scene");
    let scenes = list["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 2);
    // OBS order: last scene first, index 0 at the bottom.
    assert_eq!(scenes[0]["sceneName"], "Second");
    assert_eq!(scenes[0]["sceneIndex"], 0);
    assert_eq!(scenes[1]["sceneIndex"], 1);

    call(
        &mut ctx,
        &mut s,
        "SetCurrentProgramScene",
        json!({ "sceneName": "Second" }),
    )
    .unwrap();
    let p = s.engine.snapshot();
    assert_eq!(
        p.scene(p.canvases[0].active_scene.unwrap()).unwrap().name,
        "Second"
    );

    let err = call(
        &mut ctx,
        &mut s,
        "SetCurrentProgramScene",
        json!({ "sceneName": "Nope" }),
    )
    .unwrap_err();
    assert_eq!(err.0, status::RESOURCE_NOT_FOUND);
    let err = call(&mut ctx, &mut s, "SetCurrentProgramScene", json!({})).unwrap_err();
    assert_eq!(err.0, status::MISSING_REQUEST_FIELD);

    let err = call(&mut ctx, &mut s, "GetCurrentPreviewScene", json!({})).unwrap_err();
    assert_eq!(err.0, status::STUDIO_MODE_NOT_ACTIVE);
    call(
        &mut ctx,
        &mut s,
        "SetStudioModeEnabled",
        json!({ "studioModeEnabled": true }),
    )
    .unwrap();
    call(
        &mut ctx,
        &mut s,
        "SetCurrentPreviewScene",
        json!({ "sceneName": "Scene" }),
    )
    .unwrap();
    let preview = call(&mut ctx, &mut s, "GetCurrentPreviewScene", json!({})).unwrap();
    assert_eq!(preview["sceneName"], "Scene");
    call(&mut ctx, &mut s, "TriggerStudioModeTransition", json!({})).unwrap();
    let program = call(&mut ctx, &mut s, "GetCurrentProgramScene", json!({})).unwrap();
    assert_eq!(program["currentProgramSceneName"], "Scene");

    let created = call(
        &mut ctx,
        &mut s,
        "CreateScene",
        json!({ "sceneName": "Third" }),
    )
    .unwrap();
    assert!(created["sceneUuid"].is_string());
    let err = call(
        &mut ctx,
        &mut s,
        "CreateScene",
        json!({ "sceneName": "Third" }),
    )
    .unwrap_err();
    assert_eq!(err.0, status::RESOURCE_ALREADY_EXISTS);
    call(
        &mut ctx,
        &mut s,
        "SetSceneName",
        json!({ "sceneName": "Third", "newSceneName": "3rd" }),
    )
    .unwrap();
    call(
        &mut ctx,
        &mut s,
        "RemoveScene",
        json!({ "sceneName": "3rd" }),
    )
    .unwrap();
    assert_eq!(s.engine.snapshot().scenes.len(), 2);
}

#[test]
fn inputs_audio_and_undo() {
    let mut s = studio();
    let mut ctx = Ctx::default();
    let inputs = call(&mut ctx, &mut s, "GetInputList", json!({})).unwrap();
    assert_eq!(inputs["inputs"].as_array().unwrap().len(), 2);
    let colors = call(
        &mut ctx,
        &mut s,
        "GetInputList",
        json!({ "inputKind": "color_source_v3" }),
    )
    .unwrap();
    assert_eq!(colors["inputs"][0]["inputName"], "Box");
    assert_eq!(colors["inputs"][0]["inputKind"], "color_source_v3");

    let toggled = call(
        &mut ctx,
        &mut s,
        "ToggleInputMute",
        json!({ "inputName": "Mic" }),
    )
    .unwrap();
    assert_eq!(toggled["inputMuted"], true);
    assert!(s.engine.snapshot().sources[1].audio.muted);
    // Remote edits go through the command bus, so they can be undone.
    assert!(s.engine.undo().unwrap());
    assert!(!s.engine.snapshot().sources[1].audio.muted);

    let err = call(
        &mut ctx,
        &mut s,
        "GetInputMute",
        json!({ "inputName": "Box" }),
    )
    .unwrap_err();
    assert_eq!(
        err.0,
        status::INVALID_RESOURCE_STATE,
        "color sources have no audio"
    );

    call(
        &mut ctx,
        &mut s,
        "SetInputVolume",
        json!({ "inputName": "Mic", "inputVolumeDb": -6.0 }),
    )
    .unwrap();
    let v = call(
        &mut ctx,
        &mut s,
        "GetInputVolume",
        json!({ "inputName": "Mic" }),
    )
    .unwrap();
    assert!((v["inputVolumeMul"].as_f64().unwrap() - 0.501).abs() < 0.001);
    let err = call(
        &mut ctx,
        &mut s,
        "SetInputVolume",
        json!({ "inputName": "Mic", "inputVolumeDb": 40 }),
    )
    .unwrap_err();
    assert_eq!(err.0, status::REQUEST_FIELD_OUT_OF_RANGE);

    call(
        &mut ctx,
        &mut s,
        "SetInputAudioTracks",
        json!({ "inputName": "Mic", "inputAudioTracks": { "1": false, "3": true } }),
    )
    .unwrap();
    let t = call(
        &mut ctx,
        &mut s,
        "GetInputAudioTracks",
        json!({ "inputName": "Mic" }),
    )
    .unwrap();
    assert_eq!(t["inputAudioTracks"]["1"], false);
    assert_eq!(t["inputAudioTracks"]["3"], true);

    let created = call(
        &mut ctx,
        &mut s,
        "CreateInput",
        json!({
            "sceneName": "Second",
            "inputName": "Red",
            "inputKind": "color_source_v3",
            "inputSettings": { "color": 0xff0000ffu32, "width": 100 },
        }),
    )
    .unwrap();
    assert!(created["sceneItemId"].as_i64().unwrap() > 0);
    let settings = call(
        &mut ctx,
        &mut s,
        "GetInputSettings",
        json!({ "inputName": "Red" }),
    )
    .unwrap();
    assert_eq!(
        settings["inputSettings"]["color"],
        json!({ "r": 255, "g": 0, "b": 0, "a": 255 })
    );
    assert_eq!(settings["inputSettings"]["width"], 100);
    call(
        &mut ctx,
        &mut s,
        "SetInputSettings",
        json!({ "inputName": "Red", "inputSettings": { "height": 50 } }),
    )
    .unwrap();
    let settings = call(
        &mut ctx,
        &mut s,
        "GetInputSettings",
        json!({ "inputName": "Red" }),
    )
    .unwrap();
    assert_eq!(
        settings["inputSettings"]["width"], 100,
        "overlay keeps other settings"
    );
    assert_eq!(settings["inputSettings"]["height"], 50);

    call(
        &mut ctx,
        &mut s,
        "SetInputName",
        json!({ "inputName": "Red", "newInputName": "Crimson" }),
    )
    .unwrap();
    call(
        &mut ctx,
        &mut s,
        "PressInputPropertiesButton",
        json!({ "inputName": "Crimson", "propertyName": "refreshnocache" }),
    )
    .unwrap();
    assert_eq!(s.pressed.len(), 1);
    call(
        &mut ctx,
        &mut s,
        "RemoveInput",
        json!({ "inputName": "Crimson" }),
    )
    .unwrap();
    assert_eq!(s.engine.snapshot().sources.len(), 2);

    let err = call(&mut ctx, &mut s, "NoSuchRequest", json!({})).unwrap_err();
    assert_eq!(err.0, status::UNKNOWN_REQUEST_TYPE);
}

#[test]
fn scene_items_and_transforms() {
    let mut s = studio();
    let mut ctx = Ctx::default();
    let list = call(
        &mut ctx,
        &mut s,
        "GetSceneItemList",
        json!({ "sceneName": "Scene" }),
    )
    .unwrap();
    let item = &list["sceneItems"][0];
    assert_eq!(item["sourceName"], "Box");
    assert_eq!(item["sceneItemIndex"], 0);
    let id = item["sceneItemId"].as_i64().unwrap();
    let found = call(
        &mut ctx,
        &mut s,
        "GetSceneItemId",
        json!({ "sceneName": "Scene", "sourceName": "Box" }),
    )
    .unwrap();
    assert_eq!(found["sceneItemId"], id);

    call(
        &mut ctx,
        &mut s,
        "SetSceneItemEnabled",
        json!({ "sceneName": "Scene", "sceneItemId": id, "sceneItemEnabled": false }),
    )
    .unwrap();
    assert!(!s.engine.snapshot().scenes[0].items[0].visible);
    call(
        &mut ctx,
        &mut s,
        "SetSceneItemTransform",
        json!({
            "sceneName": "Scene",
            "sceneItemId": id,
            "sceneItemTransform": { "positionX": 100.0, "rotation": 45.0, "boundsType": "OBS_BOUNDS_STRETCH", "boundsWidth": 50, "boundsHeight": 20, "cropLeft": 4 },
        }),
    )
    .unwrap();
    let t = call(
        &mut ctx,
        &mut s,
        "GetSceneItemTransform",
        json!({ "sceneName": "Scene", "sceneItemId": id }),
    )
    .unwrap()["sceneItemTransform"]
        .clone();
    assert_eq!(t["positionX"], 100.0);
    assert_eq!(t["rotation"], 45.0);
    assert_eq!(t["boundsType"], "OBS_BOUNDS_STRETCH");
    assert_eq!(t["boundsWidth"], 50.0);
    assert_eq!(t["cropLeft"], 4);
    assert_eq!(t["alignment"], 5);

    let dup = call(
        &mut ctx,
        &mut s,
        "DuplicateSceneItem",
        json!({ "sceneName": "Scene", "sceneItemId": id }),
    )
    .unwrap();
    let dup_id = dup["sceneItemId"].as_i64().unwrap();
    assert_ne!(dup_id, id);
    call(
        &mut ctx,
        &mut s,
        "SetSceneItemIndex",
        json!({ "sceneName": "Scene", "sceneItemId": dup_id, "sceneItemIndex": 0 }),
    )
    .unwrap();
    let index = call(
        &mut ctx,
        &mut s,
        "GetSceneItemIndex",
        json!({ "sceneName": "Scene", "sceneItemId": id }),
    )
    .unwrap();
    assert_eq!(index["sceneItemIndex"], 1);
    call(
        &mut ctx,
        &mut s,
        "RemoveSceneItem",
        json!({ "sceneName": "Scene", "sceneItemId": dup_id }),
    )
    .unwrap();
    let err = call(
        &mut ctx,
        &mut s,
        "GetSceneItemEnabled",
        json!({ "sceneName": "Scene", "sceneItemId": 999 }),
    )
    .unwrap_err();
    assert_eq!(err.0, status::RESOURCE_NOT_FOUND);
}

#[test]
fn transitions_and_outputs() {
    let mut s = studio();
    let mut ctx = Ctx::default();
    call(
        &mut ctx,
        &mut s,
        "SetCurrentSceneTransition",
        json!({ "transitionName": "Slide" }),
    )
    .unwrap();
    call(
        &mut ctx,
        &mut s,
        "SetCurrentSceneTransitionDuration",
        json!({ "transitionDuration": 700 }),
    )
    .unwrap();
    let t = call(&mut ctx, &mut s, "GetCurrentSceneTransition", json!({})).unwrap();
    assert_eq!(t["transitionName"], "Slide");
    assert_eq!(t["transitionKind"], "slide_transition");
    assert_eq!(t["transitionDuration"], 700);
    let err = call(
        &mut ctx,
        &mut s,
        "SetCurrentSceneTransition",
        json!({ "transitionName": "Stinger" }),
    )
    .unwrap_err();
    assert_eq!(err.0, status::INVALID_RESOURCE_STATE);

    call(&mut ctx, &mut s, "StartRecord", json!({})).unwrap();
    let err = call(&mut ctx, &mut s, "StartRecord", json!({})).unwrap_err();
    assert_eq!(err.0, status::OUTPUT_RUNNING);
    let stopped = call(&mut ctx, &mut s, "StopRecord", json!({})).unwrap();
    assert_eq!(stopped["outputPath"], "C:/rec/out.mkv");
    let toggled = call(&mut ctx, &mut s, "ToggleStream", json!({})).unwrap();
    assert_eq!(toggled["outputActive"], true);
    let err = call(&mut ctx, &mut s, "SaveReplayBuffer", json!({})).unwrap_err();
    assert_eq!(err.0, status::OUTPUT_NOT_RUNNING);
}

#[test]
fn snapshot_diffs_become_events() {
    let mut s = studio();
    let mut ctx = Ctx::default();
    let mut tr = Translator::default();
    assert!(
        tr.diff(&s.engine.snapshot(), &s.engine, &mut ctx.ids, true)
            .is_empty()
    );

    let p = s.engine.snapshot();
    let (scene, mic, item) = (p.scenes[0].id, p.sources[1].id, p.scenes[0].items[0].id);
    let mut audio = p.sources[1].audio.clone();
    audio.muted = true;
    s.engine
        .execute(Command::Batch {
            commands: vec![
                Command::RenameScene {
                    scene,
                    name: "Main".into(),
                },
                Command::UpdateSourceAudio { source: mic, audio },
                Command::UpdateItem {
                    scene,
                    item,
                    patch: estream_engine::ItemPatch {
                        visible: Some(false),
                        ..Default::default()
                    },
                },
            ],
        })
        .unwrap();
    let events = tr.diff(&s.engine.snapshot(), &s.engine, &mut ctx.ids, true);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&"SceneNameChanged"), "{kinds:?}");
    assert!(kinds.contains(&"SceneListChanged"), "{kinds:?}");
    assert!(kinds.contains(&"InputMuteStateChanged"), "{kinds:?}");
    assert!(kinds.contains(&"SceneItemEnableStateChanged"), "{kinds:?}");
    let rename = events
        .iter()
        .find(|e| e.kind == "SceneNameChanged")
        .unwrap();
    assert_eq!(rename.data.as_ref().unwrap()["oldSceneName"], "Scene");

    // Undo produces events too.
    s.engine.undo().unwrap();
    let events = tr.diff(&s.engine.snapshot(), &s.engine, &mut ctx.ids, false);
    assert!(
        events.iter().any(|e| e.kind == "InputMuteStateChanged"
            && e.data.as_ref().unwrap()["inputMuted"] == false)
    );

    // App state is polled.
    let mut poller = StatePoller::default();
    assert!(poller.poll(&s).is_empty());
    s.set_studio_mode(true);
    s.active[1] = true;
    let kinds: Vec<&str> = poller.poll(&s).iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&"StudioModeStateChanged"), "{kinds:?}");
    assert!(kinds.contains(&"CurrentPreviewSceneChanged"), "{kinds:?}");
    assert!(kinds.contains(&"RecordStateChanged"), "{kinds:?}");
}

// ---------------------------------------------------------------- WebSocket end to end

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// Runs an API server with a fake studio on its own thread until the returned guard drops.
struct Running {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run_server(password: Option<&str>) -> Running {
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let password = password.map(str::to_owned);
    let thread = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut studio = studio();
            let mut api = Api::start(0, password).unwrap();
            tx.send(api.port()).unwrap();
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                api.tick(&mut studio);
                std::thread::sleep(Duration::from_millis(2));
            }
            api.shutdown();
        })
    };
    Running {
        port: rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        stop,
        thread: Some(thread),
    }
}

fn connect(port: u16, subprotocol: Option<&str>) -> Socket {
    use tungstenite::client::IntoClientRequest;
    let mut request = format!("ws://127.0.0.1:{port}")
        .into_client_request()
        .unwrap();
    if let Some(p) = subprotocol {
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", p.parse().unwrap());
    }
    let (socket, _) = tungstenite::connect(request).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = socket.get_ref() {
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    }
    socket
}

fn send(ws: &mut Socket, value: Value) {
    ws.send(tungstenite::Message::text(value.to_string()))
        .unwrap();
}

/// Next message, or the close code if the server closed the connection.
fn recv(ws: &mut Socket) -> Result<Value, u16> {
    loop {
        match ws.read() {
            Ok(tungstenite::Message::Text(t)) => {
                return Ok(serde_json::from_str(t.as_str()).unwrap());
            }
            Ok(tungstenite::Message::Binary(b)) => return Ok(rmp_serde::from_slice(&b).unwrap()),
            Ok(tungstenite::Message::Close(frame)) => {
                return Err(frame.map_or(0, |f| u16::from(f.code)));
            }
            Ok(_) => continue,
            Err(e) => panic!("socket error: {e}"),
        }
    }
}

/// Reads until a message with `op` arrives (skipping events and other replies).
fn recv_op(ws: &mut Socket, wanted: u64) -> Value {
    loop {
        let msg = recv(ws).unwrap();
        if msg["op"] == wanted {
            return msg;
        }
    }
}

fn identify(ws: &mut Socket, password: Option<&str>) -> Result<Value, u16> {
    let hello = recv(ws).unwrap();
    assert_eq!(hello["op"], op::HELLO);
    assert_eq!(hello["d"]["rpcVersion"], 1);
    let mut d = json!({ "rpcVersion": 1 });
    if let Some(pw) = password {
        let auth = &hello["d"]["authentication"];
        d["authentication"] = protocol::auth_response(
            pw,
            auth["salt"].as_str().unwrap(),
            auth["challenge"].as_str().unwrap(),
        )
        .into();
    }
    send(ws, json!({ "op": op::IDENTIFY, "d": d }));
    recv(ws)
}

#[test]
fn websocket_sessions_end_to_end() {
    let server = run_server(Some("hunter2"));

    // Wrong password.
    let mut ws = connect(server.port, None);
    assert_eq!(
        identify(&mut ws, Some("wrong")),
        Err(close::AUTHENTICATION_FAILED)
    );

    // Requests before Identify.
    let mut ws = connect(server.port, None);
    let _hello = recv(&mut ws).unwrap();
    send(
        &mut ws,
        json!({ "op": op::REQUEST, "d": { "requestType": "GetVersion", "requestId": "x" } }),
    );
    assert_eq!(recv(&mut ws), Err(close::NOT_IDENTIFIED));

    // A good session.
    let mut ws = connect(server.port, Some("obswebsocket.json"));
    let identified = identify(&mut ws, Some("hunter2")).unwrap();
    assert_eq!(identified["op"], op::IDENTIFIED);
    assert_eq!(identified["d"]["negotiatedRpcVersion"], 1);

    send(
        &mut ws,
        json!({ "op": op::REQUEST, "d": { "requestType": "GetVersion", "requestId": "v" } }),
    );
    let reply = recv_op(&mut ws, op::REQUEST_RESPONSE);
    assert_eq!(reply["d"]["requestId"], "v");
    assert_eq!(reply["d"]["requestStatus"]["result"], true);
    assert_eq!(reply["d"]["requestStatus"]["code"], 100);
    assert_eq!(reply["d"]["responseData"]["rpcVersion"], 1);

    // A change made remotely comes back as an event.
    send(
        &mut ws,
        json!({ "op": op::REQUEST, "d": { "requestType": "SetCurrentProgramScene", "requestId": "s", "requestData": { "sceneName": "Second" } } }),
    );
    let mut got_event = false;
    let mut got_reply = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(got_event && got_reply) && Instant::now() < deadline {
        let msg = recv(&mut ws).unwrap();
        if msg["op"] == op::EVENT && msg["d"]["eventType"] == "CurrentProgramSceneChanged" {
            assert_eq!(msg["d"]["eventData"]["sceneName"], "Second");
            assert_eq!(msg["d"]["eventIntent"], 4);
            got_event = true;
        }
        if msg["op"] == op::REQUEST_RESPONSE && msg["d"]["requestId"] == "s" {
            assert_eq!(msg["d"]["requestStatus"]["result"], true);
            got_reply = true;
        }
    }
    assert!(got_event && got_reply);

    // Failed request: status code and comment.
    send(
        &mut ws,
        json!({ "op": op::REQUEST, "d": { "requestType": "SetCurrentProgramScene", "requestId": "bad", "requestData": { "sceneName": "Nope" } } }),
    );
    let reply = recv_op(&mut ws, op::REQUEST_RESPONSE);
    assert_eq!(reply["d"]["requestStatus"]["result"], false);
    assert_eq!(reply["d"]["requestStatus"]["code"], 600);

    // Batch with Sleep, halting on failure.
    let started = Instant::now();
    send(
        &mut ws,
        json!({ "op": op::REQUEST_BATCH, "d": {
            "requestId": "b",
            "haltOnFailure": true,
            "requests": [
                { "requestType": "Sleep", "requestData": { "sleepMillis": 150 } },
                { "requestType": "GetSceneList", "requestId": "list" },
                { "requestType": "GetInputMute", "requestData": { "inputName": "Nope" } },
                { "requestType": "GetVersion" },
            ],
        } }),
    );
    let batch = recv_op(&mut ws, op::REQUEST_BATCH_RESPONSE);
    assert!(started.elapsed() >= Duration::from_millis(150));
    let results = batch["d"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 3, "halted after the failure");
    assert_eq!(results[1]["requestId"], "list");
    assert_eq!(
        results[1]["responseData"]["currentProgramSceneName"],
        "Second"
    );
    assert_eq!(results[2]["requestStatus"]["code"], 600);

    // Reidentify with no subscriptions: no more events.
    send(
        &mut ws,
        json!({ "op": op::REIDENTIFY, "d": { "eventSubscriptions": 0 } }),
    );
    assert_eq!(recv_op(&mut ws, op::IDENTIFIED)["op"], op::IDENTIFIED);
    send(
        &mut ws,
        json!({ "op": op::REQUEST, "d": { "requestType": "SetCurrentProgramScene", "requestId": "t", "requestData": { "sceneName": "Scene" } } }),
    );
    let next = recv(&mut ws).unwrap();
    assert_eq!(
        next["op"],
        op::REQUEST_RESPONSE,
        "no event before the reply: {next}"
    );

    // A second Identify is refused.
    send(
        &mut ws,
        json!({ "op": op::IDENTIFY, "d": { "rpcVersion": 1 } }),
    );
    assert_eq!(recv(&mut ws), Err(close::ALREADY_IDENTIFIED));
}

#[test]
fn msgpack_sessions_and_shutdown() {
    let server = run_server(None);
    let mut ws = connect(server.port, Some("obswebsocket.msgpack"));
    let hello = recv(&mut ws).unwrap();
    assert!(
        hello["d"].get("authentication").is_none(),
        "no password, no challenge"
    );
    let identify = json!({ "op": op::IDENTIFY, "d": { "rpcVersion": 1 } });
    ws.send(tungstenite::Message::binary(
        rmp_serde::to_vec_named(&identify).unwrap(),
    ))
    .unwrap();
    assert_eq!(recv(&mut ws).unwrap()["op"], op::IDENTIFIED);
    let request = json!({ "op": op::REQUEST, "d": { "requestType": "GetStudioModeEnabled", "requestId": 7 } });
    ws.send(tungstenite::Message::binary(
        rmp_serde::to_vec_named(&request).unwrap(),
    ))
    .unwrap();
    let reply = recv_op(&mut ws, op::REQUEST_RESPONSE);
    assert_eq!(reply["d"]["requestId"], 7);
    assert_eq!(reply["d"]["responseData"]["studioModeEnabled"], false);

    // Shutting down: ExitStarted, then close 1001.
    drop(server);
    let mut saw_exit = false;
    let code = loop {
        match recv(&mut ws) {
            Ok(msg) => saw_exit |= msg["d"]["eventType"] == "ExitStarted",
            Err(code) => break code,
        }
    };
    assert!(saw_exit);
    assert_eq!(code, close::GOING_AWAY);
}

#[test]
fn busy_port_is_reported() {
    let held = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = held.local_addr().unwrap().port();
    match Api::start(port, None) {
        Err(estream_api::ServerError::PortInUse(p)) => assert_eq!(p, port),
        Err(e) => panic!("unexpected error: {e}"),
        Ok(_) => panic!("started on a busy port"),
    }
}
