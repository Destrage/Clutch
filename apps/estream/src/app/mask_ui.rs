//! The app side of item masks: the mask editor over the preview, and the inspector's Mask and
//! Edge effects panels. Every change is an item command; a gesture is one undo step.

use super::*;
use crate::mask_edit::{Editor, Frame, Mods, Tool, outline, polyline};
use crate::ui::{MaskEditView, MaskHandle, MaskInspect, MaskOutline, MaskShapeRow};
use estream_project::{
    Border, BorderPlacement, Glow, GlowPlacement, ItemMask, MaskEffects, MaskGeometry, MaskMode,
    Shadow,
};

/// Inspector order of the border and glow placements.
const BORDER_PLACES: [BorderPlacement; 3] = [
    BorderPlacement::Inside,
    BorderPlacement::Center,
    BorderPlacement::Outside,
];
const GLOW_PLACES: [GlowPlacement; 3] = [
    GlowPlacement::Outside,
    GlowPlacement::Inside,
    GlowPlacement::Both,
];
/// Corner radius the Rounded corners preset starts with, in canvas pixels.
const ROUNDED_RADIUS: f32 = 24.0;
/// Long side of the dim picture shown while editing.
const DIM_SIDE: f32 = 256.0;

/// What the mask editor dims, kept until the mask or the item's size changes.
pub(super) struct MaskDim {
    mask: ItemMask,
    size: [u32; 2],
    image: slint::Image,
}

impl App {
    /// The selected item's visible picture on the canvas (its unit square in canvas pixels).
    fn mask_frame(&self, item: &SceneItem) -> Option<Frame> {
        let g = self.geoms.iter().find(|g| g.id == item.id)?;
        if g.is_group || g.source_size.min_element() <= 0.0 {
            return None;
        }
        let size = estream_core::Size::new(g.source_size.x, g.source_size.y);
        let layout = item.transform.layout(item.crop, size);
        (layout.size.min_element() > 0.0).then(|| Frame {
            world: g.parent * layout.matrix,
        })
    }

    /// The selected source item, if the mask editor may work on it.
    fn mask_target(&self) -> Option<(SceneId, SceneItem)> {
        self.selected().filter(|(_, item)| !item.is_group())
    }

    /// Applies `change` to the selected item's mask (an empty mask is removed).
    fn change_mask(&mut self, change: impl FnOnce(&mut ItemMask)) {
        let Some((_, item)) = self.mask_target() else {
            return;
        };
        let mut mask = item.mask.clone().unwrap_or_default();
        change(&mut mask);
        let next = (mask != ItemMask::default()).then_some(mask);
        if next != item.mask {
            self.update_selected(ItemPatch {
                mask: Some(next),
                ..Default::default()
            });
        }
    }

    /// One whole mask edit as its own undo step (presets, styles, list buttons).
    fn change_mask_once(&mut self, change: impl FnOnce(&mut ItemMask)) {
        self.commit();
        self.change_mask(change);
        self.commit();
    }

