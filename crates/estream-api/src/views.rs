//! How eStream's model looks through obs-websocket: ids, names, kinds and JSON shapes shared
//! by requests and events.

use estream_core::{Alignment, BoundsMode, ItemId, ItemTransform, Size};
use estream_engine::Engine;
use estream_project::{
    AudioConfig, FilterDef, Monitoring, Project, Scene, SceneItem, SourceDef, TransitionKind,
};
use glam::Vec2;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Integer scene item ids (obs-websocket) for eStream's UUID item ids, stable per session.
#[derive(Debug, Default)]
pub struct ItemIds {
    by_item: HashMap<ItemId, i64>,
    by_number: HashMap<i64, ItemId>,
    next: i64,
}

impl ItemIds {
    pub fn number(&mut self, item: ItemId) -> i64 {
        if let Some(n) = self.by_item.get(&item) {
            return *n;
        }
        self.next += 1;
        self.by_item.insert(item, self.next);
        self.by_number.insert(self.next, item);
        self.next
    }

    pub fn item(&self, number: i64) -> Option<ItemId> {
        self.by_number.get(&number).copied()
    }
}

/// eStream source kinds and the OBS input kinds controllers know them by.
const KIND_ALIASES: &[(&str, &str)] = &[
    ("browser", "browser_source"),
    ("media", "ffmpeg_source"),
    ("image", "image_source"),
    ("color", "color_source_v3"),
    ("text", "text_gdiplus_v3"),
    ("display_capture", "monitor_capture"),
    ("window_capture", "window_capture"),
    ("game_capture", "game_capture"),
    ("camera", "dshow_input"),
    ("audio_input", "wasapi_input_capture"),
    ("audio_output", "wasapi_output_capture"),
    ("app_audio", "wasapi_process_output_capture"),
];

/// The OBS input kind for an eStream kind (unchanged if OBS has no equivalent).
pub fn obs_kind(kind: &str) -> &str {
    KIND_ALIASES
        .iter()
        .find(|(e, _)| *e == kind)
        .map_or(kind, |(_, o)| o)
}

/// The eStream kind for an OBS input kind (or an eStream kind, versioned or not).
pub fn estream_kind(kind: &str) -> String {
    let base = unversioned(kind);
    KIND_ALIASES
        .iter()
        .find(|(_, o)| *o == kind || unversioned(o) == base)
        .map_or_else(|| kind.to_owned(), |(e, _)| (*e).to_owned())
}

/// `color_source_v3` → `color_source`.
pub fn unversioned(kind: &str) -> &str {
    match kind.rsplit_once("_v") {
        Some((base, version))
            if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => kind,
    }
}

pub fn main_program(p: &Project) -> Option<&Scene> {
    p.canvases
        .first()
        .and_then(|c| c.active_scene)
        .and_then(|id| p.scene(id))
}

/// `GetSceneList` scenes: last scene first, `sceneIndex` 0 at the bottom (as OBS does).
pub fn scene_list(p: &Project) -> Vec<Value> {
    let ordered = scenes_in_order(p);
    let n = ordered.len();
    ordered
        .iter()
        .enumerate()
        .rev()
        .map(|(i, s)| {
            let canvas = p.canvas(s.canvas);
            json!({
                "sceneIndex": n - 1 - i,
                "sceneName": s.name,
                "sceneUuid": s.id.to_string(),
                "canvasName": canvas.map(|c| c.name.as_str()),
                "canvasUuid": s.canvas.to_string(),
            })
        })
        .collect()
}

/// Scenes grouped by canvas (main canvas first), in list order.
pub fn scenes_in_order(p: &Project) -> Vec<&Scene> {
    p.canvases.iter().flat_map(|c| p.scenes_of(c.id)).collect()
}

pub fn input_json(s: &SourceDef) -> Value {
    let kind = obs_kind(&s.kind);
    json!({
        "inputName": s.name,
        "inputUuid": s.id.to_string(),
        "inputKind": kind,
        "unversionedInputKind": unversioned(kind),
        "estreamKind": s.kind,
    })
}

