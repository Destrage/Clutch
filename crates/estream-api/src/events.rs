//! Turns changes into obs-websocket events. Project changes come from diffing snapshots, so
//! edits from the UI, undo and remote clients all produce the same events. App state that
//! isn't in the project (studio mode, outputs) is polled.

use crate::protocol::intent;
use crate::studio::{Output, OutputStatus, Studio};
use crate::views::{self, ItemIds};
use estream_core::{ItemId, SceneId, SourceId};
use estream_engine::Engine;
use estream_project::{FilterDef, Project, Scene, SceneItem, SourceDef};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// One event to broadcast.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiEvent {
    pub kind: &'static str,
    pub intent: u32,
    pub data: Option<Value>,
}

impl ApiEvent {
    fn new(kind: &'static str, intent: u32, data: Value) -> Self {
        Self {
            kind,
            intent,
            data: Some(data),
        }
    }
}

/// Where an item sits and what it looks like, for diffing.
struct ItemView<'a> {
    item: &'a SceneItem,
    scene: &'a Scene,
    container: String,
    container_uuid: String,
    index: usize,
}

fn items_of(p: &Project) -> HashMap<ItemId, ItemView<'_>> {
    fn add<'a>(
        out: &mut HashMap<ItemId, ItemView<'a>>,
        scene: &'a Scene,
        items: &'a [SceneItem],
        container: &str,
        container_uuid: &str,
    ) {
        for (index, item) in items.iter().enumerate() {
            out.insert(
                item.id,
                ItemView {
                    item,
                    scene,
                    container: container.to_owned(),
                    container_uuid: container_uuid.to_owned(),
                    index,
                },
            );
            if let estream_project::ItemContent::Group { items } = &item.content {
                let name = item.name.clone().unwrap_or_else(|| "Group".into());
                add(out, scene, items, &name, &item.id.to_string());
            }
        }
    }
    let mut out = HashMap::new();
    for scene in &p.scenes {
        add(
            &mut out,
            scene,
            &scene.items,
            &scene.name,
            &scene.id.to_string(),
        );
    }
    out
}

fn source_name(p: &Project, item: &SceneItem) -> (String, String) {
    match item.source_id().and_then(|s| p.source(s)) {
        Some(s) => (s.name.clone(), s.id.to_string()),
        None => (
            item.name.clone().unwrap_or_else(|| "Group".into()),
            item.id.to_string(),
        ),
    }
}

fn filter_map(s: &SourceDef) -> Vec<(&str, &FilterDef)> {
    views::filters(s).map(|f| (f.id.as_str(), f)).collect()
}

/// Diffs project snapshots into events.
#[derive(Default)]
pub struct Translator {
    last: Option<Arc<Project>>,
}

impl Translator {
    /// Events for the change from the previous snapshot to `new` (none for the first one).
    /// `transforms`: also report item transform changes (high-volume, only if wanted).
    pub fn diff(
        &mut self,
        new: &Arc<Project>,
        engine: &Engine,
        ids: &mut ItemIds,
        transforms: bool,
    ) -> Vec<ApiEvent> {
        let Some(old) = self.last.replace(new.clone()) else {
            return Vec::new();
        };
        if Arc::ptr_eq(&old, new) {
            return Vec::new();
        }
        let mut out = Vec::new();
        diff_scenes(&old, new, &mut out);
        diff_sources(&old, new, &mut out);
        diff_items(&old, new, engine, ids, transforms, &mut out);
        diff_transition(&old, new, &mut out);
        out
    }

