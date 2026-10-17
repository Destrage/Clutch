//! Request handlers. They run on the app thread; every change goes through the command bus.

use crate::protocol::{self, status as st};
use crate::server::{Reply, RequestError, SessionCounters};
use crate::studio::{MediaAction, MediaState, Output, Studio};
use crate::views::{self, ItemIds};
use estream_core::{BlendMode, Crop, ItemId, SceneId, SourceId};
use estream_engine::{Command, Engine, ItemPatch};
use estream_project::{
    Canvas, ItemContent, Project, Scene, SceneItem, SourceDef, Transition, TransitionKind,
};
use serde_json::{Map, Value, json};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

/// API state that outlives single requests.
#[derive(Debug, Default)]
pub struct Ctx {
    pub ids: ItemIds,
    /// Frames per second the main canvas actually renders (measured by the API tick).
    pub active_fps: f64,
}

/// Every request type eStream answers (for `GetVersion`).
pub const REQUESTS: &[&str] = &[
    "GetVersion",
    "GetStats",
    "BroadcastCustomEvent",
    "Sleep",
    "GetHotkeyList",
    "TriggerHotkeyByName",
    "GetSceneCollectionList",
    "GetProfileList",
    "GetVideoSettings",
    "GetRecordDirectory",
    "GetSceneList",
    "GetGroupList",
    "GetCurrentProgramScene",
    "SetCurrentProgramScene",
    "GetCurrentPreviewScene",
    "SetCurrentPreviewScene",
    "CreateScene",
    "RemoveScene",
    "SetSceneName",
    "GetInputList",
    "GetInputKindList",
    "GetSpecialInputs",
    "CreateInput",
    "RemoveInput",
    "SetInputName",
    "GetInputDefaultSettings",
    "GetInputSettings",
    "SetInputSettings",
    "GetInputMute",
    "SetInputMute",
    "ToggleInputMute",
    "GetInputVolume",
    "SetInputVolume",
    "GetInputAudioBalance",
    "SetInputAudioBalance",
    "GetInputAudioSyncOffset",
    "SetInputAudioSyncOffset",
    "GetInputAudioMonitorType",
    "SetInputAudioMonitorType",
    "GetInputAudioTracks",
    "SetInputAudioTracks",
    "PressInputPropertiesButton",
    "GetTransitionKindList",
    "GetSceneTransitionList",
    "GetCurrentSceneTransition",
    "SetCurrentSceneTransition",
    "SetCurrentSceneTransitionDuration",
    "TriggerStudioModeTransition",
    "GetSourceActive",
    "GetSourceFilterList",
    "GetSourceFilter",
    "SetSourceFilterEnabled",
    "GetSceneItemList",
    "GetGroupSceneItemList",
    "GetSceneItemId",
    "GetSceneItemSource",
    "CreateSceneItem",
    "RemoveSceneItem",
    "DuplicateSceneItem",
    "GetSceneItemTransform",
    "SetSceneItemTransform",
    "GetSceneItemEnabled",
    "SetSceneItemEnabled",
    "GetSceneItemLocked",
    "SetSceneItemLocked",
    "GetSceneItemIndex",
    "SetSceneItemIndex",
    "GetSceneItemBlendMode",
    "SetSceneItemBlendMode",
    "GetVirtualCamStatus",
    "StartVirtualCam",
    "StopVirtualCam",
    "ToggleVirtualCam",
    "GetStreamStatus",
    "ToggleStream",
    "StartStream",
    "StopStream",
    "GetRecordStatus",
    "ToggleRecord",
    "StartRecord",
    "StopRecord",
    "GetReplayBufferStatus",
    "ToggleReplayBuffer",
    "StartReplayBuffer",
    "StopReplayBuffer",
    "SaveReplayBuffer",
    "GetLastReplayBufferReplay",
    "GetMediaInputStatus",
    "SetMediaInputCursor",
    "OffsetMediaInputCursor",
    "TriggerMediaInputAction",
    "GetStudioModeEnabled",
    "SetStudioModeEnabled",
];

type E = RequestError;

fn not_found(what: impl Into<String>) -> E {
    RequestError::new(st::RESOURCE_NOT_FOUND, what)
}

fn failed(e: impl std::fmt::Display) -> E {
    RequestError::new(st::REQUEST_PROCESSING_FAILED, e.to_string())
}

/// Typed access to `requestData` with OBS status codes for bad fields.
struct Args<'a>(&'a Value);

impl<'a> Args<'a> {
    fn get(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    fn missing(key: &str) -> E {
        RequestError::new(st::MISSING_REQUEST_FIELD, format!("missing `{key}`"))
    }

    fn wrong_type(key: &str, ty: &str) -> E {
        RequestError::new(
            st::INVALID_REQUEST_FIELD_TYPE,
            format!("`{key}` must be {ty}"),
        )
    }

    fn opt_str(&self, key: &str) -> Result<Option<&'a str>, E> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .map(Some)
                .ok_or_else(|| Self::wrong_type(key, "a string")),
        }
    }

    fn str(&self, key: &str) -> Result<&'a str, E> {
        let s = self.opt_str(key)?.ok_or_else(|| Self::missing(key))?;
        if s.is_empty() {
            return Err(RequestError::new(
                st::REQUEST_FIELD_EMPTY,
                format!("`{key}` is empty"),
            ));
        }
        Ok(s)
    }

    fn opt_bool(&self, key: &str) -> Result<Option<bool>, E> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_bool()
                .map(Some)
                .ok_or_else(|| Self::wrong_type(key, "a boolean")),
        }
    }

    fn bool(&self, key: &str) -> Result<bool, E> {
        self.opt_bool(key)?.ok_or_else(|| Self::missing(key))
    }

    fn opt_f64(&self, key: &str, min: f64, max: f64) -> Result<Option<f64>, E> {
        let Some(v) = self.get(key) else {
            return Ok(None);
        };
        let n = v
            .as_f64()
            .ok_or_else(|| Self::wrong_type(key, "a number"))?;
        if !(min..=max).contains(&n) {
            return Err(RequestError::new(
                st::REQUEST_FIELD_OUT_OF_RANGE,
                format!("`{key}` must be between {min} and {max}"),
            ));
        }
        Ok(Some(n))
    }

    fn f64(&self, key: &str, min: f64, max: f64) -> Result<f64, E> {
        self.opt_f64(key, min, max)?
            .ok_or_else(|| Self::missing(key))
    }

    fn i64(&self, key: &str, min: i64, max: i64) -> Result<i64, E> {
        let v = self.get(key).ok_or_else(|| Self::missing(key))?;
        let n = v
            .as_i64()
            .ok_or_else(|| Self::wrong_type(key, "an integer"))?;
        if !(min..=max).contains(&n) {
            return Err(RequestError::new(
                st::REQUEST_FIELD_OUT_OF_RANGE,
                format!("`{key}` must be between {min} and {max}"),
            ));
        }
        Ok(n)
    }

    fn object(&self, key: &str) -> Result<&'a Map<String, Value>, E> {
        self.get(key)
            .ok_or_else(|| Self::missing(key))?
            .as_object()
            .ok_or_else(|| Self::wrong_type(key, "an object"))
    }
}

