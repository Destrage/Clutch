//! Immutable render graph: the flattened, render-ready view of a project that
//! the engine thread hands to the video thread after every change.

use estream_core::{BlendMode, CanvasId, Crop, ItemTransform, Rational, Rgba8, SourceId};
use estream_project::{ItemContent, Project, SceneItem, SourceDef};
use glam::Affine2;

#[derive(Clone, Debug, Default)]
pub struct RenderGraph {
    pub revision: u64,
    pub canvases: Vec<CanvasGraph>,
    /// Every source in the project; all stay alive even when not on screen.
    pub sources: Vec<SourceDef>,
}

#[derive(Clone, Debug)]
pub struct CanvasGraph {
    pub id: CanvasId,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub background: Rgba8,
    /// Visible items of the active scene, bottom first.
    pub items: Vec<FlatItem>,
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
        let canvases = project
            .canvases
            .iter()
            .map(|canvas| {
                let mut items = Vec::new();
                if let Some(scene) = canvas.active_scene.and_then(|id| project.scene(id)) {
                    flatten(&scene.items, Affine2::IDENTITY, 1.0, &mut items);
                }
                CanvasGraph {
                    id: canvas.id,
                    width: canvas.width,
                    height: canvas.height,
                    fps: canvas.fps,
                    background: canvas.background,
                    items,
                }
            })
            .collect();
        Self {
            revision,
            canvases,
            sources: project.sources.clone(),
        }
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
