//! The command bus vocabulary. Every project mutation is a [`Command`]; applying
//! one yields its exact inverse, which powers undo/redo and atomic batches.

use estream_core::{
    BlendMode, CanvasId, Crop, ItemId, ItemTransform, Rational, Rgba8, SceneId, SourceId,
};
use estream_project::{Canvas, ItemContent, Project, Scene, SceneItem, SourceDef};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum CommandError {
    #[error("canvas {0:?} does not exist")]
    NoCanvas(CanvasId),
    #[error("scene {0:?} does not exist")]
    NoScene(SceneId),
    #[error("source {0:?} does not exist")]
    NoSource(SourceId),
    #[error("item {0:?} does not exist")]
    NoItem(ItemId),
    #[error("an object with id {0} already exists")]
    DuplicateId(String),
    #[error("item {0:?} is not a group")]
    NotAGroup(ItemId),
    #[error("cannot move a group into itself or its descendants")]
    Cycle,
    #[error("scene {scene:?} belongs to a different canvas than {canvas:?}")]
    WrongCanvas { scene: SceneId, canvas: CanvasId },
    #[error("invalid value: {0}")]
    Invalid(String),
}

/// Partial update of a scene item. `None` fields are left unchanged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<ItemTransform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendMode>,
}

impl ItemPatch {
    /// Bitmask of the fields this patch touches (used for undo coalescing).
    pub fn fields(&self) -> u8 {
        [
            self.name.is_some(),
            self.transform.is_some(),
            self.crop.is_some(),
            self.visible.is_some(),
            self.locked.is_some(),
            self.opacity.is_some(),
            self.blend.is_some(),
        ]
        .iter()
        .enumerate()
        .fold(0, |mask, (i, set)| mask | ((*set as u8) << i))
    }
}

/// Partial update of a canvas.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<Rational>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Rgba8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    AddCanvas {
        canvas: Canvas,
        index: usize,
    },
    /// Removes a canvas and all of its scenes.
    RemoveCanvas {
        canvas: CanvasId,
    },
    UpdateCanvas {
        canvas: CanvasId,
        patch: CanvasPatch,
    },
    AddScene {
        scene: Scene,
        index: usize,
    },
    RemoveScene {
        scene: SceneId,
    },
    RenameScene {
        scene: SceneId,
        name: String,
    },
    MoveScene {
        scene: SceneId,
        index: usize,
    },
    SetActiveScene {
        canvas: CanvasId,
        scene: Option<SceneId>,
    },
    AddSource {
        source: SourceDef,
        index: usize,
    },
    /// Removes a source and every item that references it.
    RemoveSource {
        source: SourceId,
    },
    RenameSource {
        source: SourceId,
        name: String,
    },
    SetSourceSettings {
        source: SourceId,
        settings: serde_json::Value,
    },
    AddItem {
        scene: SceneId,
        parent: Option<ItemId>,
        index: usize,
        item: SceneItem,
    },
    RemoveItem {
        scene: SceneId,
        item: ItemId,
    },
    MoveItem {
        scene: SceneId,
        item: ItemId,
        parent: Option<ItemId>,
        index: usize,
    },
    UpdateItem {
        scene: SceneId,
        item: ItemId,
        patch: ItemPatch,
    },
    /// Applies all commands or none (a transaction).
    Batch {
        commands: Vec<Command>,
    },
}

/// What changed, for UI and remote API consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    CanvasAdded {
        canvas: CanvasId,
    },
    CanvasRemoved {
        canvas: CanvasId,
    },
    CanvasChanged {
        canvas: CanvasId,
    },
    SceneAdded {
        scene: SceneId,
    },
    SceneRemoved {
        scene: SceneId,
    },
    SceneChanged {
        scene: SceneId,
    },
    ActiveSceneChanged {
        canvas: CanvasId,
        scene: Option<SceneId>,
    },
    SourceAdded {
        source: SourceId,
    },
    SourceRemoved {
        source: SourceId,
    },
    SourceChanged {
        source: SourceId,
    },
    ItemAdded {
        scene: SceneId,
        item: ItemId,
    },
    ItemRemoved {
        scene: SceneId,
        item: ItemId,
    },
    ItemMoved {
        scene: SceneId,
        item: ItemId,
    },
    ItemChanged {
        scene: SceneId,
        item: ItemId,
    },
}

