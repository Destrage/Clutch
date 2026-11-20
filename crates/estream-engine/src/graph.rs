//! Immutable render graph: the flattened, render-ready view of a project that
//! the engine thread hands to the video thread after every change.

use estream_audio::{AudioGraph, AudioNode};
use estream_core::{
    BlendMode, CanvasId, Crop, ItemTransform, Rational, Rgba8, SceneId, SourceId, Timestamp,
};
use estream_project::{
    ItemAnimation, ItemAnimations, ItemContent, MoveSettings, Project, SceneItem, SourceDef,
    Transition, TransitionKind,
};
use glam::Affine2;
use std::collections::HashMap;

/// Runtime view state that is not part of the document: what the studio Preview shows
/// and which transitions are running.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    /// Studio-mode Preview scene per canvas (absent = studio mode off).
    pub preview: HashMap<CanvasId, SceneId>,
    pub transitions: HashMap<CanvasId, ActiveTransition>,
    /// Show and hide animations started by visibility changes.
    pub item_animations: HashMap<estream_core::ItemId, ItemAnimationRun>,
}

/// A show or hide animation in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemAnimationRun {
    pub showing: bool,
    pub animation: ItemAnimation,
    pub start: Timestamp,
    /// Linear progress at `start` (0..1): above 0 when this run reverses another one midway,
    /// so it carries on from where that one was.
    pub from: f32,
}

impl ItemAnimationRun {
    /// A run from the beginning.
    pub fn new(showing: bool, animation: ItemAnimation, start: Timestamp) -> Self {
        Self {
            showing,
            animation,
            start,
            from: 0.0,
        }
    }

    pub fn end(&self) -> Timestamp {
        let left =
            (1.0 - f64::from(self.from.clamp(0.0, 1.0))) * self.animation.duration_ns() as f64;
        Timestamp(self.start.0.saturating_add(left as u64))
    }

    /// Linear progress at `t`: `from` until the start, 1 from the end on.
    pub fn progress(&self, t: Timestamp) -> f32 {
        let from = f64::from(self.from.clamp(0.0, 1.0));
        let d = self.animation.duration_ns();
        if t <= self.start {
            from as f32
        } else if d == 0 {
            1.0
        } else {
            (from + (t.0 - self.start.0) as f64 / d as f64).min(1.0) as f32
        }
    }

    /// How much of the item is there at `t`: 0 gone, 1 fully shown (eased, so a spring can
    /// go past 1).
    pub fn presence(&self, t: Timestamp) -> f32 {
        let e = self.animation.ease.apply(self.progress(t));
        if self.showing { e } else { 1.0 - e }
    }
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
    /// A gradient background (`None` for a solid one).
    pub background_paint: Option<estream_core::Paint>,
    /// Visible items of the active (program) scene, bottom first.
    pub items: Vec<FlatItem>,
    /// A running transition into `items`.
    pub transition: Option<TransitionGraph>,
    /// Studio-mode Preview scene items (`None` = studio mode off).
    pub preview: Option<Vec<FlatItem>>,
    /// Masked items of every scene on this canvas, hidden ones too, so their masks are baked
    /// before a switch or a show animation needs them. Never drawn from this list.
    pub standby: Vec<FlatItem>,
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
    /// "Move matching items" (`None` = off).
    pub moves: Option<MoveSettings>,
    /// Items that travel to their new place: (index in `from`, index in the program items).
    pub pairs: Vec<(usize, usize)>,
    /// Items only in the scene being left that play their hide animation (index in `from`).
    pub leaving: Vec<(usize, ItemAnimationRun)>,
    /// Items only in the new scene that play their show animation (index in the program items).
    pub arriving: Vec<(usize, ItemAnimationRun)>,
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
    /// The scene item (fills keep their GPU state per item).
    pub item: estream_core::ItemId,
    pub source: SourceId,
    /// Accumulated transform of all enclosing groups.
    pub parent: Affine2,
    pub transform: ItemTransform,
    pub crop: Crop,
    pub opacity: f32,
    pub blend: BlendMode,
    /// What fills the rest of the bounding box (drawn only with Fit bounds).
    pub fill: estream_project::ItemFill,
    /// The item's mask and effects, when they change how it's drawn.
    pub mask: Option<estream_project::ItemMask>,
    pub animations: ItemAnimations,
    /// A running show or hide animation.
    pub run: Option<ItemAnimationRun>,
}