    /// Forgets the previous snapshot (a different collection was opened).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

fn diff_scenes(old: &Project, new: &Project, out: &mut Vec<ApiEvent>) {
    let old_by: HashMap<SceneId, &Scene> = old.scenes.iter().map(|s| (s.id, s)).collect();
    let new_by: HashMap<SceneId, &Scene> = new.scenes.iter().map(|s| (s.id, s)).collect();
    for s in &old.scenes {
        if !new_by.contains_key(&s.id) {
            out.push(ApiEvent::new(
                "SceneRemoved",
                intent::SCENES,
                json!({ "sceneName": s.name, "sceneUuid": s.id.to_string(), "isGroup": false }),
            ));
        }
    }
    for s in &new.scenes {
        match old_by.get(&s.id) {
            None => out.push(ApiEvent::new(
                "SceneCreated",
                intent::SCENES,
                json!({ "sceneName": s.name, "sceneUuid": s.id.to_string(), "isGroup": false }),
            )),
            Some(o) if o.name != s.name => out.push(ApiEvent::new(
                "SceneNameChanged",
                intent::SCENES,
                json!({ "sceneUuid": s.id.to_string(), "oldSceneName": o.name, "sceneName": s.name }),
            )),
            Some(_) => {}
        }
    }
    let order = |p: &Project| -> Vec<(SceneId, String)> {
        views::scenes_in_order(p)
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect()
    };
    if order(old) != order(new) {
        out.push(ApiEvent::new(
            "SceneListChanged",
            intent::SCENES,
            json!({ "scenes": views::scene_list(new) }),
        ));
    }
    let program = |p: &Project| views::main_program(p).map(|s| s.id);
    if program(old) != program(new)
        && let Some(s) = views::main_program(new)
    {
        out.push(ApiEvent::new(
            "CurrentProgramSceneChanged",
            intent::SCENES,
            json!({ "sceneName": s.name, "sceneUuid": s.id.to_string() }),
        ));
    }
}

fn diff_sources(old: &Project, new: &Project, out: &mut Vec<ApiEvent>) {
    let old_by: HashMap<SourceId, &SourceDef> = old.sources.iter().map(|s| (s.id, s)).collect();
    let new_ids: std::collections::HashSet<SourceId> = new.sources.iter().map(|s| s.id).collect();
    for s in &old.sources {
        if !new_ids.contains(&s.id) {
            out.push(ApiEvent::new(
                "InputRemoved",
                intent::INPUTS,
                json!({ "inputName": s.name, "inputUuid": s.id.to_string() }),
            ));
        }
    }
    for s in &new.sources {
        let id = || json!({ "inputName": s.name, "inputUuid": s.id.to_string() });
        let with = |extra: Value| {
            let mut v = id();
            if let (Some(map), Some(more)) = (v.as_object_mut(), extra.as_object()) {
                map.extend(more.clone());
            }
            v
        };
        let Some(o) = old_by.get(&s.id) else {
            let mut data = views::input_json(s);
            data["inputSettings"] = s.settings.clone();
            out.push(ApiEvent::new("InputCreated", intent::INPUTS, data));
            continue;
        };
        if o.name != s.name {
            out.push(ApiEvent::new(
                "InputNameChanged",
                intent::INPUTS,
                json!({ "inputUuid": s.id.to_string(), "oldInputName": o.name, "inputName": s.name }),
            ));
        }
        if o.settings != s.settings {
            out.push(ApiEvent::new(
                "InputSettingsChanged",
                intent::INPUTS,
                with(json!({ "inputSettings": s.settings })),
            ));
        }
        let (a, b) = (&o.audio, &s.audio);
        if a.muted != b.muted {
            out.push(ApiEvent::new(
                "InputMuteStateChanged",
                intent::INPUTS,
                with(json!({ "inputMuted": b.muted })),
            ));
        }
        if a.volume_db != b.volume_db {
            out.push(ApiEvent::new(
                "InputVolumeChanged",
                intent::INPUTS,
                with(json!({
                    "inputVolumeMul": views::volume_mul(b.volume_db),
                    "inputVolumeDb": b.volume_db,
                })),
            ));
        }
        if a.balance != b.balance {
            out.push(ApiEvent::new(
                "InputAudioBalanceChanged",
                intent::INPUTS,
                with(json!({ "inputAudioBalance": b.balance })),
            ));
        }
        if a.sync_offset_ms != b.sync_offset_ms {
            out.push(ApiEvent::new(
                "InputAudioSyncOffsetChanged",
                intent::INPUTS,
                with(json!({ "inputAudioSyncOffset": b.sync_offset_ms })),
            ));
        }
        if a.tracks != b.tracks {
            out.push(ApiEvent::new(
                "InputAudioTracksChanged",
                intent::INPUTS,
                with(json!({ "inputAudioTracks": views::tracks_json(b) })),
            ));
        }
        if a.monitoring != b.monitoring {
            out.push(ApiEvent::new(
                "InputAudioMonitorTypeChanged",
                intent::INPUTS,
                with(json!({ "monitorType": views::monitor_type(b.monitoring) })),
            ));
        }
        // Filters (video, then audio, as listed by GetSourceFilterList).
        let before = filter_map(o);
        let after = filter_map(s);
        for (name, _) in &before {
            if !after.iter().any(|(n, _)| n == name) {
                out.push(ApiEvent::new(
                    "SourceFilterRemoved",
                    intent::FILTERS,
                    json!({ "sourceName": s.name, "filterName": name }),
                ));
            }
        }
        for (index, (name, f)) in after.iter().enumerate() {
            match before.iter().find(|(n, _)| n == name) {
                None => out.push(ApiEvent::new(
                    "SourceFilterCreated",
                    intent::FILTERS,
                    json!({
                        "sourceName": s.name,
                        "filterName": name,
                        "filterKind": f.kind,
                        "filterIndex": index,
                        "filterSettings": f.settings,
                        "defaultFilterSettings": {},
                    }),
                )),
                Some((_, old_f)) => {
                    if old_f.enabled != f.enabled {
                        out.push(ApiEvent::new(
                            "SourceFilterEnableStateChanged",
                            intent::FILTERS,
                            json!({ "sourceName": s.name, "filterName": name, "filterEnabled": f.enabled }),
                        ));
                    }
                    if old_f.settings != f.settings {
                        out.push(ApiEvent::new(
                            "SourceFilterSettingsChanged",
                            intent::FILTERS,
                            json!({ "sourceName": s.name, "filterName": name, "filterSettings": f.settings }),
                        ));
                    }
                }
            }
        }
    }
}

fn diff_items(
    old: &Project,
    new: &Project,
    engine: &Engine,
    ids: &mut ItemIds,
    transforms: bool,
    out: &mut Vec<ApiEvent>,
) {
    let before = items_of(old);
    let after = items_of(new);
    let new_scenes: std::collections::HashSet<SceneId> = new.scenes.iter().map(|s| s.id).collect();
    // Containers whose order changed (reported once each).
    let mut reindexed: Vec<(SceneId, String, String, Option<ItemId>)> = Vec::new();
    for (id, v) in &before {
        if after.contains_key(id) || !new_scenes.contains(&v.scene.id) {
            continue; // kept, or its whole scene went away (SceneRemoved covers it)
        }
        let (name, uuid) = source_name(old, v.item);
        out.push(ApiEvent::new(
            "SceneItemRemoved",
            intent::SCENE_ITEMS,
            json!({
                "sceneName": v.container,
                "sceneUuid": v.container_uuid,
                "sourceName": name,
                "sourceUuid": uuid,
                "sceneItemId": ids.number(*id),
            }),
        ));
    }
    let mut created: Vec<&ItemView<'_>> = after
        .iter()
        .filter(|(id, _)| !before.contains_key(id))
        .map(|(_, v)| v)
        .collect();
    created.sort_by_key(|v| (v.container_uuid.clone(), v.index));
    for v in created {
        let (name, uuid) = source_name(new, v.item);
        out.push(ApiEvent::new(
            "SceneItemCreated",
            intent::SCENE_ITEMS,
            json!({
                "sceneName": v.container,
                "sceneUuid": v.container_uuid,
                "sourceName": name,
                "sourceUuid": uuid,
                "sceneItemId": ids.number(v.item.id),
                "sceneItemIndex": v.index,
            }),
        ));
    }
    for (id, v) in &after {
        let Some(o) = before.get(id) else { continue };
        let base = || {
            json!({
                "sceneName": v.container,
                "sceneUuid": v.container_uuid,
            })
        };
        let mut push =
            |kind: &'static str, intent_bit: u32, key: &str, value: Value, ids: &mut ItemIds| {
                let mut data = base();
                data["sceneItemId"] = ids.number(*id).into();
                data[key] = value;
                out.push(ApiEvent::new(kind, intent_bit, data));
            };
        if o.item.visible != v.item.visible {
            push(
                "SceneItemEnableStateChanged",
                intent::SCENE_ITEMS,
                "sceneItemEnabled",
                v.item.visible.into(),
                ids,
            );
        }
        if o.item.locked != v.item.locked {
            push(
                "SceneItemLockStateChanged",
                intent::SCENE_ITEMS,
                "sceneItemLocked",
                v.item.locked.into(),
                ids,
            );
        }
        if transforms && (o.item.transform != v.item.transform || o.item.crop != v.item.crop) {
            push(
                "SceneItemTransformChanged",
                intent::SCENE_ITEM_TRANSFORM_CHANGED,
                "sceneItemTransform",
                views::transform_json(Some(engine), v.item),
                ids,
            );
        }
        if (o.index != v.index || o.container_uuid != v.container_uuid)
            && !reindexed
                .iter()
                .any(|(_, _, uuid, _)| *uuid == v.container_uuid)
        {
            let group = views::locate(v.scene, *id).and_then(|pl| pl.group.map(|g| g.id));
            reindexed.push((
                v.scene.id,
                v.container.clone(),
                v.container_uuid.clone(),
                group,
            ));
        }
    }
    for (scene_id, container, container_uuid, group) in reindexed {
        let Some(scene) = new.scene(scene_id) else {
            continue;
        };
        let items = match group.and_then(|g| scene.find_item(g)).map(|g| &g.content) {
            Some(estream_project::ItemContent::Group { items }) => items,
            _ => &scene.items,
        };
        let list: Vec<Value> = items
            .iter()
            .enumerate()
            .map(|(i, item)| json!({ "sceneItemId": ids.number(item.id), "sceneItemIndex": i }))
            .collect();
        out.push(ApiEvent::new(
            "SceneItemListReindexed",
            intent::SCENE_ITEMS,
            json!({ "sceneName": container, "sceneUuid": container_uuid, "sceneItems": list }),
        ));
    }
}