/// Identifies edits that merge into one undo step when repeated quickly (e.g. drags).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CoalesceKey {
    Item(ItemId, u8),
    Canvas(CanvasId),
    SourceSettings(SourceId),
}

#[derive(Debug)]
pub struct Applied {
    /// Command that exactly reverts this application.
    pub inverse: Command,
    pub changes: Vec<Change>,
}

fn insert_index(index: usize, len: usize) -> usize {
    index.min(len)
}

impl Command {
    pub fn coalesce_key(&self) -> Option<CoalesceKey> {
        match self {
            Command::UpdateItem { item, patch, .. } => {
                Some(CoalesceKey::Item(*item, patch.fields()))
            }
            Command::UpdateCanvas { canvas, .. } => Some(CoalesceKey::Canvas(*canvas)),
            Command::SetSourceSettings { source, .. } => Some(CoalesceKey::SourceSettings(*source)),
            _ => None,
        }
    }

    /// Applies the command. On error the project is left unchanged.
    pub fn apply(self, project: &mut Project) -> Result<Applied, CommandError> {
        use Command::*;
        match self {
            AddCanvas { canvas, index } => {
                if project.canvas(canvas.id).is_some() {
                    return Err(CommandError::DuplicateId(canvas.id.to_string()));
                }
                validate_canvas(canvas.width, canvas.height, canvas.fps)?;
                if canvas.active_scene.is_some() {
                    return Err(CommandError::Invalid(
                        "new canvas cannot have an active scene".into(),
                    ));
                }
                let id = canvas.id;
                let index = insert_index(index, project.canvases.len());
                project.canvases.insert(index, canvas);
                Ok(applied(
                    RemoveCanvas { canvas: id },
                    Change::CanvasAdded { canvas: id },
                ))
            }
            RemoveCanvas { canvas } => {
                let index = project
                    .canvases
                    .iter()
                    .position(|c| c.id == canvas)
                    .ok_or(CommandError::NoCanvas(canvas))?;
                let mut cascade: Vec<Command> = Vec::new();
                if project.canvases[index].active_scene.is_some() {
                    cascade.push(SetActiveScene {
                        canvas,
                        scene: None,
                    });
                }
                cascade.extend(
                    project
                        .scenes_of(canvas)
                        .map(|s| RemoveScene { scene: s.id }),
                );
                if !cascade.is_empty() {
                    cascade.push(RemoveCanvas { canvas });
                    return Batch { commands: cascade }.apply(project);
                }
                let removed = project.canvases.remove(index);
                Ok(applied(
                    AddCanvas {
                        canvas: removed,
                        index,
                    },
                    Change::CanvasRemoved { canvas },
                ))
            }
            UpdateCanvas { canvas, patch } => {
                let c = project
                    .canvas_mut(canvas)
                    .ok_or(CommandError::NoCanvas(canvas))?;
                validate_canvas(
                    patch.width.unwrap_or(c.width),
                    patch.height.unwrap_or(c.height),
                    patch.fps.unwrap_or(c.fps),
                )?;
                let mut old = CanvasPatch::default();
                if let Some(v) = patch.name {
                    old.name = Some(std::mem::replace(&mut c.name, v));
                }
                if let Some(v) = patch.width {
                    old.width = Some(std::mem::replace(&mut c.width, v));
                }
                if let Some(v) = patch.height {
                    old.height = Some(std::mem::replace(&mut c.height, v));
                }
                if let Some(v) = patch.fps {
                    old.fps = Some(std::mem::replace(&mut c.fps, v));
                }
                if let Some(v) = patch.background {
                    old.background = Some(std::mem::replace(&mut c.background, v));
                }
                Ok(applied(
                    UpdateCanvas { canvas, patch: old },
                    Change::CanvasChanged { canvas },
                ))
            }
            AddScene { scene, index } => {
                if project.scene(scene.id).is_some() {
                    return Err(CommandError::DuplicateId(scene.id.to_string()));
                }
                if project.canvas(scene.canvas).is_none() {
                    return Err(CommandError::NoCanvas(scene.canvas));
                }
                validate_items(project, &scene.items, &mut Vec::new())?;
                let id = scene.id;
                let index = insert_index(index, project.scenes.len());
                project.scenes.insert(index, scene);
                Ok(applied(
                    RemoveScene { scene: id },
                    Change::SceneAdded { scene: id },
                ))
            }
            RemoveScene { scene } => {
                let index = project
                    .scenes
                    .iter()
                    .position(|s| s.id == scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let canvas = project.scenes[index].canvas;
                let is_active = project
                    .canvas(canvas)
                    .is_some_and(|c| c.active_scene == Some(scene));
                if is_active {
                    // Fall back to a neighbouring scene of the same canvas.
                    let fallback = project
                        .scenes_of(canvas)
                        .map(|s| s.id)
                        .find(|id| *id != scene);
                    return Batch {
                        commands: vec![
                            SetActiveScene {
                                canvas,
                                scene: fallback,
                            },
                            RemoveScene { scene },
                        ],
                    }
                    .apply(project);
                }
                let removed = project.scenes.remove(index);
                Ok(applied(
                    AddScene {
                        scene: removed,
                        index,
                    },
                    Change::SceneRemoved { scene },
                ))
            }
            RenameScene { scene, name } => {
                let s = project
                    .scene_mut(scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let old = std::mem::replace(&mut s.name, name);
                Ok(applied(
                    RenameScene { scene, name: old },
                    Change::SceneChanged { scene },
                ))
            }
            MoveScene { scene, index } => {
                let from = project
                    .scenes
                    .iter()
                    .position(|s| s.id == scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let s = project.scenes.remove(from);
                let to = insert_index(index, project.scenes.len());
                project.scenes.insert(to, s);
                Ok(applied(
                    MoveScene { scene, index: from },
                    Change::SceneChanged { scene },
                ))
            }
            SetActiveScene { canvas, scene } => {
                if let Some(sid) = scene {
                    let s = project.scene(sid).ok_or(CommandError::NoScene(sid))?;
                    if s.canvas != canvas {
                        return Err(CommandError::WrongCanvas { scene: sid, canvas });
                    }
                }
                let c = project
                    .canvas_mut(canvas)
                    .ok_or(CommandError::NoCanvas(canvas))?;
                let old = std::mem::replace(&mut c.active_scene, scene);
                Ok(applied(
                    SetActiveScene { canvas, scene: old },
                    Change::ActiveSceneChanged { canvas, scene },
                ))
            }
            AddSource { source, index } => {
                if project.source(source.id).is_some() {
                    return Err(CommandError::DuplicateId(source.id.to_string()));
                }
                if source.kind.trim().is_empty() {
                    return Err(CommandError::Invalid(
                        "source kind must not be empty".into(),
                    ));
                }
                let id = source.id;
                let index = insert_index(index, project.sources.len());
                project.sources.insert(index, source);
                Ok(applied(
                    RemoveSource { source: id },
                    Change::SourceAdded { source: id },
                ))
            }
            RemoveSource { source } => {
                let index = project
                    .sources
                    .iter()
                    .position(|s| s.id == source)
                    .ok_or(CommandError::NoSource(source))?;
                let users = project.items_using(source);
                if !users.is_empty() {
                    let mut commands: Vec<Command> = users
                        .into_iter()
                        .rev() // children before parents, later items first: indices stay valid
                        .map(|(scene, item)| RemoveItem { scene, item })
                        .collect();
                    commands.push(RemoveSource { source });
                    return Batch { commands }.apply(project);
                }
                let removed = project.sources.remove(index);
                Ok(applied(
                    AddSource {
                        source: removed,
                        index,
                    },
                    Change::SourceRemoved { source },
                ))
            }
            RenameSource { source, name } => {
                let s = project
                    .source_mut(source)
                    .ok_or(CommandError::NoSource(source))?;
                let old = std::mem::replace(&mut s.name, name);
                Ok(applied(
                    RenameSource { source, name: old },
                    Change::SourceChanged { source },
                ))
            }
            SetSourceSettings { source, settings } => {
                let s = project
                    .source_mut(source)
                    .ok_or(CommandError::NoSource(source))?;
                let old = std::mem::replace(&mut s.settings, settings);
                Ok(applied(
                    SetSourceSettings {
                        source,
                        settings: old,
                    },
                    Change::SourceChanged { source },
                ))
            }
            AddItem {
                scene,
                parent,
                index,
                item,
            } => {
                let mut ids = Vec::new();
                validate_items(project, std::slice::from_ref(&item), &mut ids)?;
                let s = project
                    .scene_mut(scene)
                    .ok_or(CommandError::NoScene(scene))?;
                if let Some(dup) = ids.iter().find(|id| s.find_item(**id).is_some()) {
                    return Err(CommandError::DuplicateId(dup.to_string()));
                }
                let siblings = match parent {
                    Some(pid) => {
                        s.find_item(pid).ok_or(CommandError::NoItem(pid))?;
                        s.children_mut(Some(pid))
                            .ok_or(CommandError::NotAGroup(pid))?
                    }
                    None => &mut s.items,
                };
                let id = item.id;
                let index = insert_index(index, siblings.len());
                siblings.insert(index, item);
                Ok(applied(
                    RemoveItem { scene, item: id },
                    Change::ItemAdded { scene, item: id },
                ))
            }
            RemoveItem { scene, item } => {
                let s = project
                    .scene_mut(scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let (parent, index) = s.locate_item(item).ok_or(CommandError::NoItem(item))?;
                let removed = s
                    .children_mut(parent)
                    .ok_or(CommandError::NoItem(item))?
                    .remove(index);
                Ok(applied(
                    AddItem {
                        scene,
                        parent,
                        index,
                        item: removed,
                    },
                    Change::ItemRemoved { scene, item },
                ))
            }
            MoveItem {
                scene,
                item,
                parent,
                index,
            } => {
                let s = project
                    .scene_mut(scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let (old_parent, old_index) =
                    s.locate_item(item).ok_or(CommandError::NoItem(item))?;
                if let Some(pid) = parent {
                    let target = s.find_item(pid).ok_or(CommandError::NoItem(pid))?;
                    if !target.is_group() {
                        return Err(CommandError::NotAGroup(pid));
                    }
                    if s.find_item(item).is_some_and(|i| i.contains(pid)) {
                        return Err(CommandError::Cycle);
                    }
                }
                let moved = s
                    .children_mut(old_parent)
                    .ok_or(CommandError::NoItem(item))?
                    .remove(old_index);
                let siblings = s.children_mut(parent).ok_or(CommandError::NoItem(item))?;
                let to = insert_index(index, siblings.len());
                siblings.insert(to, moved);
                Ok(applied(
                    MoveItem {
                        scene,
                        item,
                        parent: old_parent,
                        index: old_index,
                    },
                    Change::ItemMoved { scene, item },
                ))
            }
            UpdateItem { scene, item, patch } => {
                if let Some(o) = patch.opacity
                    && !(0.0..=1.0).contains(&o)
                {
                    return Err(CommandError::Invalid(format!(
                        "opacity {o} is outside 0..=1"
                    )));
                }
                let s = project
                    .scene_mut(scene)
                    .ok_or(CommandError::NoScene(scene))?;
                let it = s.find_item_mut(item).ok_or(CommandError::NoItem(item))?;
                let mut old = ItemPatch::default();
                if let Some(v) = patch.name {
                    old.name = Some(std::mem::replace(&mut it.name, v));
                }
                if let Some(v) = patch.transform {
                    old.transform = Some(std::mem::replace(&mut it.transform, v));
                }
                if let Some(v) = patch.crop {
                    old.crop = Some(std::mem::replace(&mut it.crop, v));
                }
                if let Some(v) = patch.visible {
                    old.visible = Some(std::mem::replace(&mut it.visible, v));
                }
                if let Some(v) = patch.locked {
                    old.locked = Some(std::mem::replace(&mut it.locked, v));
                }
                if let Some(v) = patch.opacity {
                    old.opacity = Some(std::mem::replace(&mut it.opacity, v));
                }
                if let Some(v) = patch.blend {
                    old.blend = Some(std::mem::replace(&mut it.blend, v));
                }
                Ok(applied(
                    UpdateItem {
                        scene,
                        item,
                        patch: old,
                    },
                    Change::ItemChanged { scene, item },
                ))
            }
            Batch { commands } => {
                let mut inverses = Vec::with_capacity(commands.len());
                let mut changes = Vec::new();
                for command in commands {
                    match command.apply(project) {
                        Ok(a) => {
                            inverses.push(a.inverse);
                            changes.extend(a.changes);
                        }
                        Err(err) => {
                            // Roll back everything applied so far, newest first.
                            for inverse in inverses.into_iter().rev() {
                                if let Err(e) = inverse.apply(project) {
                                    tracing::error!(error = %e, "rollback failed; project may be inconsistent");
                                }
                            }
                            return Err(err);
                        }
                    }
                }
                inverses.reverse();
                Ok(Applied {
                    inverse: Batch { commands: inverses },
                    changes,
                })
            }
        }
    }
}

fn applied(inverse: Command, change: Change) -> Applied {
    Applied {
        inverse,
        changes: vec![change],
    }
}

fn validate_canvas(width: u32, height: u32, fps: Rational) -> Result<(), CommandError> {
    if !(16..=16384).contains(&width) || !(16..=16384).contains(&height) {
        return Err(CommandError::Invalid(format!(
            "canvas size {width}x{height} is outside 16..=16384"
        )));
    }
    if !fps.is_valid() || fps.as_f64() > 1000.0 {
        return Err(CommandError::Invalid(format!(
            "frame rate {}/{} is invalid",
            fps.num, fps.den
        )));
    }
    Ok(())
}

/// Checks that items reference existing sources and have unique IDs; collects their IDs.
fn validate_items(
    project: &Project,
    items: &[SceneItem],
    ids: &mut Vec<ItemId>,
) -> Result<(), CommandError> {
    for item in items {
        if ids.contains(&item.id) {
            return Err(CommandError::DuplicateId(item.id.to_string()));
        }
        ids.push(item.id);
        if !(0.0..=1.0).contains(&item.opacity) {
            return Err(CommandError::Invalid(format!(
                "opacity {} is outside 0..=1",
                item.opacity
            )));
        }
        match &item.content {
            ItemContent::Source { source } => {
                project
                    .source(*source)
                    .ok_or(CommandError::NoSource(*source))?;
            }
            ItemContent::Group { items } => validate_items(project, items, ids)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_project::SourceDef;
    use glam::Vec2;

    fn project() -> (Project, SceneId) {
        let p = Project::new_default(1920, 1080, Rational::fps(60));
        let scene = p.scenes[0].id;
        (p, scene)
    }

    fn add_source(p: &mut Project) -> SourceId {
        let src = SourceDef::new("Cam", "color", serde_json::json!({}));
        let id = src.id;
        Command::AddSource {
            source: src,
            index: usize::MAX,
        }
        .apply(p)
        .unwrap();
        id
    }

    #[test]
    fn invalid_reference_is_rejected_without_changes() {
        let (mut p, scene) = project();
        let before = p.clone();
        let err = Command::AddItem {
            scene,
            parent: None,
            index: 0,
            item: SceneItem::source(SourceId::new()),
        }
        .apply(&mut p)
        .unwrap_err();
        assert!(matches!(err, CommandError::NoSource(_)));
        assert_eq!(p, before);
    }

    #[test]
    fn failed_batch_rolls_back() {
        let (mut p, scene) = project();
        let src = add_source(&mut p);
        let before = p.clone();
        let item = SceneItem::source(src);
        let err = Command::Batch {
            commands: vec![
                Command::RenameScene {
                    scene,
                    name: "Renamed".into(),
                },
                Command::AddItem {
                    scene,
                    parent: None,
                    index: 0,
                    item,
                },
                Command::RemoveScene {
                    scene: SceneId::new(),
                },
            ],
        }
        .apply(&mut p)
        .unwrap_err();
        assert!(matches!(err, CommandError::NoScene(_)));
        assert_eq!(p, before);
    }

    #[test]
    fn remove_shared_source_and_restore_with_original_ids() {
        let (mut p, scene_a) = project();
        let canvas = p.canvases[0].id;
        let scene_b = Scene::new("B", canvas);
        let scene_b_id = scene_b.id;
        Command::AddScene {
            scene: scene_b,
            index: 1,
        }
        .apply(&mut p)
        .unwrap();
        let src = add_source(&mut p);
        let other = add_source(&mut p);
        for scene in [scene_a, scene_b_id] {
            for s in [other, src, other] {
                Command::AddItem {
                    scene,
                    parent: None,
                    index: usize::MAX,
                    item: SceneItem::source(s),
                }
                .apply(&mut p)
                .unwrap();
            }
        }
        // Also nest a use inside a group.
        let group = SceneItem::group("G", vec![SceneItem::source(other), SceneItem::source(src)]);
        Command::AddItem {
            scene: scene_a,
            parent: None,
            index: 1,
            item: group,
        }
        .apply(&mut p)
        .unwrap();
        let before = p.clone();

        let applied = Command::RemoveSource { source: src }.apply(&mut p).unwrap();
        assert!(p.source(src).is_none());
        assert!(p.items_using(src).is_empty());
        assert_eq!(p.items_using(other).len(), 5);

        applied.inverse.apply(&mut p).unwrap();
        assert_eq!(p, before, "source, items, ids and order fully restored");
    }

    #[test]
    fn removing_active_scene_picks_fallback_and_undo_restores() {
        let (mut p, scene_a) = project();
        let canvas = p.canvases[0].id;
        let b = Scene::new("B", canvas);
        let b_id = b.id;
        Command::AddScene { scene: b, index: 1 }
            .apply(&mut p)
            .unwrap();
        let before = p.clone();
        let applied = Command::RemoveScene { scene: scene_a }
            .apply(&mut p)
            .unwrap();
        assert_eq!(p.canvases[0].active_scene, Some(b_id));
        applied.inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn move_into_own_descendant_is_rejected() {
        let (mut p, scene) = project();
        let inner = SceneItem::group("inner", vec![]);
        let inner_id = inner.id;
        let outer = SceneItem::group("outer", vec![inner]);
        let outer_id = outer.id;
        Command::AddItem {
            scene,
            parent: None,
            index: 0,
            item: outer,
        }
        .apply(&mut p)
        .unwrap();
        let err = Command::MoveItem {
            scene,
            item: outer_id,
            parent: Some(inner_id),
            index: 0,
        }
        .apply(&mut p)
        .unwrap_err();
        assert_eq!(err, CommandError::Cycle);
    }

    #[test]
    fn update_item_inverse_restores_only_patched_fields() {
        let (mut p, scene) = project();
        let src = add_source(&mut p);
        let item = SceneItem::source(src);
        let id = item.id;
        Command::AddItem {
            scene,
            parent: None,
            index: 0,
            item,
        }
        .apply(&mut p)
        .unwrap();
        let before = p.clone();
        let transform = ItemTransform {
            position: Vec2::new(10.0, 20.0),
            ..Default::default()
        };
        let a = Command::UpdateItem {
            scene,
            item: id,
            patch: ItemPatch {
                transform: Some(transform),
                opacity: Some(0.5),
                ..Default::default()
            },
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(a.changes, vec![Change::ItemChanged { scene, item: id }]);
        let it = p.scene(scene).unwrap().find_item(id).unwrap();
        assert_eq!(it.transform.position, Vec2::new(10.0, 20.0));
        assert_eq!(it.opacity, 0.5);
        a.inverse.apply(&mut p).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn commands_serialize() {
        let cmd = Command::SetActiveScene {
            canvas: CanvasId::new(),
            scene: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"set_active_scene\""));
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    }
}