pub fn volume_mul(db: f32) -> f64 {
    if db <= -100.0 {
        0.0
    } else {
        10f64.powf(db as f64 / 20.0)
    }
}

pub fn mul_to_db(mul: f64) -> f32 {
    if mul <= 0.000_01 {
        -100.0
    } else {
        (20.0 * mul.log10()) as f32
    }
}

pub fn monitor_type(m: Monitoring) -> &'static str {
    match m {
        Monitoring::Off => "OBS_MONITORING_TYPE_NONE",
        Monitoring::MonitorOnly => "OBS_MONITORING_TYPE_MONITOR_ONLY",
        Monitoring::MonitorAndOutput => "OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT",
    }
}

pub fn parse_monitor_type(s: &str) -> Option<Monitoring> {
    match s {
        "OBS_MONITORING_TYPE_NONE" => Some(Monitoring::Off),
        "OBS_MONITORING_TYPE_MONITOR_ONLY" => Some(Monitoring::MonitorOnly),
        "OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT" => Some(Monitoring::MonitorAndOutput),
        _ => None,
    }
}

pub fn tracks_json(audio: &AudioConfig) -> Value {
    let mut map = serde_json::Map::new();
    for i in 0..6 {
        map.insert(
            (i + 1).to_string(),
            Value::Bool(audio.tracks & (1 << i) != 0),
        );
    }
    Value::Object(map)
}

/// Every filter of a source: video filters, then audio filters.
pub fn filters(s: &SourceDef) -> impl Iterator<Item = &FilterDef> {
    s.filters.iter().chain(s.audio.filters.iter())
}

pub fn filter_json(index: usize, f: &FilterDef) -> Value {
    json!({
        "filterEnabled": f.enabled,
        "filterIndex": index,
        "filterKind": f.kind,
        "filterName": f.id,
        "filterSettings": f.settings,
    })
}

/// Transition name (as listed) and OBS kind for a transition kind.
pub fn transition_names(kind: &TransitionKind) -> (&'static str, &'static str) {
    let obs = match kind {
        TransitionKind::Cut => "cut_transition",
        TransitionKind::Fade => "fade_transition",
        TransitionKind::FadeToColor { .. } => "fade_to_color_transition",
        TransitionKind::Slide { .. } => "slide_transition",
        TransitionKind::Swipe { .. } => "swipe_transition",
        TransitionKind::Wipe { .. } => "wipe_transition",
        TransitionKind::Stinger { .. } => "obs_stinger_transition",
        TransitionKind::Shader { .. } => "shader_transition",
    };
    (kind.label(), obs)
}

/// One of each transition kind (stinger and shader with empty settings).
pub fn transition_kinds() -> Vec<TransitionKind> {
    use estream_project::Direction;
    vec![
        TransitionKind::Cut,
        TransitionKind::Fade,
        TransitionKind::FadeToColor {
            color: estream_core::Rgba8::BLACK,
        },
        TransitionKind::Slide {
            direction: Direction::Left,
        },
        TransitionKind::Swipe {
            direction: Direction::Left,
        },
        TransitionKind::Wipe {
            direction: Direction::Left,
            softness: 0.1,
        },
        TransitionKind::Stinger {
            file: String::new(),
            point_ms: 0,
        },
        TransitionKind::Shader {
            code: String::new(),
        },
    ]
}

/// The name of the scene or group that directly contains `item`.
pub struct Placement<'a> {
    pub scene: &'a Scene,
    /// Group item that contains it (`None` = top level).
    pub group: Option<&'a SceneItem>,
    pub index: usize,
}

impl Placement<'_> {
    pub fn container_name(&self) -> &str {
        match self.group {
            Some(g) => g.name.as_deref().unwrap_or("Group"),
            None => &self.scene.name,
        }
    }

    pub fn container_uuid(&self) -> String {
        match self.group {
            Some(g) => g.id.to_string(),
            None => self.scene.id.to_string(),
        }
    }
}

