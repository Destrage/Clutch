//! Project data model. Plain data: all mutation happens through engine commands.

use estream_core::{
    BlendMode, CanvasId, Crop, ItemId, ItemTransform, Rational, Rgba8, SceneId, SourceId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub canvases: Vec<Canvas>,
    /// All scenes, in display order (each belongs to one canvas).
    pub scenes: Vec<Scene>,
    pub sources: Vec<SourceDef>,
    #[serde(default)]
    pub audio: ProjectAudio,
    /// Transition used when the program scene changes.
    #[serde(default)]
    pub transition: crate::Transition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub id: CanvasId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    #[serde(default = "default_background")]
    pub background: Rgba8,
    #[serde(default)]
    pub active_scene: Option<SceneId>,
}

fn default_background() -> Rgba8 {
    Rgba8::BLACK
}

impl Canvas {
    pub fn new(name: impl Into<String>, width: u32, height: u32, fps: Rational) -> Self {
        Self {
            id: CanvasId::new(),
            name: name.into(),
            width,
            height,
            fps,
            background: Rgba8::BLACK,
            active_scene: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub canvas: CanvasId,
    /// Items in z-order, bottom first.
    #[serde(default)]
    pub items: Vec<SceneItem>,
}

impl Scene {
    pub fn new(name: impl Into<String>, canvas: CanvasId) -> Self {
        Self {
            id: SceneId::new(),
            name: name.into(),
            canvas,
            items: Vec::new(),
        }
    }

    pub fn find_item(&self, id: ItemId) -> Option<&SceneItem> {
        find_in(&self.items, id)
    }

    pub fn find_item_mut(&mut self, id: ItemId) -> Option<&mut SceneItem> {
        find_in_mut(&mut self.items, id)
    }

    /// Returns `(parent group, index within parent)` of an item.
    pub fn locate_item(&self, id: ItemId) -> Option<(Option<ItemId>, usize)> {
        locate_in(&self.items, None, id)
    }

    /// The child list of `parent` (or the scene root when `None`).
    pub fn children_mut(&mut self, parent: Option<ItemId>) -> Option<&mut Vec<SceneItem>> {
        match parent {
            None => Some(&mut self.items),
            Some(pid) => match &mut self.find_item_mut(pid)?.content {
                ItemContent::Group { items } => Some(items),
                ItemContent::Source { .. } => None,
            },
        }
    }

    /// Visits every item depth-first (parents before children).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a SceneItem)) {
        fn rec<'a>(items: &'a [SceneItem], f: &mut impl FnMut(&'a SceneItem)) {
            for item in items {
                f(item);
                if let ItemContent::Group { items } = &item.content {
                    rec(items, f);
                }
            }
        }
        rec(&self.items, f);
    }
}

fn find_in(items: &[SceneItem], id: ItemId) -> Option<&SceneItem> {
    items.iter().find_map(|item| {
        if item.id == id {
            Some(item)
        } else if let ItemContent::Group { items } = &item.content {
            find_in(items, id)
        } else {
            None
        }
    })
}

fn find_in_mut(items: &mut [SceneItem], id: ItemId) -> Option<&mut SceneItem> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let ItemContent::Group { items } = &mut item.content
            && let Some(found) = find_in_mut(items, id)
        {
            return Some(found);
        }
    }
    None
}