    /// `mask.*` actions from the inspector, the preview toolbar and the palette.
    pub(super) fn mask_action(&mut self, action: &str) {
        let (name, index) = match action.split_once(':') {
            Some((name, i)) => (name, i.parse::<usize>().ok()),
            None => (action, None),
        };
        match name {
            "edit" => {
                let Some((_, item)) = self.mask_target() else {
                    self.toast("Select a source to edit its mask".into(), false);
                    return;
                };
                self.mask_edit = Some(Editor::new(item.id));
            }
            "done" => self.mask_edit = None,
            "toggle-edit" => {
                if self.mask_edit.take().is_none() {
                    return self.mask_action("edit");
                }
            }
            "keep" | "cut" => {
                if let Some(e) = &mut self.mask_edit {
                    e.cut = name == "cut";
                }
            }
            "preset.circle" | "preset.rounded" | "preset.corner" | "preset.diagonal" => {
                let aspect = self
                    .mask_target()
                    .and_then(|(_, item)| self.mask_frame(&item))
                    .map_or(1.0, |f| f.size().x / f.size().y);
                let preset = match name {
                    "preset.circle" => ItemMask::circle(aspect),
                    "preset.rounded" => ItemMask::rounded(ROUNDED_RADIUS),
                    "preset.corner" => ItemMask::cut_corner(),
                    _ => ItemMask::diagonal_split(),
                };
                self.change_mask_once(|m| {
                    m.shapes = preset.shapes;
                    m.enabled = true;
                    m.invert = false;
                });
                if let Some(e) = &mut self.mask_edit {
                    e.selected = None;
                    e.point = None;
                }
            }
            "clear" => {
                self.change_mask_once(|m| {
                    m.shapes.clear();
                    m.invert = false;
                });
                if let Some(e) = &mut self.mask_edit {
                    e.selected = None;
                }
            }
            "style.shadow" | "style.neon" | "style.border" | "style.none" => {
                let effects = match name {
                    "style.shadow" => MaskEffects::soft_shadow(),
                    "style.neon" => MaskEffects::neon_glow(),
                    "style.border" => MaskEffects::thin_border(),
                    _ => MaskEffects::default(),
                };
                self.change_mask_once(|m| m.effects = effects);
            }
            "shape.select" => {
                if self.mask_edit.is_none() {
                    self.mask_action("edit");
                }
                if let Some(e) = &mut self.mask_edit {
                    e.selected = index;
                    e.point = None;
                }
            }
            "shape.up" | "shape.down" | "shape.remove" => {
                let Some(i) = index else { return };
                let mut selected = self.mask_edit.as_ref().and_then(|e| e.selected);
                self.change_mask_once(|m| {
                    if i >= m.shapes.len() {
                        return;
                    }
                    match name {
                        // Up the list is later in the order (drawn over the shapes before it).
                        "shape.up" if i + 1 < m.shapes.len() => {
                            m.shapes.swap(i, i + 1);
                            selected = Some(i + 1);
                        }
                        "shape.down" if i > 0 => {
                            m.shapes.swap(i, i - 1);
                            selected = Some(i - 1);
                        }
                        "shape.remove" => {
                            m.shapes.remove(i);
                            selected = None;
                        }
                        _ => {}
                    }
                });
                if let Some(e) = &mut self.mask_edit {
                    e.selected = selected;
                    e.point = None;
                }
            }
            _ => {
                if let Some(tool) = name.strip_prefix("tool.").and_then(Tool::from_name) {
                    if self.mask_edit.is_none() {
                        self.mask_action("edit");
                    }
                    if let Some(e) = &mut self.mask_edit {
                        e.set_tool(tool);
                    }
                }
            }
        }
        self.refresh();
    }

    /// Inspector sliders: `mask-feather:<i>`, `mask-radius:<i>`, and the effect values.
    pub(super) fn mask_edit_field(&mut self, field: &str, value: f32) {
        let (name, index) = match field.split_once(':') {
            Some((name, i)) => (name, i.parse::<usize>().ok()),
            None => (field, None),
        };
        let length = value.clamp(0.0, estream_project::mask::MAX_LENGTH);
        let unit = value.clamp(0.0, 1.0);
        self.change_mask(|m| {
            let e = &mut m.effects;
            match name {
                "feather" => {
                    if let Some(s) = index.and_then(|i| m.shapes.get_mut(i)) {
                        s.feather = length;
                    }
                }
                "radius" => {
                    if let Some(MaskGeometry::Rect { radius, .. }) = index
                        .and_then(|i| m.shapes.get_mut(i))
                        .map(|s| &mut s.geometry)
                    {
                        *radius = length;
                    }
                }
                "border-width" => e.border.iter_mut().for_each(|b| b.width = length),
                "glow-size" => e.glow.iter_mut().for_each(|g| g.size = length),
                "glow-strength" => e.glow.iter_mut().for_each(|g| g.strength = unit),
                "shadow-x" | "shadow-y" => {
                    let v = value.clamp(
                        -estream_project::mask::MAX_LENGTH,
                        estream_project::mask::MAX_LENGTH,
                    );
                    for s in e.shadow.iter_mut() {
                        if name == "shadow-x" {
                            s.offset.x = v;
                        } else {
                            s.offset.y = v;
                        }
                    }
                }
                "shadow-softness" => e.shadow.iter_mut().for_each(|s| s.softness = length),
                "shadow-opacity" => e.shadow.iter_mut().for_each(|s| s.opacity = unit),
                _ => {}
            }
        });
    }