fn parse_uuid(key: &str, s: &str) -> Result<uuid::Uuid, E> {
    uuid::Uuid::parse_str(s)
        .map_err(|_| RequestError::new(st::INVALID_REQUEST_FIELD, format!("`{key}` isn't a UUID")))
}

fn engine(studio: &dyn Studio) -> Result<&Engine, E> {
    studio
        .engine()
        .ok_or_else(|| RequestError::new(st::NOT_READY, "no scene collection is open"))
}

fn exec(engine: &Engine, command: Command) -> Reply {
    engine.execute(command).map(|_| Value::Null).map_err(failed)
}

fn main_canvas(p: &Project) -> Result<&Canvas, E> {
    p.canvases
        .first()
        .ok_or_else(|| RequestError::new(st::NOT_READY, "the collection has no canvas"))
}

fn find_scene<'p>(
    p: &'p Project,
    a: &Args,
    name_key: &str,
    uuid_key: &str,
) -> Result<&'p Scene, E> {
    if let Some(u) = a.opt_str(uuid_key)? {
        let id = SceneId(parse_uuid(uuid_key, u)?);
        return p
            .scene(id)
            .ok_or_else(|| not_found(format!("no scene with UUID {u}")));
    }
    let name = a.opt_str(name_key)?.ok_or_else(|| {
        RequestError::new(
            st::MISSING_REQUEST_FIELD,
            format!("missing `{name_key}` or `{uuid_key}`"),
        )
    })?;
    p.scenes
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| not_found(format!("no scene named “{name}”")))
}

fn scene<'p>(p: &'p Project, a: &Args) -> Result<&'p Scene, E> {
    find_scene(p, a, "sceneName", "sceneUuid")
}

/// A scene, or a group addressed like a scene (OBS treats groups as scenes). Returns the
/// owning scene and the group item.
fn scene_or_group<'p>(p: &'p Project, a: &Args) -> Result<(&'p Scene, Option<&'p SceneItem>), E> {
    match scene(p, a) {
        Ok(s) => Ok((s, None)),
        Err(e) if e.code == st::RESOURCE_NOT_FOUND => {
            let name = a.opt_str("sceneName")?;
            let uuid = a.opt_str("sceneUuid")?;
            for s in &p.scenes {
                let mut found = None;
                s.walk(&mut |item| {
                    if found.is_none()
                        && item.is_group()
                        && (uuid.is_some_and(|u| item.id.to_string() == u)
                            || (uuid.is_none() && item.name.as_deref() == name))
                    {
                        found = Some(item);
                    }
                });
                if let Some(group) = found {
                    return Ok((s, Some(group)));
                }
            }
            Err(e)
        }
        Err(e) => Err(e),
    }
}

fn container_items<'p>(scene: &'p Scene, group: Option<&'p SceneItem>) -> &'p [SceneItem] {
    match group.map(|g| &g.content) {
        Some(ItemContent::Group { items }) => items,
        _ => &scene.items,
    }
}

fn find_input<'p>(
    p: &'p Project,
    a: &Args,
    name_key: &str,
    uuid_key: &str,
) -> Result<&'p SourceDef, E> {
    if let Some(u) = a.opt_str(uuid_key)? {
        let id = SourceId(parse_uuid(uuid_key, u)?);
        return p
            .source(id)
            .ok_or_else(|| not_found(format!("no input with UUID {u}")));
    }
    let name = a.opt_str(name_key)?.ok_or_else(|| {
        RequestError::new(
            st::MISSING_REQUEST_FIELD,
            format!("missing `{name_key}` or `{uuid_key}`"),
        )
    })?;
    p.sources
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| not_found(format!("no input named “{name}”")))
}

fn input<'p>(p: &'p Project, a: &Args) -> Result<&'p SourceDef, E> {
    find_input(p, a, "inputName", "inputUuid")
}

fn source<'p>(p: &'p Project, a: &Args) -> Result<&'p SourceDef, E> {
    find_input(p, a, "sourceName", "sourceUuid")
}

fn item<'p>(p: &'p Project, ids: &ItemIds, a: &Args) -> Result<(&'p Scene, &'p SceneItem), E> {
    let (scene, _) = scene_or_group(p, a)?;
    let n = a.i64("sceneItemId", 0, i64::MAX)?;
    let id = ids
        .item(n)
        .ok_or_else(|| not_found(format!("no scene item {n}")))?;
    let item = scene
        .find_item(id)
        .ok_or_else(|| not_found(format!("scene item {n} isn't in “{}”", scene.name)))?;
    Ok((scene, item))
}

fn audio_input<'p>(engine: &Engine, p: &'p Project, a: &Args) -> Result<&'p SourceDef, E> {
    let s = input(p, a)?;
    if !engine
        .registry()
        .get(&s.kind)
        .is_some_and(|t| t.has_audio())
    {
        return Err(RequestError::new(
            st::INVALID_RESOURCE_STATE,
            format!("“{}” has no audio", s.name),
        ));
    }
    Ok(s)
}

fn update_audio(
    engine: &Engine,
    s: &SourceDef,
    change: impl FnOnce(&mut estream_project::AudioConfig),
) -> Reply {
    let mut audio = s.audio.clone();
    change(&mut audio);
    exec(
        engine,
        Command::UpdateSourceAudio {
            source: s.id,
            audio,
        },
    )
}

fn update_item(engine: &Engine, scene: &Scene, item: &SceneItem, patch: ItemPatch) -> Reply {
    exec(
        engine,
        Command::UpdateItem {
            scene: scene.id,
            item: item.id,
            patch,
        },
    )
}

fn scene_ref(s: Option<&Scene>, prefix: &str) -> Value {
    json!({
        format!("{prefix}SceneName"): s.map(|s| s.name.clone()),
        format!("{prefix}SceneUuid"): s.map(|s| s.id.to_string()),
    })
}