pub fn locate<'a>(scene: &'a Scene, item: ItemId) -> Option<Placement<'a>> {
    let (parent, index) = scene.locate_item(item)?;
    let group = parent.and_then(|g| scene.find_item(g));
    Some(Placement {
        scene,
        group,
        index,
    })
}

const ALIGN_LEFT: u32 = 1;
const ALIGN_RIGHT: u32 = 2;
const ALIGN_TOP: u32 = 4;
const ALIGN_BOTTOM: u32 = 8;

pub fn alignment_bits(a: Alignment) -> u32 {
    use Alignment::*;
    let x = match a {
        TopLeft | Left | BottomLeft => ALIGN_LEFT,
        TopRight | Right | BottomRight => ALIGN_RIGHT,
        _ => 0,
    };
    let y = match a {
        TopLeft | Top | TopRight => ALIGN_TOP,
        BottomLeft | Bottom | BottomRight => ALIGN_BOTTOM,
        _ => 0,
    };
    x | y
}

pub fn alignment_from_bits(bits: u32) -> Alignment {
    use Alignment::*;
    let left = bits & ALIGN_LEFT != 0;
    let right = bits & ALIGN_RIGHT != 0;
    let top = bits & ALIGN_TOP != 0;
    let bottom = bits & ALIGN_BOTTOM != 0;
    match (left, right, top, bottom) {
        (true, _, true, _) => TopLeft,
        (true, _, _, true) => BottomLeft,
        (true, _, _, _) => Left,
        (_, true, true, _) => TopRight,
        (_, true, _, true) => BottomRight,
        (_, true, _, _) => Right,
        (_, _, true, _) => Top,
        (_, _, _, true) => Bottom,
        _ => Center,
    }
}

pub fn bounds_type(m: BoundsMode) -> &'static str {
    match m {
        BoundsMode::None => "OBS_BOUNDS_NONE",
        BoundsMode::Stretch => "OBS_BOUNDS_STRETCH",
        BoundsMode::Fit => "OBS_BOUNDS_SCALE_INNER",
        BoundsMode::Fill => "OBS_BOUNDS_SCALE_OUTER",
    }
}

pub fn parse_bounds_type(s: &str) -> Option<BoundsMode> {
    match s {
        "OBS_BOUNDS_NONE" => Some(BoundsMode::None),
        "OBS_BOUNDS_STRETCH" => Some(BoundsMode::Stretch),
        "OBS_BOUNDS_SCALE_INNER"
        | "OBS_BOUNDS_MAX_ONLY"
        | "OBS_BOUNDS_SCALE_TO_WIDTH"
        | "OBS_BOUNDS_SCALE_TO_HEIGHT" => Some(BoundsMode::Fit),
        "OBS_BOUNDS_SCALE_OUTER" => Some(BoundsMode::Fill),
        _ => None,
    }
}

/// `sceneItemTransform` (position is the top-left corner: alignment 5).
pub fn transform_json(engine: Option<&Engine>, item: &SceneItem) -> Value {
    let size = item
        .source_id()
        .and_then(|s| engine.and_then(|e| e.source_size(s)))
        .unwrap_or(Size::new(0.0, 0.0));
    let t: &ItemTransform = &item.transform;
    let layout = t.layout(item.crop, size);
    let cropped = Vec2::new(
        (size.width - item.crop.left as f32 - item.crop.right as f32).max(0.0),
        (size.height - item.crop.top as f32 - item.crop.bottom as f32).max(0.0),
    );
    let scale = if t.bounds_mode == BoundsMode::None || cropped.x <= 0.0 || cropped.y <= 0.0 {
        t.scale
    } else {
        layout.size / cropped
    };
    json!({
        "positionX": t.position.x,
        "positionY": t.position.y,
        "rotation": t.rotation,
        "scaleX": scale.x,
        "scaleY": scale.y,
        "width": layout.size.x,
        "height": layout.size.y,
        "sourceWidth": size.width,
        "sourceHeight": size.height,
        "alignment": ALIGN_LEFT | ALIGN_TOP,
        "boundsType": bounds_type(t.bounds_mode),
        "boundsAlignment": alignment_bits(t.bounds_alignment),
        "boundsWidth": t.bounds.x,
        "boundsHeight": t.bounds.y,
        "cropLeft": item.crop.left,
        "cropRight": item.crop.right,
        "cropTop": item.crop.top,
        "cropBottom": item.crop.bottom,
        "cropToBounds": false,
    })
}

