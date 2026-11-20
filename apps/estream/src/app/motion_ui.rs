//! The inspector's Transitions panel: an item's show and hide animations, and whether it moves
//! to its place in the next scene. Every change is an item command.

use super::*;
use estream_project::animation::{MAX_ANIMATION_MS, MIN_ANIMATION_MS};
use estream_project::{AnimationKind, Direction, Ease, ItemAnimation, ItemAnimations};

/// The inspector's index of an animation: 0 = none, then [`AnimationKind::ALL`] in order.
fn kind_index(a: Option<&ItemAnimation>) -> i32 {
    a.map_or(0, |a| {
        1 + AnimationKind::ALL
            .iter()
            .position(|k| k.label() == a.kind.label())
            .unwrap_or(0) as i32
    })
}

fn direction_index(a: Option<&ItemAnimation>) -> i32 {
    a.and_then(|a| a.kind.direction())
        .and_then(|d| Direction::ALL.iter().position(|x| *x == d))
        .unwrap_or(0) as i32
}

fn ease_index(a: Option<&ItemAnimation>) -> i32 {
    a.and_then(|a| Ease::ALL.iter().position(|e| *e == a.ease))
        .unwrap_or(0) as i32
}

/// The Transitions panel's fields for an item's animations.
pub(super) fn animation_fields(inspect: &mut Inspect, a: &ItemAnimations) {
    let (show, hide) = (a.show.as_ref(), a.hide.as_ref());
    let ms = |a: Option<&ItemAnimation>| a.map_or(400.0, |a| a.duration_ms as f32);
    inspect.anim_show = kind_index(show);
    inspect.anim_show_dir = direction_index(show);
    inspect.anim_show_ms = ms(show);
    inspect.anim_show_ease = ease_index(show);
    inspect.anim_hide = kind_index(hide);
    inspect.anim_hide_dir = direction_index(hide);
    inspect.anim_hide_ms = ms(hide);
    inspect.anim_hide_ease = ease_index(hide);
    inspect.anim_moves = a.moves;
}

/// `animations` with `field` ("show.kind", "hide.dir", "show.ms", "hide.ease" or "moves") set
/// from `value` (an index, a duration in ms, or 0/1).
fn changed(mut animations: ItemAnimations, field: &str, value: f32) -> ItemAnimations {
    if field == "moves" {
        animations.moves = value > 0.5;
        return animations;
    }
    let Some((which, what)) = field.split_once('.') else {
        return animations;
    };
    let slot = match which {
        "show" => &mut animations.show,
        "hide" => &mut animations.hide,
        _ => return animations,
    };
    let i = value.round().max(0.0) as usize;
    match what {
        "kind" => {
            *slot = match i.checked_sub(1).and_then(|k| AnimationKind::ALL.get(k)) {
                None => None,
                Some(kind) => {
                    let old = slot.unwrap_or_default();
                    // Keep the chosen direction when switching between Slide and Wipe.
                    let kind = match old.kind.direction() {
                        Some(d) => kind.with_direction(d),
                        None => *kind,
                    };
                    Some(ItemAnimation { kind, ..old })
                }
            }
        }
        "dir" => {
            if let (Some(a), Some(d)) = (slot.as_mut(), Direction::ALL.get(i)) {
                a.kind = a.kind.with_direction(*d);
            }
        }
        "ms" => {
            if let Some(a) = slot.as_mut() {
                a.duration_ms = (value.round() as u32).clamp(MIN_ANIMATION_MS, MAX_ANIMATION_MS);
            }
        }
        "ease" => {
            if let (Some(a), Some(e)) = (slot.as_mut(), Ease::ALL.get(i)) {
                a.ease = *e;
            }
        }
        _ => {}
    }
    animations
}

impl App {
    /// Changes one field of the selected item's animations (see [`changed`]).
    pub(super) fn animation_set(&mut self, field: &str, value: f32) {
        let Some((_, item)) = self.selected() else {
            return;
        };
        if item.is_group() {
            return;
        }
        let animations = changed(item.animations, field, value);
        if animations != item.animations {
            self.update_selected(ItemPatch {
                animations: Some(animations),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_change_one_thing_each() {
        let none = ItemAnimations::default();
        let slide = changed(none, "show.kind", 4.0);
        assert_eq!(
            slide.show.map(|a| a.kind),
            Some(AnimationKind::Slide {
                edge: Direction::Left
            })
        );
        let right = changed(slide, "show.dir", 1.0);
        let wipe = changed(right, "show.kind", 5.0);
        assert_eq!(
            wipe.show.map(|a| a.kind),
            Some(AnimationKind::Wipe {
                direction: Direction::Right
            }),
            "the direction carries over"
        );
        let long = changed(wipe, "show.ms", 99_999.0);
        assert_eq!(long.show.map(|a| a.duration_ms), Some(MAX_ANIMATION_MS));
        assert_eq!(changed(long, "show.kind", 0.0).show, None);
        assert_eq!(
            changed(none, "hide.ms", 300.0),
            none,
            "no hide animation to time"
        );
        assert!(!changed(none, "moves", 0.0).moves);

        let mut inspect = Inspect::default();
        animation_fields(&mut inspect, &long);
        assert_eq!((inspect.anim_show, inspect.anim_show_dir), (5, 1));
        assert_eq!(inspect.anim_hide, 0);
        assert!(inspect.anim_moves);
    }
}