fn diff_transition(old: &Project, new: &Project, out: &mut Vec<ApiEvent>) {
    let (a, b) = (&old.transition, &new.transition);
    if a.kind.label() != b.kind.label() {
        out.push(ApiEvent::new(
            "CurrentSceneTransitionChanged",
            intent::TRANSITIONS,
            json!({ "transitionName": b.kind.label(), "transitionUuid": Value::Null }),
        ));
    }
    if a.duration_ms != b.duration_ms {
        out.push(ApiEvent::new(
            "CurrentSceneTransitionDurationChanged",
            intent::TRANSITIONS,
            json!({ "transitionDuration": b.duration_ms }),
        ));
    }
}

/// App state seen at the previous poll.
#[derive(Default)]
pub struct StatePoller {
    seen: Option<Seen>,
}

#[derive(Clone, PartialEq)]
struct Seen {
    studio: bool,
    preview: Option<SceneId>,
    outputs: [OutputStatus; 4],
    last_replay: Option<PathBuf>,
    transitioning: bool,
    last_recording: Option<PathBuf>,
}

const OUTPUTS: [(Output, &str); 4] = [
    (Output::Stream, "StreamStateChanged"),
    (Output::Record, "RecordStateChanged"),
    (Output::ReplayBuffer, "ReplayBufferStateChanged"),
    (Output::VirtualCam, "VirtualcamStateChanged"),
];