    /// Inspector choices: `mask-mode:<i>` (Keep, Cut), `mask-border-place`, `mask-glow-place`.
    pub(super) fn mask_option(&mut self, field: &str, index: i32) {
        let index = index.max(0) as usize;
        let (name, shape) = match field.split_once(':') {
            Some((name, i)) => (name, i.parse::<usize>().ok()),
            None => (field, None),
        };
        self.change_mask_once(|m| match name {
            "mode" => {
                if let Some(s) = shape.and_then(|i| m.shapes.get_mut(i)) {
                    s.mode = if index == 1 {
                        MaskMode::Cut
                    } else {
                        MaskMode::Keep
                    };
                }
            }
            "border-place" => {
                if let (Some(b), Some(p)) = (&mut m.effects.border, BORDER_PLACES.get(index)) {
                    b.placement = *p;
                }
            }
            "glow-place" => {
                if let (Some(g), Some(p)) = (&mut m.effects.glow, GLOW_PLACES.get(index)) {
                    g.placement = *p;
                }
            }
            _ => {}
        });
    }

    /// Inspector switches: `mask-on`, `mask-invert`, `mask-border`, `mask-glow`, `mask-shadow`.
    pub(super) fn mask_toggle(&mut self, field: &str, on: bool) {
        self.change_mask_once(|m| {
            let e = &mut m.effects;
            match field {
                "on" => m.enabled = on,
                "invert" => m.invert = on,
                "border" => e.border = on.then(|| MaskEffects::thin_border().border).flatten(),
                "glow" => {
                    e.glow = on.then(|| MaskEffects::neon_glow().glow).flatten();
                }
                "shadow" => e.shadow = on.then(|| MaskEffects::soft_shadow().shadow).flatten(),
                _ => {}
            }
        });
    }

    /// The color of an effect (`border`, `glow` or `shadow`) from the picker.
    pub(super) fn mask_color(&mut self, which: &str, color: slint::Color) {
        let c = from_color(color);
        self.change_mask(|m| {
            let e = &mut m.effects;
            match which {
                "border" => e.border.iter_mut().for_each(|b| b.color = c),
                "glow" => e.glow.iter_mut().for_each(|g| g.color = c),
                "shadow" => e.shadow.iter_mut().for_each(|s| s.color = c),
                _ => {}
            }
        });
    }