impl RenderGraph {
    pub fn build(project: &Project, revision: u64) -> Self {
        Self::build_with(project, revision, &ViewState::default(), Timestamp(0))
    }

    /// Builds the graph including view state. Transitions that ended before `now` are left out.
    pub fn build_with(project: &Project, revision: u64, view: &ViewState, now: Timestamp) -> Self {
        let runs = &view.item_animations;
        let scene_items = |id: Option<SceneId>| {
            let mut items = Vec::new();
            if let Some(scene) = id.and_then(|id| project.scene(id)) {
                let mut f = Flatten {
                    runs,
                    now,
                    out: &mut items,
                };
                f.items(&scene.items, Affine2::IDENTITY, 1.0);
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
                        let from = scene_items(t.from);
                        let to = scene_items(canvas.active_scene);
                        let (pairs, leaving, arriving) = choreograph(&from, &to, t);
                        TransitionGraph {
                            from,
                            kind: t.transition.kind.clone(),
                            start: t.start,
                            duration_ns: t.transition.duration_ms as u64 * 1_000_000,
                            overlay,
                            moves: t.transition.moves,
                            pairs,
                            leaving,
                            arriving,
                        }
                    });
                let mut standby = Vec::new();
                for scene in project.scenes_of(canvas.id) {
                    masked(&scene.items, Affine2::IDENTITY, &mut standby);
                }
                CanvasGraph {
                    id: canvas.id,
                    width: canvas.width,
                    height: canvas.height,
                    fps: canvas.fps,
                    background: canvas.background,
                    background_paint: canvas.background_paint.clone(),
                    items: scene_items(canvas.active_scene),
                    transition,
                    preview: view
                        .preview
                        .get(&canvas.id)
                        .map(|scene| scene_items(Some(*scene))),
                    standby,
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

/// Flattens a scene into the items to draw: visible ones, and hidden ones still playing their
/// hide animation.
struct Flatten<'a> {
    runs: &'a HashMap<estream_core::ItemId, ItemAnimationRun>,
    now: Timestamp,
    out: &'a mut Vec<FlatItem>,
}

impl Flatten<'_> {
    fn items(&mut self, items: &[SceneItem], parent: Affine2, opacity: f32) {
        for item in items.iter().filter(|i| i.opacity > 0.0) {
            let run = self
                .runs
                .get(&item.id)
                .filter(|r| r.end() > self.now)
                .copied();
            let opacity = opacity * item.opacity;
            match &item.content {
                ItemContent::Source { source }
                    if item.visible || run.is_some_and(|r| !r.showing) =>
                {
                    self.out.push(flat(item, *source, parent, opacity, run))
                }
                ItemContent::Source { .. } => {}
                ItemContent::Group { items } if item.visible => {
                    self.items(items, parent * item.transform.group_matrix(), opacity)
                }
                ItemContent::Group { .. } => {}
            }
        }
    }
}

fn flat(
    item: &SceneItem,
    source: SourceId,
    parent: Affine2,
    opacity: f32,
    run: Option<ItemAnimationRun>,
) -> FlatItem {
    FlatItem {
        item: item.id,
        source,
        parent,
        transform: item.transform,
        crop: item.crop,
        opacity,
        blend: item.blend,
        fill: if item.transform.bounds_mode == estream_core::BoundsMode::Fit {
            item.fill.clone()
        } else {
            estream_project::ItemFill::None
        },
        mask: item.mask.clone().filter(|m| m.draws()),
        animations: item.animations,
        run,
    }
}

/// Every masked source item, visible or not, with its group transforms.
fn masked(items: &[SceneItem], parent: Affine2, out: &mut Vec<FlatItem>) {
    for item in items {
        match &item.content {
            ItemContent::Source { source } => {
                if item.mask.as_ref().is_some_and(|m| m.draws()) {
                    out.push(flat(item, *source, parent, item.opacity, None));
                }
            }
            ItemContent::Group { items } => {
                masked(items, parent * item.transform.group_matrix(), out)
            }
        }
    }
}

type Choreography = (
    Vec<(usize, usize)>,
    Vec<(usize, ItemAnimationRun)>,
    Vec<(usize, ItemAnimationRun)>,
);

/// What each item does during scene transition `t` from `from` to `to`: the pairs that move
/// (when moves are on), and the items only on one side that play their own animation.
///
/// Pairs share a source and are matched from the top of the list down. Hide animations start
/// with the transition; show animations end with it; both last at most as long as it does.
fn choreograph(from: &[FlatItem], to: &[FlatItem], t: &ActiveTransition) -> Choreography {
    let mut pairs = Vec::new();
    if t.transition.moves.is_some() {
        let mut taken = vec![false; from.len()];
        for (j, b) in to.iter().enumerate().rev() {
            if !b.animations.moves {
                continue;
            }
            if let Some(i) = (0..from.len())
                .rev()
                .find(|&i| !taken[i] && from[i].source == b.source && from[i].animations.moves)
            {
                taken[i] = true;
                pairs.push((i, j));
            }
        }
        pairs.reverse(); // drawn bottom first
    }
    let total = t.transition.duration_ms;
    let clamp = |a: &ItemAnimation| ItemAnimation {
        duration_ms: a.duration_ms.min(total),
        ..*a
    };
    let leaving = from
        .iter()
        .enumerate()
        .filter(|(_, a)| !to.iter().any(|b| b.source == a.source))
        .filter_map(|(i, a)| {
            let animation = clamp(a.animations.hide.as_ref()?);
            Some((i, ItemAnimationRun::new(false, animation, t.start)))
        })
        .collect();
    let arriving = to
        .iter()
        .enumerate()
        .filter(|(_, b)| !from.iter().any(|a| a.source == b.source))
        .filter_map(|(j, b)| {
            let animation = clamp(b.animations.show.as_ref()?);
            let delay = u64::from(total - animation.duration_ms) * 1_000_000;
            Some((
                j,
                ItemAnimationRun::new(true, animation, Timestamp(t.start.0.saturating_add(delay))),
            ))
        })
        .collect();
    (pairs, leaving, arriving)
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

    fn animated(kind: estream_project::AnimationKind, ms: u32) -> Option<ItemAnimation> {
        Some(ItemAnimation {
            kind,
            duration_ms: ms,
            ease: estream_project::Ease::Linear,
        })
    }

    /// Scene A (program) and B; `from_items` and `to_items` go into A and B, then the program
    /// switches to B with a 1000 ms Fade (moves on or off).
    fn switched(
        from_items: Vec<SceneItem>,
        to_items: Vec<SceneItem>,
        moves: bool,
    ) -> (RenderGraph, Vec<SourceDef>) {
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let canvas = p.canvases[0].id;
        let a = p.scenes[0].id;
        let mut b = estream_project::Scene::new("B", canvas);
        p.scenes[0].items = from_items;
        b.items = to_items;
        let b_id = b.id;
        p.scenes.push(b);
        p.canvas_mut(canvas).unwrap().active_scene = Some(b_id);
        let mut view = ViewState::default();
        view.transitions.insert(
            canvas,
            ActiveTransition {
                from: Some(a),
                transition: Transition {
                    kind: TransitionKind::Fade,
                    duration_ms: 1000,
                    moves: moves.then(MoveSettings::default),
                },
                start: Timestamp(1_000_000_000),
            },
        );
        (
            RenderGraph::build_with(&p, 1, &view, Timestamp(1_000_000_000)),
            p.sources.clone(),
        )
    }

    #[test]
    fn matching_sources_pair_from_the_top_and_opted_out_items_dont() {
        let [cam, overlay, other] = [1u128, 2, 3].map(SourceId::from_u128);
        let mut grouped = SceneItem::source(cam);
        grouped.transform.position = Vec2::new(5.0, 5.0);
        let from = vec![
            SceneItem::source(overlay),
            SceneItem::group("G", vec![grouped]),
        ];
        // B: two cameras (the upper one, last in the list, pairs) and a different overlay.
        let mut fixed = SceneItem::source(other);
        fixed.animations.moves = false;
        let to = vec![
            SceneItem::source(cam),
            SceneItem::source(other),
            SceneItem::source(cam),
            fixed,
        ];
        let (g, _) = switched(from, to, true);
        let t = g.canvases[0].transition.as_ref().unwrap();
        assert_eq!(t.from.len(), 2);
        assert_eq!(
            t.pairs,
            vec![(1, 2)],
            "the grouped camera moves to B's upper camera"
        );
        assert!(t.moves.is_some());

        let (off, _) = switched(
            vec![SceneItem::source(cam)],
            vec![SceneItem::source(cam)],
            false,
        );
        assert!(
            off.canvases[0]
                .transition
                .as_ref()
                .unwrap()
                .pairs
                .is_empty()
        );

        let mut stays = SceneItem::source(cam);
        stays.animations.moves = false;
        let (opted, _) = switched(vec![stays], vec![SceneItem::source(cam)], true);
        assert!(
            opted.canvases[0]
                .transition
                .as_ref()
                .unwrap()
                .pairs
                .is_empty()
        );
    }

    #[test]
    fn items_on_one_side_play_their_animations_within_the_transition() {
        use estream_project::AnimationKind as K;
        let [cam, alert, title] = [1u128, 2, 3].map(SourceId::from_u128);
        let mut leaving = SceneItem::source(alert);
        leaving.animations.hide = animated(K::Fade, 400);
        let mut cam_a = SceneItem::source(cam);
        cam_a.animations.hide = animated(K::Fade, 400); // in both scenes: not leaving
        let mut arriving = SceneItem::source(title);
        arriving.animations.show = animated(K::Zoom, 5000);
        let (g, _) = switched(
            vec![cam_a, leaving],
            vec![SceneItem::source(cam), arriving],
            true,
        );
        let t = g.canvases[0].transition.as_ref().unwrap();
        assert_eq!(t.leaving.len(), 1);
        let (i, run) = t.leaving[0];
        assert_eq!(
            (i, run.showing, run.start),
            (1, false, Timestamp(1_000_000_000))
        );
        assert_eq!(t.arriving.len(), 1);
        let (j, run) = t.arriving[0];
        assert_eq!(j, 1);
        assert_eq!(run.animation.duration_ms, 1000, "clamped to the transition");
        assert_eq!(
            run.end(),
            Timestamp(2_000_000_000),
            "ends with the transition"
        );
    }

    #[test]
    fn hiding_items_stay_until_their_animation_ends() {
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let mut item = SceneItem::source(SourceId::from_u128(9));
        item.visible = false;
        let id = item.id;
        p.scenes[0].items.push(item);
        let mut masked = SceneItem::source(SourceId::from_u128(8));
        masked.visible = false;
        masked.mask = Some(estream_project::ItemMask::circle(1.0));
        p.scenes[0].items.push(masked);
        let mut view = ViewState::default();
        let run = ItemAnimationRun::new(
            false,
            animated(estream_project::AnimationKind::Fade, 400).unwrap(),
            Timestamp(0),
        );
        view.item_animations.insert(id, run);
        let during = RenderGraph::build_with(&p, 1, &view, Timestamp(200_000_000));
        assert_eq!(during.canvases[0].items.len(), 1);
        assert_eq!(during.canvases[0].items[0].run, Some(run));
        assert!((run.presence(Timestamp(200_000_000)) - 0.5).abs() < 1e-6);
        let after = RenderGraph::build_with(&p, 2, &view, Timestamp(400_000_000));
        assert!(after.canvases[0].items.is_empty());
        assert_eq!(
            after.canvases[0].standby.len(),
            1,
            "hidden masked items are baked ahead"
        );
    }
}