pub fn blend_mode(item: &SceneItem) -> &'static str {
    use estream_core::BlendMode::*;
    match item.blend {
        Normal => "OBS_BLEND_NORMAL",
        Additive => "OBS_BLEND_ADDITIVE",
        Multiply => "OBS_BLEND_MULTIPLY",
        Screen => "OBS_BLEND_SCREEN",
    }
}

/// A `GetSceneItemList` entry.
pub fn item_json(
    p: &Project,
    engine: Option<&Engine>,
    ids: &mut ItemIds,
    item: &SceneItem,
    index: usize,
) -> Value {
    let source = item.source_id().and_then(|s| p.source(s));
    let (name, uuid, kind) = match (source, &item.name) {
        (Some(s), _) => (s.name.clone(), s.id.to_string(), Some(obs_kind(&s.kind))),
        (None, name) => (
            name.clone().unwrap_or_else(|| "Group".into()),
            item.id.to_string(),
            None,
        ),
    };
    json!({
        "sceneItemId": ids.number(item.id),
        "sceneItemIndex": index,
        "sourceName": name,
        "sourceUuid": uuid,
        "sourceType": if item.is_group() { "OBS_SOURCE_TYPE_SCENE" } else { "OBS_SOURCE_TYPE_INPUT" },
        "inputKind": kind,
        "isGroup": if item.is_group() { Value::Bool(true) } else { Value::Null },
        "sceneItemEnabled": item.visible,
        "sceneItemLocked": item.locked,
        "sceneItemBlendMode": blend_mode(item),
        "sceneItemTransform": transform_json(engine, item),
    })
}

/// Formats a duration as OBS does: `HH:MM:SS.mmm`.
pub fn timecode(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_map_both_ways() {
        assert_eq!(obs_kind("browser"), "browser_source");
        assert_eq!(estream_kind("browser_source"), "browser");
        assert_eq!(estream_kind("color_source_v3"), "color");
        assert_eq!(estream_kind("color_source"), "color");
        assert_eq!(estream_kind("text"), "text");
        assert_eq!(unversioned("text_gdiplus_v3"), "text_gdiplus");
        assert_eq!(unversioned("wasapi_input_capture"), "wasapi_input_capture");
    }

    #[test]
    fn alignment_bits_round_trip() {
        for a in [
            Alignment::TopLeft,
            Alignment::Top,
            Alignment::TopRight,
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::BottomLeft,
            Alignment::Bottom,
            Alignment::BottomRight,
        ] {
            assert_eq!(alignment_from_bits(alignment_bits(a)), a);
        }
        assert_eq!(alignment_bits(Alignment::TopLeft), 5);
    }

    #[test]
    fn volume_conversions() {
        assert!((volume_mul(0.0) - 1.0).abs() < 1e-9);
        assert!((mul_to_db(0.5) - -6.0206).abs() < 1e-3);
        assert_eq!(volume_mul(-100.0), 0.0);
        assert_eq!(mul_to_db(0.0), -100.0);
    }

    #[test]
    fn timecodes() {
        assert_eq!(
            timecode(std::time::Duration::from_millis(3_723_045)),
            "01:02:03.045"
        );
    }
}