    /// The Mask and Edge effects panels for the selected item.
    pub(super) fn mask_inspect(&self, item: &SceneItem) -> MaskInspect {
        if item.is_group() {
            return MaskInspect::default();
        }
        let mask = item.mask.clone().unwrap_or_default();
        let selected = self
            .mask_edit
            .as_ref()
            .filter(|e| e.item == item.id)
            .and_then(|e| e.selected);
        let mut counts = [0usize; 3];
        let shapes: Vec<MaskShapeRow> = mask
            .shapes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (kind, radius) = match s.geometry {
                    MaskGeometry::Rect { radius, .. } => (0, Some(radius)),
                    MaskGeometry::Ellipse { .. } => (1, None),
                    MaskGeometry::Polygon { .. } => (2, None),
                };
                counts[kind] += 1;
                let name = ["Rectangle", "Ellipse", "Polygon"][kind];
                MaskShapeRow {
                    name: format!("{name} {}", counts[kind]).into(),
                    cut: s.mode == MaskMode::Cut,
                    feather: s.feather,
                    has_radius: radius.is_some(),
                    radius: radius.unwrap_or_default(),
                    selected: selected == Some(i),
                }
            })
            .rev() // the list shows the last shape (drawn over the rest) first
            .collect();
        let e = &mask.effects;
        let border = e
            .border
            .unwrap_or(MaskEffects::thin_border().border.unwrap_or(Border {
                color: Rgba8::WHITE,
                width: 3.0,
                placement: BorderPlacement::Outside,
            }));
        let glow = e.glow.or(MaskEffects::neon_glow().glow).unwrap_or(Glow {
            color: Rgba8::WHITE,
            size: 24.0,
            strength: 1.0,
            placement: GlowPlacement::Outside,
        });
        let shadow = e
            .shadow
            .or(MaskEffects::soft_shadow().shadow)
            .unwrap_or(Shadow {
                color: Rgba8::BLACK,
                offset: Vec2::ZERO,
                softness: 0.0,
                opacity: 1.0,
            });
        MaskInspect {
            can: true,
            on: mask.enabled,
            invert: mask.invert,
            editing: self.mask_edit.as_ref().is_some_and(|e| e.item == item.id),
            shapes: ModelRc::new(VecModel::from(shapes)),
            border: e.border.is_some(),
            border_color: to_color(border.color),
            border_width: border.width,
            border_place: BORDER_PLACES
                .iter()
                .position(|p| *p == border.placement)
                .unwrap_or(2) as i32,
            glow: e.glow.is_some(),
            glow_color: to_color(glow.color),
            glow_size: glow.size,
            glow_strength: glow.strength,
            glow_place: GLOW_PLACES
                .iter()
                .position(|p| *p == glow.placement)
                .unwrap_or(0) as i32,
            shadow: e.shadow.is_some(),
            shadow_color: to_color(shadow.color),
            shadow_x: shadow.offset.x,
            shadow_y: shadow.offset.y,
            shadow_softness: shadow.softness,
            shadow_opacity: shadow.opacity,
        }
    }

    /// Sends the pointer to the mask editor while it is open. Returns true if it took it.
    pub(super) fn mask_pointer(&mut self, kind: &str, p: Vec2, px: f32, mods: Modifiers) -> bool {
        let Some(mut editor) = self.mask_edit.take() else {
            return false;
        };
        let target = self
            .mask_target()
            .filter(|(_, item)| item.id == editor.item)
            .and_then(|(_, item)| Some((self.mask_frame(&item)?, item)));
        let Some((frame, item)) = target else {
            // The item went away or isn't drawn: leave the editor.
            self.refresh();
            return false;
        };
        let mask = item.mask.clone().unwrap_or_default();
        let mods = Mods {
            shift: mods.shift,
            alt: mods.alt,
        };
        let next = match kind {
            "down" => editor.down(&mask, p, &frame, px, mods),
            "move" => editor.moved(&mask, p, &frame, px, mods),
            "up" => editor.up(&mask, p, &frame, px, mods),
            "double" => editor.double(&mask, p, &frame, px),
            _ => None,
        };
        let cursor = match editor.tool {
            Tool::Select if editor.busy() => "move",
            Tool::Select => "default",
            _ => "crosshair",
        };
        self.mask_edit = Some(editor);
        if let Some(w) = self.window() {
            w.set_cursor(cursor.into());
        }
        if let Some(next) = next {
            let next = (next != ItemMask::default()).then_some(next);
            self.update_selected(ItemPatch {
                mask: Some(next),
                ..Default::default()
            });
        } else {
            self.refresh_mask_view();
        }
        if matches!(kind, "up" | "double") {
            self.commit();
        }
        true
    }

    /// Keys while the mask editor is open. Returns true if used.
    pub(super) fn mask_key(&mut self, text: &str, ctrl: bool, alt: bool) -> bool {
        if self.mask_edit.is_none() || ctrl || alt {
            return false;
        }
        let mask = self
            .mask_target()
            .and_then(|(_, item)| item.mask)
            .unwrap_or_default();
        let Some(editor) = &mut self.mask_edit else {
            return false;
        };
        let next = match key_from_text(text) {
            Some(Key::Escape) => {
                if !editor.cancel() {
                    self.mask_edit = None;
                }
                None
            }
            Some(Key::Delete) => editor.delete(&mask),
            Some(Key::Char('\n' | '\r')) => editor.enter(&mask),
            Some(Key::Char('x')) => {
                editor.cut = !editor.cut;
                None
            }
            Some(Key::Char(c)) => match Tool::from_key(&c.to_string()) {
                Some(tool) => {
                    editor.set_tool(tool);
                    None
                }
                None => return false,
            },
            _ => return false,
        };
        if let Some(next) = next {
            self.commit();
            let next = (next != ItemMask::default()).then_some(next);
            self.update_selected(ItemPatch {
                mask: Some(next),
                ..Default::default()
            });
            self.commit();
        } else {
            self.refresh();
        }
        true
    }

    /// Updates the editor overlay only (pointer moves that change nothing in the project).
    fn refresh_mask_view(&mut self) {
        let view = self.mask_view();
        if let Some(w) = self.window() {
            w.set_mask_edit(view);
        }
    }

    /// The editor overlay for the selected item; closes the editor when its item is gone.
    pub(super) fn mask_view(&mut self) -> MaskEditView {
        let target = self.mask_target();
        let Some(editor) = &self.mask_edit else {
            return MaskEditView::default();
        };
        let Some((frame, item)) = target
            .filter(|(_, item)| item.id == editor.item)
            .and_then(|(_, item)| Some((self.mask_frame(&item)?, item)))
        else {
            self.mask_edit = None;
            return MaskEditView::default();
        };
        let mask = item.mask.clone().unwrap_or_default();
        let size = frame.size();
        let outlines: Vec<MaskOutline> = mask
            .shapes
            .iter()
            .enumerate()
            .map(|(i, s)| MaskOutline {
                commands: outline(&s.geometry, size).into(),
                selected: editor.selected == Some(i),
                cut: s.mode == MaskMode::Cut,
            })
            .collect();
        let draft = match (&editor.draft, editor.tool) {
            (Some(shape), _) => outline(&shape.geometry, size),
            (None, Tool::Polygon) if !editor.drawing.is_empty() => {
                let mut points = editor.drawing.clone();
                points.extend(editor.pointer);
                polyline(&points, false)
            }
            (None, Tool::Freehand) => polyline(&editor.drawing, false),
            _ => String::new(),
        };
        let handles: Vec<MaskHandle> = editor
            .handles(&mask)
            .iter()
            .enumerate()
            .map(|(k, u)| MaskHandle {
                u: u.x,
                v: u.y,
                active: editor.point == Some(k),
            })
            .collect();
        let hint = match editor.tool {
            Tool::Select if mask.shapes.is_empty() => {
                "Pick a shape tool and draw on the picture, or choose a preset"
            }
            Tool::Select => "Drag a shape to move it, its handles to resize. Delete removes it",
            Tool::Rect | Tool::Ellipse => {
                "Drag to draw. Shift: square or circle, Alt: from the center"
            }
            Tool::Polygon if editor.drawing.is_empty() => "Click to place points",
            Tool::Polygon => "Click the first point, double-click or press Enter to close",
            Tool::Freehand => "Drag to draw an outline",
        };
        let (tool, cut) = (editor.tool.index(), editor.cut);
        let (center, rotation) = {
            let x = frame.world.matrix2.x_axis;
            (
                frame.to_canvas(Vec2::splat(0.5)),
                x.y.atan2(x.x).to_degrees(),
            )
        };
        MaskEditView {
            visible: true,
            cx: center.x,
            cy: center.y,
            w: size.x,
            h: size.y,
            rotation,
            dim: self.mask_dim(&mask, size),
            outlines: ModelRc::new(VecModel::from(outlines)),
            draft: draft.into(),
            handles: ModelRc::new(VecModel::from(handles)),
            tool,
            cut,
            hint: hint.into(),
        }
    }

    /// A small picture that darkens what the mask hides (empty when nothing is cut).
    fn mask_dim(&mut self, mask: &ItemMask, size: Vec2) -> slint::Image {
        let key = [size.x.round() as u32, size.y.round() as u32];
        if let Some(d) = &self.mask_dim
            && d.mask == *mask
            && d.size == key
        {
            return d.image.clone();
        }
        let shapes_only = ItemMask {
            effects: MaskEffects::default(),
            ..mask.clone()
        };
        let image = estream_engine::mask::bake_with(&shapes_only, size, DIM_SIDE)
            .mask
            .map(|m| {
                let mut pixels =
                    slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(m.width, m.height);
                for (px, cover) in pixels.make_mut_slice().iter_mut().zip(&m.pixels) {
                    let a = ((255 - cover) as f32 * 0.6) as u8;
                    *px = slint::Rgba8Pixel::new(0, 0, 0, a);
                }
                slint::Image::from_rgba8(pixels)
            })
            .unwrap_or_default();
        self.mask_dim = Some(MaskDim {
            mask: mask.clone(),
            size: key,
            image: image.clone(),
        });
        image
    }
}
