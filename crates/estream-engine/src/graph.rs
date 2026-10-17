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
        let canvases = project
            .canvases
            .iter()
            .map(|canvas| {
                let transition = view
                    .transitions
                    .get(&canvas.id)
                    .filter(|t| t.end() > now)
                    .map(|t| TransitionGraph {
                        from: scene_items(t.from),
                        kind: t.transition.kind.clone(),
                        start: t.start,
                        duration_ns: t.transition.duration_ms as u64 * 1_000_000,
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
        Self {
            revision,
            canvases,
            sources: project.sources.clone(),
            audio: audio_graph(project, view, now),
        }
    }
}

/// Global audio sources, then sources visible in the first canvas's active scene (and in the
/// scene being left while a transition runs).
fn audio_graph(project: &Project, view: &ViewState, now: Timestamp) -> AudioGraph {
    let mut ids: Vec<SourceId> = project
        .sources
        .iter()
        .filter(|s| s.audio.global)
        .map(|s| s.id)
        .collect();
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
    if let Some(canvas) = project.canvases.first() {
        let leaving = view
            .transitions
            .get(&canvas.id)
            .filter(|t| t.end() > now)
            .and_then(|t| t.from);
        for id in [canvas.active_scene, leaving].into_iter().flatten() {
            if let Some(scene) = project.scene(id) {
                visible(&scene.items, &mut ids);
            }
        }
    }
    AudioGraph {
        nodes: ids
            .into_iter()
            .filter_map(|id| project.source(id))
            .map(|s| AudioNode {
                source: s.id,
                kind: s.kind.clone(),
                settings: s.settings.clone(),
                config: s.audio.clone(),
            })
            .collect(),
        monitor_device: project.audio.monitor_device.clone(),
    }
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
}