fn locate_in(
    items: &[SceneItem],
    parent: Option<ItemId>,
    id: ItemId,
) -> Option<(Option<ItemId>, usize)> {
    for (index, item) in items.iter().enumerate() {
        if item.id == id {
            return Some((parent, index));
        }
        if let ItemContent::Group { items } = &item.content
            && let Some(found) = locate_in(items, Some(item.id), id)
        {
            return Some(found);
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemContent {
    Source { source: SourceId },
    Group { items: Vec<SceneItem> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneItem {
    pub id: ItemId,
    /// Optional display name override (groups always use this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub content: ItemContent,
    #[serde(default)]
    pub transform: ItemTransform,
    #[serde(default)]
    pub crop: Crop,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: BlendMode,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl SceneItem {
    pub fn source(source: SourceId) -> Self {
        Self::with_content(ItemContent::Source { source })
    }

    pub fn group(name: impl Into<String>, items: Vec<SceneItem>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::with_content(ItemContent::Group { items })
        }
    }

    fn with_content(content: ItemContent) -> Self {
        Self {
            id: ItemId::new(),
            name: None,
            content,
            transform: ItemTransform::default(),
            crop: Crop::NONE,
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
        }
    }

    pub fn source_id(&self) -> Option<SourceId> {
        match self.content {
            ItemContent::Source { source } => Some(source),
            ItemContent::Group { .. } => None,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.content, ItemContent::Group { .. })
    }

    /// True if `id` is this item or one of its descendants.
    pub fn contains(&self, id: ItemId) -> bool {
        self.id == id
            || match &self.content {
                ItemContent::Group { items } => items.iter().any(|i| i.contains(id)),
                ItemContent::Source { .. } => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceDef {
    pub id: SourceId,
    pub name: String,
    /// Source type identifier, e.g. `"color"`, `"display_capture"`.
    pub kind: String,
    #[serde(default)]
    pub settings: serde_json::Value,
    /// Mixer settings (used by source types that produce audio).
    #[serde(default, skip_serializing_if = "AudioConfig::is_default")]
    pub audio: AudioConfig,
    /// Video filters, applied in order wherever the source appears.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<FilterDef>,
}

/// How a source is monitored (heard locally by the streamer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Monitoring {
    #[default]
    Off,
    /// Heard locally, not sent to outputs.
    MonitorOnly,
    /// Heard locally and sent to outputs.
    MonitorAndOutput,
}

/// One filter in a source's audio or video chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterDef {
    pub id: String,
    pub kind: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// Audio filters use the common filter shape.
pub type AudioFilterDef = FilterDef;

/// Per-source mixer settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub volume_db: f32,
    pub muted: bool,
    /// 0 = left, 0.5 = center, 1 = right.
    pub balance: f32,
    pub mono: bool,
    pub sync_offset_ms: u32,
    pub monitoring: Monitoring,
    /// Bitmask of output tracks 1–6 (bit 0 = track 1).
    pub tracks: u8,
    /// Mixed in every scene (like OBS's global audio devices).
    pub global: bool,
    pub filters: Vec<AudioFilterDef>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            muted: false,
            balance: 0.5,
            mono: false,
            sync_offset_ms: 0,
            monitoring: Monitoring::Off,
            tracks: 0b11_1111,
            global: false,
            filters: Vec::new(),
        }
    }
}

impl AudioConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Project-wide audio settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectAudio {
    /// Monitoring output device id (empty = system default).
    pub monitor_device: String,
}

impl SourceDef {
    pub fn new(
        name: impl Into<String>,
        kind: impl Into<String>,
        settings: serde_json::Value,
    ) -> Self {
        Self {
            id: SourceId::new(),
            name: name.into(),
            kind: kind.into(),
            settings,
            audio: AudioConfig::default(),
            filters: Vec::new(),
        }
    }
}

impl Project {
    /// A new project with one canvas and one empty, active scene.
    pub fn new_default(width: u32, height: u32, fps: Rational) -> Self {
        let mut canvas = Canvas::new("Main", width, height, fps);
        let scene = Scene::new("Scene", canvas.id);
        canvas.active_scene = Some(scene.id);
        Self {
            canvases: vec![canvas],
            scenes: vec![scene],
            sources: Vec::new(),
            audio: ProjectAudio::default(),
            transition: crate::Transition::default(),
        }
    }

    pub fn canvas(&self, id: CanvasId) -> Option<&Canvas> {
        self.canvases.iter().find(|c| c.id == id)
    }

    pub fn canvas_mut(&mut self, id: CanvasId) -> Option<&mut Canvas> {
        self.canvases.iter_mut().find(|c| c.id == id)
    }

    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn scene_mut(&mut self, id: SceneId) -> Option<&mut Scene> {
        self.scenes.iter_mut().find(|s| s.id == id)
    }

    pub fn source(&self, id: SourceId) -> Option<&SourceDef> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut SourceDef> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    pub fn scenes_of(&self, canvas: CanvasId) -> impl Iterator<Item = &Scene> {
        self.scenes.iter().filter(move |s| s.canvas == canvas)
    }

    /// All `(scene, item)` pairs that reference `source`.
    pub fn items_using(&self, source: SourceId) -> Vec<(SceneId, ItemId)> {
        let mut out = Vec::new();
        for scene in &self.scenes {
            scene.walk(&mut |item| {
                if item.source_id() == Some(source) {
                    out.push((scene.id, item.id));
                }
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_lookup_and_location() {
        let src = SourceId::new();
        let child = SceneItem::source(src);
        let child_id = child.id;
        let group = SceneItem::group("G", vec![SceneItem::source(src), child]);
        let group_id = group.id;
        let mut scene = Scene::new("S", CanvasId::new());
        scene.items.push(SceneItem::source(src));
        scene.items.push(group);

        assert_eq!(scene.locate_item(child_id), Some((Some(group_id), 1)));
        assert_eq!(scene.locate_item(group_id), Some((None, 1)));
        assert!(scene.find_item(group_id).unwrap().contains(child_id));
        assert_eq!(scene.children_mut(Some(group_id)).unwrap().len(), 2);

        let mut count = 0;
        scene.walk(&mut |_| count += 1);
        assert_eq!(count, 4);
    }
}