/// Light translation of OBS setting names for kinds that have an eStream equivalent.
fn import_settings(kind: &str, mut settings: Map<String, Value>) -> Map<String, Value> {
    match kind {
        "browser" => {
            if let Some(local) = settings.get("is_local_file").and_then(Value::as_bool) {
                let file = if local {
                    settings.get("local_file").cloned().unwrap_or_default()
                } else {
                    Value::String(String::new())
                };
                settings.insert("file".into(), file);
            }
        }
        "media" => {
            if let Some(file) = settings.get("local_file").cloned() {
                settings.insert("file".into(), file);
            }
            if let Some(looping) = settings.get("looping").cloned() {
                settings.insert("loop".into(), looping);
            }
        }
        "color" => {
            // OBS stores colors as 0xAABBGGRR integers.
            if let Some(abgr) = settings.get("color").and_then(Value::as_u64) {
                settings.insert(
                    "color".into(),
                    json!({
                        "r": abgr & 0xff,
                        "g": (abgr >> 8) & 0xff,
                        "b": (abgr >> 16) & 0xff,
                        "a": (abgr >> 24) & 0xff,
                    }),
                );
            }
        }
        _ => {}
    }
    settings
}

fn output_json(studio: &dyn Studio, output: Output) -> Value {
    let s = studio.output_status(output);
    match output {
        Output::Stream => json!({
            "outputActive": s.active,
            "outputReconnecting": s.reconnecting,
            "outputTimecode": views::timecode(s.duration),
            "outputDuration": s.duration.as_millis() as u64,
            "outputCongestion": 0.0,
            "outputBytes": s.bytes,
            "outputSkippedFrames": s.skipped_frames,
            "outputTotalFrames": s.total_frames,
        }),
        Output::Record => json!({
            "outputActive": s.active,
            "outputPaused": false,
            "outputTimecode": views::timecode(s.duration),
            "outputDuration": s.duration.as_millis() as u64,
            "outputBytes": s.bytes,
        }),
        Output::ReplayBuffer | Output::VirtualCam => json!({ "outputActive": s.active }),
    }
}

fn set_output(
    studio: &mut dyn Studio,
    output: Output,
    active: bool,
) -> Result<Option<std::path::PathBuf>, E> {
    let current = studio.output_status(output).active;
    if current == active {
        return Err(RequestError::new(
            if active {
                st::OUTPUT_RUNNING
            } else {
                st::OUTPUT_NOT_RUNNING
            },
            if active {
                "the output is already running"
            } else {
                "the output isn't running"
            },
        ));
    }
    studio
        .set_output_active(output, active)
        .map_err(|e| RequestError::new(st::REQUEST_PROCESSING_FAILED, e))
}

fn transition_json(t: &Transition) -> Value {
    let (name, kind) = views::transition_names(&t.kind);
    let mut settings = serde_json::to_value(&t.kind).unwrap_or_default();
    if let Some(map) = settings.as_object_mut() {
        map.remove("type");
    }
    json!({
        "transitionName": name,
        "transitionUuid": Value::Null,
        "transitionKind": kind,
        "transitionFixed": matches!(t.kind, TransitionKind::Cut),
        "transitionDuration": if matches!(t.kind, TransitionKind::Cut) { Value::Null } else { t.duration_ms.into() },
        "transitionConfigurable": !matches!(t.kind, TransitionKind::Cut | TransitionKind::Fade),
        "transitionSettings": settings,
    })
}

/// True if `source` is visible in `scene` (inside visible groups).
fn shows(scene: &Scene, source: SourceId) -> bool {
    fn any(items: &[SceneItem], source: SourceId) -> bool {
        items.iter().any(|i| {
            i.visible
                && match &i.content {
                    ItemContent::Source { source: s } => *s == source,
                    ItemContent::Group { items } => any(items, source),
                }
        })
    }
    any(&scene.items, source)
}

/// Copies an item with fresh ids (groups included).
fn fresh_copy(item: &SceneItem) -> SceneItem {
    let mut copy = item.clone();
    copy.id = ItemId::new();
    if let ItemContent::Group { items } = &mut copy.content {
        *items = items.iter().map(fresh_copy).collect();
    }
    copy
}

