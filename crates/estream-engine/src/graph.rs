//! Immutable render graph: the flattened, render-ready view of a project that
//! the engine thread hands to the video thread after every change.

use estream_audio::{AudioGraph, AudioNode};
use estream_core::{
    BlendMode, CanvasId, Crop, ItemTransform, Rational, Rgba8, SceneId, SourceId, Timestamp,
};
use estream_project::{ItemContent, Project, SceneItem, SourceDef, Transition, TransitionKind};
use glam::Affine2;
use std::collections::HashMap;

/// Runtime view state that is not part of the document: what the studio Preview shows
/// and which transitions are running.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    /// Studio-mode Preview scene per canvas (absent = studio mode off).
    pub preview: HashMap<CanvasId, SceneId>,
    pub transitions: HashMap<CanvasId, ActiveTransition>,
}

/// A program transition in progress.
#[derive(Clone, Debug)]
pub struct ActiveTransition {
    /// The scene being left (`None` = the background).
    pub from: Option<SceneId>,
    pub transition: Transition,
    pub start: Timestamp,
}

impl ActiveTransition {
    pub fn end(&self) -> Timestamp {
        Timestamp(
            self.start
                .0
                .saturating_add(self.transition.duration_ms as u64 * 1_000_000),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderGraph {
    /// Generation counter: changes whenever the project or the view state changes.
    pub revision: u64,
    pub canvases: Vec<CanvasGraph>,
    /// Every source in the project; all stay alive even when not on screen.
    pub sources: Vec<SourceDef>,
    /// Sources to mix: global ones plus those visible in the active scene.
    pub audio: AudioGraph,
}

#[derive(Clone, Debug)]
pub struct CanvasGraph {
    pub id: CanvasId,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub background: Rgba8,
    /// Visible items of the active (program) scene, bottom first.
    pub items: Vec<FlatItem>,
    /// A running transition into `items`.
    pub transition: Option<TransitionGraph>,
    /// Studio-mode Preview scene items (`None` = studio mode off).
    pub preview: Option<Vec<FlatItem>>,
}

/// What the video thread needs to draw a transition.
#[derive(Clone, Debug)]
pub struct TransitionGraph {
    /// Items of the scene being left.
    pub from: Vec<FlatItem>,
    pub kind: TransitionKind,
    pub start: Timestamp,
    pub duration_ns: u64,
    /// Stinger: the helper media source drawn on top, and when the scene switches.
    pub overlay: Option<(SourceId, u64)>,
}

impl TransitionGraph {
    /// Progress at `t`, or `None` outside the transition.
    pub fn progress(&self, t: Timestamp) -> Option<f32> {
        if t < self.start || self.duration_ns == 0 {
            return None;
        }
        let elapsed = t.0 - self.start.0;
        (elapsed < self.duration_ns).then(|| elapsed as f32 / self.duration_ns as f32)
    }
}

/// A visible source item with its group transforms and opacity resolved.
#[derive(Clone, Debug)]
pub struct FlatItem {
    pub source: SourceId,
    /// Accumulated transform of all enclosing groups.
    pub parent: Affine2,
    pub transform: ItemTransform,
    pub crop: Crop,
    pub opacity: f32,
    pub blend: BlendMode,
}

impl RenderGraph {
    pub fn build(project: &Project, revision: u64) -> Self {
        Self::build_with(project, revision, &ViewState::default(), Timestamp(0))
    }

    /// Builds the graph including view state. Transitions that ended before `now` are left out.
    pub fn build_with(project: &Project, revision: u64, view: &ViewState, now: Timestamp) -> Self {
        let scene_items = |id: Option<SceneId>| {
            let mut items = Vec::new();
            if let Some(scene) = id.and_then(|id| project.scene(id)) {
                flatten(&scene.items, Affine2::IDENTITY, 1.0, &mut items);
            }
            items
        };
        // Helper sources that exist only while a transition runs (stinger videos).
        let mut helpers = Vec::new();
        let canvases = project
            .canvases
            .iter()
            .map(|canvas| {
                let transition = view
                    .transitions
                    .get(&canvas.id)
                    .filter(|t| t.end() > now)
                    .map(|t| {
                        let overlay = stinger_source(canvas.id, t).map(|(def, point)| {
                            let id = def.id;
                            helpers.push((def, canvas.id));
                            (id, point)
                        });
                        TransitionGraph {
                            from: scene_items(t.from),
                            kind: t.transition.kind.clone(),
                            start: t.start,
                            duration_ns: t.transition.duration_ms as u64 * 1_000_000,
                            overlay,
                        }
                    });
                CanvasGraph {
                    id: canvas.id,
                    width: canvas.width,
                    height: canvas.height,
                    fps: canvas.fps,
                    background: canvas.background,
                    items: scene_items(canvas.active_scene),
                    transition,
                    preview: view
                        .preview
                        .get(&canvas.id)
                        .map(|scene| scene_items(Some(*scene))),
                }
            })
            .collect();
        let mut audio = audio_graph(project, view, now);
        // A stinger is heard on its own canvas only.
        audio
            .nodes
            .extend(helpers.iter().map(|(s, canvas)| AudioNode {
                source: s.id,
                kind: s.kind.clone(),
                settings: runtime_settings(s),
                config: s.audio.clone(),
                canvases: vec![*canvas],
            }));
        Self {
            revision,
            canvases,
            sources: project
                .sources
                .iter()
                .chain(helpers.iter().map(|(s, _)| s))
                .map(|s| SourceDef {
                    settings: runtime_settings(s),
                    ..s.clone()
                })
                .collect(),
            audio,
        }
    }
}

/// The program changes a switch of `canvas` to `scene` makes: that canvas (unless it already
/// shows the scene), then, if it follows scene switches, every other following canvas with a
/// scene of the same name that it isn't showing yet.
pub fn scene_switch_targets(
    project: &Project,
    canvas: CanvasId,
    scene: SceneId,
) -> Vec<(CanvasId, SceneId)> {
    let Some(c) = project.canvas(canvas) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    if c.active_scene != Some(scene) {
        targets.push((canvas, scene));
    }
    let Some(name) = project
        .scene(scene)
        .filter(|s| s.canvas == canvas)
        .map(|s| s.name.as_str())
    else {
        return targets;
    };
    if !c.follow_scenes {
        return targets;
    }
    for other in project
        .canvases
        .iter()
        .filter(|o| o.id != canvas && o.follow_scenes)
    {
        if let Some(s) = project.scenes_of(other.id).find(|s| s.name == name)
            && other.active_scene != Some(s.id)
        {
            targets.push((other.id, s.id));
        }
    }
    targets
}

/// The helper media source playing a stinger, and the switch point (ns after the start).
/// Its id depends on the transition's start, so every stinger plays from the beginning.
fn stinger_source(canvas: CanvasId, t: &ActiveTransition) -> Option<(SourceDef, u64)> {
    let TransitionKind::Stinger { file, point_ms } = &t.transition.kind else {
        return None;
    };
    const SALT: u128 = 0x5354_494e_4745_5200_0000_0000_0000_0000; // "STINGER"
    let id = SourceId::from_u128(canvas.0.as_u128() ^ ((t.start.0 as u128) << 32) ^ SALT);
    let mut def = SourceDef::new(
        "Stinger",
        "media",
        serde_json::json!({ "file": file, "loop": false }),
    );
    def.id = id;
    def.audio.global = true;
    Some((def, *point_ms as u64 * 1_000_000))
}

/// Global audio sources (heard on every canvas), then the sources visible in each canvas's
/// active scene (and in the scene being left while a transition runs), each listed once with
/// the canvases whose mix it belongs to.
fn audio_graph(project: &Project, view: &ViewState, now: Timestamp) -> AudioGraph {
    let mut heard: Vec<(SourceId, Vec<CanvasId>)> = project
        .sources
        .iter()
        .filter(|s| s.audio.global)
        .map(|s| (s.id, Vec::new()))
        .collect();
    let globals = heard.len();
    fn visible(items: &[SceneItem], out: &mut Vec<SourceId>) {
        for item in items.iter().filter(|i| i.visible) {
            match &item.content {
                ItemContent::Source { source } => {
                    if !out.contains(source) {
                        out.push(*source);
                    }
                }
                ItemContent::Group { items } => visible(items, out),
            }
        }
    }
    let mut on_canvas = Vec::new();
    for canvas in &project.canvases {
        let leaving = view
            .transitions
            .get(&canvas.id)
            .filter(|t| t.end() > now)
            .and_then(|t| t.from);
        on_canvas.clear();
        for id in [canvas.active_scene, leaving].into_iter().flatten() {
            if let Some(scene) = project.scene(id) {
                visible(&scene.items, &mut on_canvas);
            }
        }
        for source in &on_canvas {
            match heard.iter().position(|(id, _)| id == source) {
                Some(i) if i < globals => {} // already on every canvas
                Some(i) => heard[i].1.push(canvas.id),
                None => heard.push((*source, vec![canvas.id])),
            }
        }
    }
    AudioGraph {
        nodes: heard
            .into_iter()
            .filter_map(|(id, canvases)| project.source(id).map(|s| (s, canvases)))
            .map(|(s, canvases)| AudioNode {
                source: s.id,
                kind: s.kind.clone(),
                settings: runtime_settings(s),
                config: s.audio.clone(),
                canvases,
            })
            .collect(),
        monitor_device: project.audio.monitor_device.clone(),
        canvases: project.canvases.iter().map(|c| c.id).collect(),
    }
}

/// Settings handed to source factories: the stored settings plus `_source_id`, so a source's
/// video and audio sides can find each other (e.g. a shared media player). Never saved.
pub fn runtime_settings(def: &SourceDef) -> serde_json::Value {
    let mut settings = match &def.settings {
        serde_json::Value::Object(_) => def.settings.clone(),
        _ => serde_json::json!({}),
    };
    settings["_source_id"] = serde_json::Value::from(def.id.to_string());
    settings
}

fn flatten(items: &[SceneItem], parent: Affine2, opacity: f32, out: &mut Vec<FlatItem>) {
    for item in items.iter().filter(|i| i.visible && i.opacity > 0.0) {
        let opacity = opacity * item.opacity;
        match &item.content {
            ItemContent::Source { source } => out.push(FlatItem {
                source: *source,
                parent,
                transform: item.transform,
                crop: item.crop,
                opacity,
                blend: item.blend,
            }),
            ItemContent::Group { items } => {
                flatten(items, parent * item.transform.group_matrix(), opacity, out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn groups_compose_transforms_and_opacity() {
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let src = SourceDef::new("S", "color", serde_json::Value::Null);
        let mut child = SceneItem::source(src.id);
        child.transform.position = Vec2::new(10.0, 0.0);
        child.opacity = 0.5;
        let mut hidden = SceneItem::source(src.id);
        hidden.visible = false;
        let mut group = SceneItem::group("G", vec![child, hidden]);
        group.transform.position = Vec2::new(100.0, 50.0);
        group.opacity = 0.5;
        p.scenes[0].items.push(group);
        p.sources.push(src);

        let g = RenderGraph::build(&p, 7);
        assert_eq!(g.revision, 7);
        let items = &g.canvases[0].items;
        assert_eq!(items.len(), 1, "hidden item skipped");
        let item = &items[0];
        assert_eq!(item.opacity, 0.25);
        let origin = item.parent
            * item
                .transform
                .layout(Crop::NONE, estream_core::Size::new(1.0, 1.0))
                .matrix;
        assert_eq!(origin.transform_point2(Vec2::ZERO), Vec2::new(110.0, 50.0));
    }

    /// Main (Gameplay, BRB) and a 1440p copy (Gameplay, BRB) plus a vertical canvas with only
    /// "Vertical gameplay"; every canvas on its first scene.
    fn three_canvases() -> (Project, [CanvasId; 3], [SceneId; 5]) {
        use estream_project::{Canvas, Scene};
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let main = p.canvases[0].id;
        p.scenes[0].name = "Gameplay".into();
        let gameplay = p.scenes[0].id;
        let brb = Scene::new("BRB", main);
        let mut big = Canvas::new("1440p", 2560, 1440, Rational::fps(60));
        let big_gameplay = Scene::new("Gameplay", big.id);
        let big_brb = Scene::new("BRB", big.id);
        big.active_scene = Some(big_gameplay.id);
        let mut vertical = Canvas::new("Vertical", 1080, 1920, Rational::fps(30));
        let v_scene = Scene::new("Vertical gameplay", vertical.id);
        vertical.active_scene = Some(v_scene.id);
        let ids = [main, big.id, vertical.id];
        let scenes = [gameplay, brb.id, big_gameplay.id, big_brb.id, v_scene.id];
        p.canvases.extend([big, vertical]);
        p.scenes.extend([brb, big_gameplay, big_brb, v_scene]);
        (p, ids, scenes)
    }

    #[test]
    fn same_named_scenes_switch_together() {
        let (p, [main, big, _], [_, brb, _, big_brb, _]) = three_canvases();
        assert_eq!(
            scene_switch_targets(&p, main, brb),
            vec![(main, brb), (big, big_brb)],
            "the vertical canvas has no BRB and stays"
        );
        // From the copy, the original follows too.
        assert_eq!(
            scene_switch_targets(&p, big, big_brb),
            vec![(big, big_brb), (main, brb)]
        );
    }

    #[test]
    fn a_canvas_already_there_still_brings_the_others() {
        let (mut p, [main, big, _], [gameplay, _, _, big_brb, _]) = three_canvases();
        p.canvas_mut(big).unwrap().active_scene = Some(big_brb);
        let big_gameplay = p.scenes_of(big).find(|s| s.name == "Gameplay").unwrap().id;
        assert_eq!(
            scene_switch_targets(&p, main, gameplay),
            vec![(big, big_gameplay)]
        );
    }

    #[test]
    fn canvases_that_dont_follow_switch_alone() {
        let (mut p, [main, big, _], [_, brb, _, big_brb, _]) = three_canvases();
        p.canvas_mut(big).unwrap().follow_scenes = false;
        assert_eq!(scene_switch_targets(&p, main, brb), vec![(main, brb)]);
        assert_eq!(scene_switch_targets(&p, big, big_brb), vec![(big, big_brb)]);
    }

    #[test]
    fn audio_follows_every_canvas_and_mixes_shared_sources_once() {
        use estream_project::{Canvas, Scene};
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let cam = SourceDef::new("Cam", "audio_test_tone", serde_json::Value::Null);
        let music = SourceDef::new("Music", "audio_test_tone", serde_json::Value::Null);
        let mut vertical = Canvas::new("Vertical", 1080, 1920, Rational::fps(30));
        let mut v_scene = Scene::new("V", vertical.id);
        v_scene.items = vec![SceneItem::source(cam.id), SceneItem::source(music.id)];
        vertical.active_scene = Some(v_scene.id);
        p.scenes[0].items = vec![SceneItem::source(cam.id)];
        p.canvases.push(vertical);
        p.scenes.push(v_scene);
        p.sources = vec![cam.clone(), music.clone()];

        let mut mic = SourceDef::new("Mic", "audio_test_tone", serde_json::Value::Null);
        mic.audio.global = true;
        p.sources.push(mic.clone());
        let (main, vertical) = (p.canvases[0].id, p.canvases[1].id);

        let g = RenderGraph::build(&p, 1);
        let heard: Vec<(SourceId, Vec<CanvasId>)> = g
            .audio
            .nodes
            .iter()
            .map(|n| (n.source, n.canvases.clone()))
            .collect();
        assert_eq!(
            heard,
            vec![
                (mic.id, vec![]),
                (cam.id, vec![main, vertical]),
                (music.id, vec![vertical]),
            ],
            "global mic everywhere, camera listed once for both, music only on the vertical"
        );
        assert_eq!(g.audio.canvases, vec![main, vertical]);
        assert_eq!(g.canvases.len(), 2);
        assert_eq!(g.canvases[1].items.len(), 2);
    }
}