impl StatePoller {
    pub fn poll(&mut self, studio: &dyn Studio) -> Vec<ApiEvent> {
        let engine = studio.engine();
        let snapshot = engine.map(|e| e.snapshot());
        let main = snapshot
            .as_ref()
            .and_then(|p| p.canvases.first().map(|c| c.id));
        let now = Seen {
            studio: studio.studio_mode(),
            preview: studio.preview_scene(),
            outputs: OUTPUTS.map(|(o, _)| studio.output_status(o)),
            last_replay: studio.last_replay(),
            last_recording: studio.last_recording(),
            transitioning: engine.zip(main).is_some_and(|(e, c)| e.is_transitioning(c)),
        };
        let Some(before) = self.seen.replace(now.clone()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if before.studio != now.studio {
            out.push(ApiEvent::new(
                "StudioModeStateChanged",
                intent::UI,
                json!({ "studioModeEnabled": now.studio }),
            ));
        }
        if now.studio
            && (before.preview != now.preview || !before.studio)
            && let Some(s) = now
                .preview
                .and_then(|id| snapshot.as_ref()?.scene(id).cloned())
        {
            out.push(ApiEvent::new(
                "CurrentPreviewSceneChanged",
                intent::SCENES,
                json!({ "sceneName": s.name, "sceneUuid": s.id.to_string() }),
            ));
        }
        for (i, (output, kind)) in OUTPUTS.iter().enumerate() {
            let (a, b) = (&before.outputs[i], &now.outputs[i]);
            let state = if a.active != b.active {
                Some(if b.active { "STARTED" } else { "STOPPED" })
            } else if a.reconnecting != b.reconnecting && b.active {
                Some(if b.reconnecting {
                    "RECONNECTING"
                } else {
                    "RECONNECTED"
                })
            } else {
                None
            };
            if let Some(state) = state {
                let mut data = json!({
                    "outputActive": b.active,
                    "outputState": format!("OBS_WEBSOCKET_OUTPUT_{state}"),
                });
                if *output == Output::Record {
                    data["outputPath"] = match (b.active, &now.last_recording) {
                        (false, Some(path)) => path.to_string_lossy().into(),
                        _ => Value::Null,
                    };
                }
                out.push(ApiEvent::new(kind, intent::OUTPUTS, data));
            }
        }
        if now.last_replay != before.last_replay
            && let Some(path) = &now.last_replay
        {
            out.push(ApiEvent::new(
                "ReplayBufferSaved",
                intent::OUTPUTS,
                json!({ "savedReplayPath": path.to_string_lossy() }),
            ));
        }
        if now.transitioning != before.transitioning {
            let name = snapshot
                .as_ref()
                .map(|p| p.transition.kind.label())
                .unwrap_or("Fade");
            out.push(ApiEvent::new(
                if now.transitioning {
                    "SceneTransitionStarted"
                } else {
                    "SceneTransitionEnded"
                },
                intent::TRANSITIONS,
                json!({ "transitionName": name, "transitionUuid": Value::Null }),
            ));
        }
        out
    }
}

/// `InputVolumeMeters`: linear `[magnitude, peak, input peak]` per channel.
pub fn volume_meters(engine: &Engine) -> ApiEvent {
    let p = engine.snapshot();
    let meters = engine.audio_meters();
    let inputs: Vec<Value> = meters
        .sources
        .iter()
        .filter_map(|(id, meter)| {
            let s = p.source(*id)?;
            let r = meter.read();
            Some(json!({
                "inputName": s.name,
                "inputUuid": s.id.to_string(),
                "inputLevelsMul": [
                    [r.rms[0], r.peak[0], r.peak[0]],
                    [r.rms[1], r.peak[1], r.peak[1]],
                ],
            }))
        })
        .collect();
    ApiEvent::new(
        "InputVolumeMeters",
        intent::INPUT_VOLUME_METERS,
        json!({ "inputs": inputs }),
    )
}