/// Handles one request.
pub fn handle(
    ctx: &mut Ctx,
    studio: &mut dyn Studio,
    request_type: &str,
    data: &Value,
    session: &SessionCounters,
) -> Reply {
    let a = Args(data);
    match request_type {
        // ------------------------------------------------------------ general
        "GetVersion" => Ok(json!({
            "obsVersion": protocol::OBS_VERSION,
            "obsWebSocketVersion": protocol::OBS_WEBSOCKET_VERSION,
            "rpcVersion": protocol::RPC_VERSION,
            "availableRequests": REQUESTS,
            "supportedImageFormats": [],
            "platform": std::env::consts::OS,
            "platformDescription": format!("eStream {}", env!("CARGO_PKG_VERSION")),
            "estreamVersion": env!("CARGO_PKG_VERSION"),
        })),
        "GetStats" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let output = main_canvas(&p)
                .ok()
                .and_then(|c| engine.canvas_output(c.id));
            let (rendered, lagged, render_ns) = output.map_or((0, 0, 0), |o| {
                (
                    o.stats.rendered.load(Relaxed),
                    o.stats.lagged.load(Relaxed),
                    o.stats.last_render_ns.load(Relaxed),
                )
            });
            let stream = studio.output_status(Output::Stream);
            let record = studio.output_status(Output::Record);
            Ok(json!({
                "cpuUsage": 0.0,
                "memoryUsage": 0.0,
                "availableDiskSpace": 0.0,
                "activeFps": ctx.active_fps,
                "averageFrameRenderTime": render_ns as f64 / 1e6,
                "renderSkippedFrames": lagged,
                "renderTotalFrames": rendered,
                "outputSkippedFrames": stream.skipped_frames + record.skipped_frames,
                "outputTotalFrames": stream.total_frames + record.total_frames,
                "webSocketSessionIncomingMessages": session.incoming.load(Relaxed),
                "webSocketSessionOutgoingMessages": session.outgoing.load(Relaxed),
            }))
        }
        // eStream's macros stand in for OBS hotkeys (Stream Deck "hotkey" buttons).
        "GetHotkeyList" => Ok(json!({ "hotkeys": studio.macro_names() })),
        "TriggerHotkeyByName" => {
            let name = a.str("hotkeyName")?;
            studio
                .run_macro(name)
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::RESOURCE_NOT_FOUND, e))
        }
        "GetSceneCollectionList" => {
            let name = studio.collection_name();
            Ok(json!({ "currentSceneCollectionName": name, "sceneCollections": [name] }))
        }
        "GetProfileList" => Ok(json!({ "currentProfileName": "Default", "profiles": ["Default"] })),
        "GetVideoSettings" => {
            let p = engine(studio)?.snapshot();
            let c = main_canvas(&p)?;
            Ok(json!({
                "fpsNumerator": c.fps.num,
                "fpsDenominator": c.fps.den,
                "baseWidth": c.width,
                "baseHeight": c.height,
                "outputWidth": c.width,
                "outputHeight": c.height,
            }))
        }
        "GetRecordDirectory" => Ok(json!({
            "recordDirectory": studio.record_directory().to_string_lossy(),
        })),

        // ------------------------------------------------------------ scenes
        "GetSceneList" => {
            let p = engine(studio)?.snapshot();
            let program = views::main_program(&p);
            let preview = studio
                .studio_mode()
                .then(|| studio.preview_scene().and_then(|id| p.scene(id)))
                .flatten();
            let mut out = json!({
                "currentProgramSceneName": program.map(|s| s.name.clone()),
                "currentProgramSceneUuid": program.map(|s| s.id.to_string()),
                "currentPreviewSceneName": preview.map(|s| s.name.clone()),
                "currentPreviewSceneUuid": preview.map(|s| s.id.to_string()),
            });
            out["scenes"] = views::scene_list(&p).into();
            Ok(out)
        }
        "GetGroupList" => {
            let p = engine(studio)?.snapshot();
            let mut groups = Vec::new();
            for s in &p.scenes {
                s.walk(&mut |i| {
                    if i.is_group() {
                        groups.push(i.name.clone().unwrap_or_else(|| "Group".into()));
                    }
                });
            }
            Ok(json!({ "groups": groups }))
        }
        "GetCurrentProgramScene" => {
            let p = engine(studio)?.snapshot();
            let s = views::main_program(&p);
            let mut out = scene_ref(s, "currentProgram");
            out["sceneName"] = s.map(|s| s.name.clone()).into();
            out["sceneUuid"] = s.map(|s| s.id.to_string()).into();
            Ok(out)
        }
        "SetCurrentProgramScene" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = scene(&p, &a)?;
            engine
                .transition_to(s.canvas, s.id, None)
                .map(|_| Value::Null)
                .map_err(failed)
        }
        "GetCurrentPreviewScene" => {
            if !studio.studio_mode() {
                return Err(RequestError::new(
                    st::STUDIO_MODE_NOT_ACTIVE,
                    "studio mode is off",
                ));
            }
            let p = engine(studio)?.snapshot();
            let s = studio.preview_scene().and_then(|id| p.scene(id));
            let mut out = scene_ref(s, "currentPreview");
            out["sceneName"] = s.map(|s| s.name.clone()).into();
            out["sceneUuid"] = s.map(|s| s.id.to_string()).into();
            Ok(out)
        }
        "SetCurrentPreviewScene" => {
            if !studio.studio_mode() {
                return Err(RequestError::new(
                    st::STUDIO_MODE_NOT_ACTIVE,
                    "studio mode is off",
                ));
            }
            let p = engine(studio)?.snapshot();
            let id = scene(&p, &a)?.id;
            studio
                .set_preview_scene(id)
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::REQUEST_PROCESSING_FAILED, e))
        }
        "CreateScene" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let name = a.str("sceneName")?;
            if p.scenes.iter().any(|s| s.name == name) {
                return Err(RequestError::new(
                    st::RESOURCE_ALREADY_EXISTS,
                    format!("a scene named “{name}” already exists"),
                ));
            }
            let new = Scene::new(name, main_canvas(&p)?.id);
            let id = new.id;
            exec(
                engine,
                Command::AddScene {
                    scene: new,
                    index: usize::MAX,
                },
            )?;
            Ok(json!({ "sceneUuid": id.to_string() }))
        }
        "RemoveScene" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = scene(&p, &a)?;
            exec(engine, Command::RemoveScene { scene: s.id })
        }
        "SetSceneName" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = scene(&p, &a)?;
            let name = a.str("newSceneName")?;
            if p.scenes.iter().any(|o| o.name == name && o.id != s.id) {
                return Err(RequestError::new(
                    st::RESOURCE_ALREADY_EXISTS,
                    format!("a scene named “{name}” already exists"),
                ));
            }
            exec(
                engine,
                Command::RenameScene {
                    scene: s.id,
                    name: name.to_owned(),
                },
            )
        }

        // ------------------------------------------------------------ inputs
        "GetInputList" => {
            let p = engine(studio)?.snapshot();
            let kind = a.opt_str("inputKind")?.map(views::estream_kind);
            let inputs: Vec<Value> = p
                .sources
                .iter()
                .filter(|s| kind.as_ref().is_none_or(|k| *k == s.kind))
                .map(views::input_json)
                .collect();
            Ok(json!({ "inputs": inputs }))
        }
        "GetInputKindList" => {
            let unversioned = a.opt_bool("unversioned")?.unwrap_or(false);
            let engine = engine(studio)?;
            let kinds: Vec<&str> = engine
                .registry()
                .kinds()
                .map(|k| {
                    let obs = views::obs_kind(k);
                    if unversioned {
                        views::unversioned(obs)
                    } else {
                        obs
                    }
                })
                .collect();
            Ok(json!({ "inputKinds": kinds }))
        }
        "GetSpecialInputs" => {
            let p = engine(studio)?.snapshot();
            let names = |kind: &str| -> Vec<String> {
                p.sources
                    .iter()
                    .filter(|s| s.kind == kind)
                    .map(|s| s.name.clone())
                    .collect()
            };
            let desktop = names("audio_output");
            let mic = names("audio_input");
            Ok(json!({
                "desktop1": desktop.first(),
                "desktop2": desktop.get(1),
                "mic1": mic.first(),
                "mic2": mic.get(1),
                "mic3": mic.get(2),
                "mic4": mic.get(3),
            }))
        }
        "CreateInput" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let target = scene(&p, &a)?;
            let name = a.str("inputName")?;
            if p.sources.iter().any(|s| s.name == name) {
                return Err(RequestError::new(
                    st::RESOURCE_ALREADY_EXISTS,
                    format!("an input named “{name}” already exists"),
                ));
            }
            let kind = views::estream_kind(a.str("inputKind")?);
            let Some(source_type) = engine.registry().get(&kind) else {
                return Err(RequestError::new(
                    st::INVALID_INPUT_KIND,
                    format!("eStream has no “{kind}” inputs"),
                ));
            };
            let given = match a.get("inputSettings") {
                Some(v) => v
                    .as_object()
                    .cloned()
                    .ok_or_else(|| Args::wrong_type("inputSettings", "an object"))?,
                None => Map::new(),
            };
            let defaults = source_type.schema(&json!({})).with_defaults(&json!({}));
            let mut settings = defaults.as_object().cloned().unwrap_or_default();
            settings.extend(import_settings(&kind, given));
            let new = SourceDef::new(name, kind, Value::Object(settings));
            let mut new_item = SceneItem::source(new.id);
            new_item.visible = a.opt_bool("sceneItemEnabled")?.unwrap_or(true);
            let (source_id, item_id) = (new.id, new_item.id);
            exec(
                engine,
                Command::Batch {
                    commands: vec![
                        Command::AddSource {
                            source: new,
                            index: usize::MAX,
                        },
                        Command::AddItem {
                            scene: target.id,
                            parent: None,
                            index: usize::MAX,
                            item: new_item,
                        },
                    ],
                },
            )?;
            Ok(json!({
                "inputUuid": source_id.to_string(),
                "sceneItemId": ctx.ids.number(item_id),
            }))
        }
        "RemoveInput" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = input(&p, &a)?;
            exec(engine, Command::RemoveSource { source: s.id })
        }
        "SetInputName" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = input(&p, &a)?;
            let name = a.str("newInputName")?;
            if p.sources.iter().any(|o| o.name == name && o.id != s.id) {
                return Err(RequestError::new(
                    st::RESOURCE_ALREADY_EXISTS,
                    format!("an input named “{name}” already exists"),
                ));
            }
            exec(
                engine,
                Command::RenameSource {
                    source: s.id,
                    name: name.to_owned(),
                },
            )
        }
        "GetInputDefaultSettings" => {
            let engine = engine(studio)?;
            let kind = views::estream_kind(a.str("inputKind")?);
            let Some(t) = engine.registry().get(&kind) else {
                return Err(RequestError::new(
                    st::INVALID_INPUT_KIND,
                    format!("eStream has no “{kind}” inputs"),
                ));
            };
            Ok(json!({
                "defaultInputSettings": t.schema(&json!({})).with_defaults(&json!({})),
            }))
        }
        "GetInputSettings" => {
            let p = engine(studio)?.snapshot();
            let s = input(&p, &a)?;
            Ok(json!({ "inputSettings": s.settings, "inputKind": views::obs_kind(&s.kind) }))
        }
        "SetInputSettings" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = input(&p, &a)?;
            let given = import_settings(&s.kind, a.object("inputSettings")?.clone());
            let overlay = a.opt_bool("overlay")?.unwrap_or(true);
            let settings = if overlay {
                let mut merged = s.settings.as_object().cloned().unwrap_or_default();
                merged.extend(given);
                merged
            } else {
                given
            };
            exec(
                engine,
                Command::SetSourceSettings {
                    source: s.id,
                    settings: Value::Object(settings),
                },
            )
        }
        "GetInputMute" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({ "inputMuted": s.audio.muted }))
        }
        "SetInputMute" | "ToggleInputMute" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let muted = if request_type == "ToggleInputMute" {
                !s.audio.muted
            } else {
                a.bool("inputMuted")?
            };
            update_audio(engine, s, |au| au.muted = muted)?;
            Ok(if request_type == "ToggleInputMute" {
                json!({ "inputMuted": muted })
            } else {
                Value::Null
            })
        }
        "GetInputVolume" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({
                "inputVolumeMul": views::volume_mul(s.audio.volume_db),
                "inputVolumeDb": s.audio.volume_db,
            }))
        }
        "SetInputVolume" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let mul = a.opt_f64("inputVolumeMul", 0.0, 20.0)?;
            let db = a.opt_f64("inputVolumeDb", -100.0, 26.0)?;
            let db = match (mul, db) {
                (Some(m), None) => views::mul_to_db(m),
                (None, Some(d)) => d as f32,
                (None, None) => {
                    return Err(RequestError::new(
                        st::MISSING_REQUEST_FIELD,
                        "pass `inputVolumeMul` or `inputVolumeDb`",
                    ));
                }
                (Some(_), Some(_)) => {
                    return Err(RequestError::new(
                        st::INVALID_REQUEST_FIELD,
                        "pass only one of `inputVolumeMul` and `inputVolumeDb`",
                    ));
                }
            };
            update_audio(engine, s, |au| au.volume_db = db)
        }
        "GetInputAudioBalance" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({ "inputAudioBalance": s.audio.balance }))
        }
        "SetInputAudioBalance" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let balance = a.f64("inputAudioBalance", 0.0, 1.0)? as f32;
            update_audio(engine, s, |au| au.balance = balance)
        }
        "GetInputAudioSyncOffset" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({ "inputAudioSyncOffset": s.audio.sync_offset_ms }))
        }
        "SetInputAudioSyncOffset" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let offset = a.i64("inputAudioSyncOffset", 0, 20_000)? as u32;
            update_audio(engine, s, |au| au.sync_offset_ms = offset)
        }
        "GetInputAudioMonitorType" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({ "monitorType": views::monitor_type(s.audio.monitoring) }))
        }
        "SetInputAudioMonitorType" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let kind = a.str("monitorType")?;
            let monitoring = views::parse_monitor_type(kind).ok_or_else(|| {
                RequestError::new(
                    st::INVALID_REQUEST_FIELD,
                    format!("unknown monitor type {kind}"),
                )
            })?;
            update_audio(engine, s, |au| au.monitoring = monitoring)
        }
        "GetInputAudioTracks" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            Ok(json!({ "inputAudioTracks": views::tracks_json(&s.audio) }))
        }
        "SetInputAudioTracks" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = audio_input(engine, &p, &a)?;
            let mut tracks = s.audio.tracks;
            for (key, value) in a.object("inputAudioTracks")? {
                let n: u8 = key
                    .parse()
                    .ok()
                    .filter(|n| (1..=6).contains(n))
                    .ok_or_else(|| {
                        RequestError::new(
                            st::INVALID_REQUEST_FIELD,
                            format!("track “{key}” doesn't exist (use 1–6)"),
                        )
                    })?;
                let on = value
                    .as_bool()
                    .ok_or_else(|| Args::wrong_type("inputAudioTracks", "booleans"))?;
                if on {
                    tracks |= 1 << (n - 1);
                } else {
                    tracks &= !(1 << (n - 1));
                }
            }
            update_audio(engine, s, |au| au.tracks = tracks)
        }
        "PressInputPropertiesButton" => {
            let p = engine(studio)?.snapshot();
            let id = input(&p, &a)?.id;
            let property = a.str("propertyName")?.to_owned();
            studio
                .press_input_button(id, &property)
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::RESOURCE_ACTION_FAILED, e))
        }

        // ------------------------------------------------------------ transitions
        "GetTransitionKindList" => {
            let kinds: Vec<&str> = views::transition_kinds()
                .iter()
                .map(|k| views::transition_names(k).1)
                .collect();
            Ok(json!({ "transitionKinds": kinds }))
        }
        "GetSceneTransitionList" => {
            let p = engine(studio)?.snapshot();
            let (current, current_kind) = views::transition_names(&p.transition.kind);
            let list: Vec<Value> = views::transition_kinds()
                .iter()
                .map(|k| {
                    let (name, kind) = views::transition_names(k);
                    json!({
                        "transitionName": name,
                        "transitionUuid": Value::Null,
                        "transitionKind": kind,
                        "transitionFixed": matches!(k, TransitionKind::Cut),
                        "transitionConfigurable": !matches!(k, TransitionKind::Cut | TransitionKind::Fade),
                    })
                })
                .collect();
            Ok(json!({
                "currentSceneTransitionName": current,
                "currentSceneTransitionUuid": Value::Null,
                "currentSceneTransitionKind": current_kind,
                "transitions": list,
            }))
        }
        "GetCurrentSceneTransition" => {
            let p = engine(studio)?.snapshot();
            Ok(transition_json(&p.transition))
        }
        "SetCurrentSceneTransition" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let name = a.str("transitionName")?;
            if p.transition.kind.label() == name {
                return Ok(Value::Null);
            }
            let Some(kind) = views::transition_kinds()
                .into_iter()
                .find(|k| k.label() == name)
            else {
                return Err(not_found(format!("no transition named “{name}”")));
            };
            if matches!(
                kind,
                TransitionKind::Stinger { .. } | TransitionKind::Shader { .. }
            ) {
                return Err(RequestError::new(
                    st::INVALID_RESOURCE_STATE,
                    format!("set up the {name} transition in eStream first"),
                ));
            }
            exec(
                engine,
                Command::SetTransition {
                    transition: Transition {
                        kind,
                        duration_ms: p.transition.duration_ms,
                    },
                },
            )
        }
        "SetCurrentSceneTransitionDuration" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let ms = a.i64("transitionDuration", 50, 20_000)? as u32;
            exec(
                engine,
                Command::SetTransition {
                    transition: Transition {
                        kind: p.transition.kind.clone(),
                        duration_ms: ms,
                    },
                },
            )
        }
        "TriggerStudioModeTransition" => {
            if !studio.studio_mode() {
                return Err(RequestError::new(
                    st::STUDIO_MODE_NOT_ACTIVE,
                    "studio mode is off",
                ));
            }
            studio
                .trigger_studio_transition()
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::REQUEST_PROCESSING_FAILED, e))
        }

        // ------------------------------------------------------------ sources and filters
        "GetSourceActive" => {
            let p = engine(studio)?.snapshot();
            let s = source(&p, &a)?;
            let active = p
                .canvases
                .iter()
                .filter_map(|c| c.active_scene.and_then(|id| p.scene(id)))
                .any(|scene| shows(scene, s.id));
            let showing = active
                || (studio.studio_mode()
                    && studio
                        .preview_scene()
                        .and_then(|id| p.scene(id))
                        .is_some_and(|scene| shows(scene, s.id)));
            Ok(json!({ "videoActive": active, "videoShowing": showing }))
        }
        "GetSourceFilterList" => {
            let p = engine(studio)?.snapshot();
            let s = source(&p, &a)?;
            let list: Vec<Value> = views::filters(s)
                .enumerate()
                .map(|(i, f)| views::filter_json(i, f))
                .collect();
            Ok(json!({ "filters": list }))
        }
        "GetSourceFilter" => {
            let p = engine(studio)?.snapshot();
            let s = source(&p, &a)?;
            let name = a.str("filterName")?;
            let (i, f) = views::filters(s)
                .enumerate()
                .find(|(_, f)| f.id == name)
                .ok_or_else(|| not_found(format!("“{}” has no filter “{name}”", s.name)))?;
            let mut out = views::filter_json(i, f);
            if let Some(map) = out.as_object_mut() {
                map.remove("filterName");
            }
            Ok(out)
        }
        "SetSourceFilterEnabled" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let s = source(&p, &a)?;
            let name = a.str("filterName")?;
            let enabled = a.bool("filterEnabled")?;
            if s.filters.iter().any(|f| f.id == name) {
                let mut filters = s.filters.clone();
                for f in filters.iter_mut().filter(|f| f.id == name) {
                    f.enabled = enabled;
                }
                exec(
                    engine,
                    Command::SetSourceFilters {
                        source: s.id,
                        filters,
                    },
                )
            } else if s.audio.filters.iter().any(|f| f.id == name) {
                update_audio(engine, s, |au| {
                    for f in au.filters.iter_mut().filter(|f| f.id == name) {
                        f.enabled = enabled;
                    }
                })
            } else {
                Err(not_found(format!("“{}” has no filter “{name}”", s.name)))
            }
        }

        // ------------------------------------------------------------ scene items
        "GetSceneItemList" | "GetGroupSceneItemList" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, group) = scene_or_group(&p, &a)?;
            if request_type == "GetGroupSceneItemList" && group.is_none() {
                return Err(RequestError::new(
                    st::INVALID_RESOURCE_TYPE,
                    format!("“{}” is a scene, not a group", scene.name),
                ));
            }
            let items: Vec<Value> = container_items(scene, group)
                .iter()
                .enumerate()
                .map(|(i, item)| views::item_json(&p, Some(engine), &mut ctx.ids, item, i))
                .collect();
            Ok(json!({ "sceneItems": items }))
        }
        "GetSceneItemId" => {
            let p = engine(studio)?.snapshot();
            let (scene, group) = scene_or_group(&p, &a)?;
            let name = a.str("sourceName")?;
            let offset = match a.get("searchOffset") {
                Some(_) => a.i64("searchOffset", -1, i64::MAX)?,
                None => 0,
            };
            let matches: Vec<&SceneItem> = container_items(scene, group)
                .iter()
                .filter(|i| match i.source_id() {
                    Some(src) => p.source(src).is_some_and(|s| s.name == name),
                    None => i.name.as_deref() == Some(name),
                })
                .collect();
            let found = if offset < 0 {
                matches.last()
            } else {
                matches.get(offset as usize)
            };
            let found =
                found.ok_or_else(|| not_found(format!("“{}” has no item “{name}”", scene.name)))?;
            Ok(json!({ "sceneItemId": ctx.ids.number(found.id) }))
        }
        "GetSceneItemSource" => {
            let p = engine(studio)?.snapshot();
            let (_, item) = item(&p, &ctx.ids, &a)?;
            let s = item.source_id().and_then(|id| p.source(id));
            Ok(json!({
                "sourceName": s.map_or_else(|| item.name.clone().unwrap_or_default(), |s| s.name.clone()),
                "sourceUuid": s.map_or_else(|| item.id.to_string(), |s| s.id.to_string()),
            }))
        }
        "CreateSceneItem" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let target = scene(&p, &a)?;
            let src = match source(&p, &a) {
                Ok(s) => s,
                Err(e) if e.code == st::RESOURCE_NOT_FOUND => {
                    if find_scene(&p, &a, "sourceName", "sourceUuid").is_ok() {
                        return Err(RequestError::new(
                            st::INVALID_RESOURCE_TYPE,
                            "eStream can't put a scene inside a scene yet",
                        ));
                    }
                    return Err(e);
                }
                Err(e) => return Err(e),
            };
            let mut new_item = SceneItem::source(src.id);
            new_item.visible = a.opt_bool("sceneItemEnabled")?.unwrap_or(true);
            let id = new_item.id;
            exec(
                engine,
                Command::AddItem {
                    scene: target.id,
                    parent: None,
                    index: usize::MAX,
                    item: new_item,
                },
            )?;
            Ok(json!({ "sceneItemId": ctx.ids.number(id) }))
        }
        "RemoveSceneItem" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            exec(
                engine,
                Command::RemoveItem {
                    scene: scene.id,
                    item: item.id,
                },
            )
        }
        "DuplicateSceneItem" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (from, original) = item(&p, &ctx.ids, &a)?;
            let to = match (
                a.opt_str("destinationSceneName")?,
                a.opt_str("destinationSceneUuid")?,
            ) {
                (None, None) => from,
                _ => find_scene(&p, &a, "destinationSceneName", "destinationSceneUuid")?,
            };
            let copy = fresh_copy(original);
            let id = copy.id;
            exec(
                engine,
                Command::AddItem {
                    scene: to.id,
                    parent: None,
                    index: usize::MAX,
                    item: copy,
                },
            )?;
            Ok(json!({ "sceneItemId": ctx.ids.number(id) }))
        }
        "GetSceneItemTransform" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (_, item) = item(&p, &ctx.ids, &a)?;
            Ok(json!({ "sceneItemTransform": views::transform_json(Some(engine), item) }))
        }
        "SetSceneItemTransform" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let t = Args(
                a.get("sceneItemTransform")
                    .ok_or_else(|| Args::missing("sceneItemTransform"))?,
            );
            if !t.0.is_object() {
                return Err(Args::wrong_type("sceneItemTransform", "an object"));
            }
            let big = 1e7;
            let mut transform = item.transform;
            if let Some(x) = t.opt_f64("positionX", -big, big)? {
                transform.position.x = x as f32;
            }
            if let Some(y) = t.opt_f64("positionY", -big, big)? {
                transform.position.y = y as f32;
            }
            if let Some(r) = t.opt_f64("rotation", -360.0, 360.0)? {
                transform.rotation = r as f32;
            }
            if let Some(x) = t.opt_f64("scaleX", -big, big)? {
                transform.scale.x = x as f32;
            }
            if let Some(y) = t.opt_f64("scaleY", -big, big)? {
                transform.scale.y = y as f32;
            }
            if let Some(kind) = t.opt_str("boundsType")? {
                transform.bounds_mode = views::parse_bounds_type(kind).ok_or_else(|| {
                    RequestError::new(
                        st::INVALID_REQUEST_FIELD,
                        format!("unknown bounds type {kind}"),
                    )
                })?;
            }
            if let Some(w) = t.opt_f64("boundsWidth", 0.0, big)? {
                transform.bounds.x = w as f32;
            }
            if let Some(h) = t.opt_f64("boundsHeight", 0.0, big)? {
                transform.bounds.y = h as f32;
            }
            if let Some(bits) = t.opt_f64("boundsAlignment", 0.0, 15.0)? {
                transform.bounds_alignment = views::alignment_from_bits(bits as u32);
            }
            let mut crop: Crop = item.crop;
            for (key, side) in [
                ("cropLeft", &mut crop.left),
                ("cropRight", &mut crop.right),
                ("cropTop", &mut crop.top),
                ("cropBottom", &mut crop.bottom),
            ] {
                if let Some(v) = t.opt_f64(key, 0.0, big)? {
                    *side = v as u32;
                }
            }
            update_item(
                engine,
                scene,
                item,
                ItemPatch {
                    transform: Some(transform),
                    crop: Some(crop),
                    ..Default::default()
                },
            )
        }
        "GetSceneItemEnabled" => {
            let p = engine(studio)?.snapshot();
            let (_, item) = item(&p, &ctx.ids, &a)?;
            Ok(json!({ "sceneItemEnabled": item.visible }))
        }
        "SetSceneItemEnabled" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let visible = a.bool("sceneItemEnabled")?;
            update_item(
                engine,
                scene,
                item,
                ItemPatch {
                    visible: Some(visible),
                    ..Default::default()
                },
            )
        }
        "GetSceneItemLocked" => {
            let p = engine(studio)?.snapshot();
            let (_, item) = item(&p, &ctx.ids, &a)?;
            Ok(json!({ "sceneItemLocked": item.locked }))
        }
        "SetSceneItemLocked" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let locked = a.bool("sceneItemLocked")?;
            update_item(
                engine,
                scene,
                item,
                ItemPatch {
                    locked: Some(locked),
                    ..Default::default()
                },
            )
        }
        "GetSceneItemIndex" => {
            let p = engine(studio)?.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let index = views::locate(scene, item.id).map_or(0, |pl| pl.index);
            Ok(json!({ "sceneItemIndex": index }))
        }
        "SetSceneItemIndex" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let index = a.i64("sceneItemIndex", 0, i64::MAX)? as usize;
            let parent = views::locate(scene, item.id).and_then(|pl| pl.group.map(|g| g.id));
            exec(
                engine,
                Command::MoveItem {
                    scene: scene.id,
                    item: item.id,
                    parent,
                    index,
                },
            )
        }
        "GetSceneItemBlendMode" => {
            let p = engine(studio)?.snapshot();
            let (_, item) = item(&p, &ctx.ids, &a)?;
            Ok(json!({ "sceneItemBlendMode": views::blend_mode(item) }))
        }
        "SetSceneItemBlendMode" => {
            let engine = engine(studio)?;
            let p = engine.snapshot();
            let (scene, item) = item(&p, &ctx.ids, &a)?;
            let mode = match a.str("sceneItemBlendMode")? {
                "OBS_BLEND_NORMAL" => BlendMode::Normal,
                "OBS_BLEND_ADDITIVE" => BlendMode::Additive,
                "OBS_BLEND_MULTIPLY" => BlendMode::Multiply,
                "OBS_BLEND_SCREEN" => BlendMode::Screen,
                other => {
                    return Err(RequestError::new(
                        st::INVALID_REQUEST_FIELD,
                        format!("eStream doesn't support blend mode {other}"),
                    ));
                }
            };
            update_item(
                engine,
                scene,
                item,
                ItemPatch {
                    blend: Some(mode),
                    ..Default::default()
                },
            )
        }

        // ------------------------------------------------------------ outputs
        "GetVirtualCamStatus" => Ok(output_json(studio, Output::VirtualCam)),
        "StartVirtualCam" => set_output(studio, Output::VirtualCam, true).map(|_| Value::Null),
        "StopVirtualCam" => set_output(studio, Output::VirtualCam, false).map(|_| Value::Null),
        "GetStreamStatus" => Ok(output_json(studio, Output::Stream)),
        "GetRecordStatus" => Ok(output_json(studio, Output::Record)),
        "GetReplayBufferStatus" => Ok(output_json(studio, Output::ReplayBuffer)),
        "ToggleStream" | "ToggleRecord" | "ToggleReplayBuffer" | "ToggleVirtualCam" => {
            let output = match request_type {
                "ToggleStream" => Output::Stream,
                "ToggleRecord" => Output::Record,
                "ToggleVirtualCam" => Output::VirtualCam,
                _ => Output::ReplayBuffer,
            };
            let active = !studio.output_status(output).active;
            set_output(studio, output, active)?;
            Ok(json!({ "outputActive": active }))
        }
        "StartStream" => set_output(studio, Output::Stream, true).map(|_| Value::Null),
        "StopStream" => set_output(studio, Output::Stream, false).map(|_| Value::Null),
        "StartRecord" => set_output(studio, Output::Record, true).map(|_| Value::Null),
        "StopRecord" => {
            let path = set_output(studio, Output::Record, false)?;
            Ok(json!({ "outputPath": path.map(|p| p.to_string_lossy().into_owned()) }))
        }
        "PauseRecord" | "ResumeRecord" | "ToggleRecordPause" => Err(RequestError::new(
            st::OUTPUT_DISABLED,
            "eStream can't pause recordings yet",
        )),
        "StartReplayBuffer" => set_output(studio, Output::ReplayBuffer, true).map(|_| Value::Null),
        "StopReplayBuffer" => set_output(studio, Output::ReplayBuffer, false).map(|_| Value::Null),
        "SaveReplayBuffer" => {
            if !studio.output_status(Output::ReplayBuffer).active {
                return Err(RequestError::new(
                    st::OUTPUT_NOT_RUNNING,
                    "the replay buffer isn't running",
                ));
            }
            studio
                .save_replay()
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::REQUEST_PROCESSING_FAILED, e))
        }
        "GetLastReplayBufferReplay" => {
            let path = studio.last_replay().ok_or_else(|| {
                RequestError::new(st::INVALID_RESOURCE_STATE, "no replay has been saved yet")
            })?;
            Ok(json!({ "savedReplayPath": path.to_string_lossy() }))
        }

        // ------------------------------------------------------------ media inputs
        "GetMediaInputStatus" => {
            let p = engine(studio)?.snapshot();
            let s = input(&p, &a)?;
            let status = studio.media_status(s.id).ok_or_else(|| {
                RequestError::new(
                    st::INVALID_RESOURCE_STATE,
                    format!("“{}” isn't a media input", s.name),
                )
            })?;
            let state = match status.state {
                MediaState::None => "OBS_MEDIA_STATE_NONE",
                MediaState::Playing => "OBS_MEDIA_STATE_PLAYING",
                MediaState::Paused => "OBS_MEDIA_STATE_PAUSED",
                MediaState::Ended => "OBS_MEDIA_STATE_ENDED",
            };
            Ok(json!({
                "mediaState": state,
                "mediaDuration": status.duration.map(|d| d.as_millis() as u64),
                "mediaCursor": status.position.map(|d| d.as_millis() as u64),
            }))
        }
        "SetMediaInputCursor" | "OffsetMediaInputCursor" => {
            let p = engine(studio)?.snapshot();
            let s = input(&p, &a)?;
            let status = studio.media_status(s.id).ok_or_else(|| {
                RequestError::new(
                    st::INVALID_RESOURCE_STATE,
                    format!("“{}” isn't a media input", s.name),
                )
            })?;
            let ms = if request_type == "SetMediaInputCursor" {
                a.f64("mediaCursor", 0.0, f64::MAX)?
            } else {
                let offset = a.f64("mediaCursorOffset", f64::MIN, f64::MAX)?;
                (status.position.unwrap_or_default().as_millis() as f64 + offset).max(0.0)
            };
            studio
                .media_seek(s.id, Duration::from_millis(ms as u64))
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::RESOURCE_ACTION_FAILED, e))
        }
        "TriggerMediaInputAction" => {
            let p = engine(studio)?.snapshot();
            let s = input(&p, &a)?;
            if studio.media_status(s.id).is_none() {
                return Err(RequestError::new(
                    st::INVALID_RESOURCE_STATE,
                    format!("“{}” isn't a media input", s.name),
                ));
            }
            let action = match a.str("mediaAction")? {
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NONE" => return Ok(Value::Null),
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY" => MediaAction::Play,
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PAUSE" => MediaAction::Pause,
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" => MediaAction::Stop,
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" => MediaAction::Restart,
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NEXT" => MediaAction::Next,
                "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PREVIOUS" => MediaAction::Previous,
                other => {
                    return Err(RequestError::new(
                        st::INVALID_REQUEST_FIELD,
                        format!("unknown media action {other}"),
                    ));
                }
            };
            studio
                .media_action(s.id, action)
                .map(|_| Value::Null)
                .map_err(|e| RequestError::new(st::RESOURCE_ACTION_FAILED, e))
        }

        // ------------------------------------------------------------ ui
        "GetStudioModeEnabled" => Ok(json!({ "studioModeEnabled": studio.studio_mode() })),
        "SetStudioModeEnabled" => {
            let enabled = a.bool("studioModeEnabled")?;
            if enabled != studio.studio_mode() {
                studio.set_studio_mode(enabled);
            }
            Ok(Value::Null)
        }

        "" => Err(RequestError::new(
            st::MISSING_REQUEST_TYPE,
            "missing `requestType`",
        )),
        other => Err(RequestError::new(
            st::UNKNOWN_REQUEST_TYPE,
            format!("eStream doesn't support {other} yet"),
        )),
    }
}
