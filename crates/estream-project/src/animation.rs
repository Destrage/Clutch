//! Item transitions: how an item that is in both scenes moves to its new place during a scene
//! transition, and how an item animates when it is shown or hidden.

use crate::transition::Direction;
use serde::{Deserialize, Serialize};

/// Shortest and longest show or hide animation.
pub const MIN_ANIMATION_MS: u32 = 50;
pub const MAX_ANIMATION_MS: u32 = 5000;

/// How progress speeds up and slows down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    /// Starts and ends slowly (cubic ease in and out).
    #[default]
    Smooth,
    /// Fast start, a small overshoot, then settles.
    Spring,
    Linear,
}

impl Ease {
    pub const ALL: [Ease; 3] = [Ease::Smooth, Ease::Spring, Ease::Linear];

    pub fn label(self) -> &'static str {
        match self {
            Ease::Smooth => "Smooth",
            Ease::Spring => "Spring",
            Ease::Linear => "Linear",
        }
    }

    /// Eased progress for linear progress `t` (clamped to 0..1). 0 maps to 0 and 1 to 1;
    /// Spring goes past 1 on the way.
    pub fn apply(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Ease::Linear => t,
            Ease::Smooth => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
                }
            }
            Ease::Spring => {
                let spring = |t: f32| 1.0 - (-6.0 * t).exp() * (9.0 * t).cos();
                spring(t) / spring(1.0)
            }
        }
    }
}

/// "Move matching items" on a scene transition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoveSettings {
    pub ease: Ease,
    /// Travel on an arc instead of a straight line.
    pub curved: bool,
}

/// What a show or hide animation does. Hiding plays it backwards (the item ends gone).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnimationKind {
    #[default]
    Fade,
    /// Grows from (or shrinks to) its center.
    Zoom,
    /// Zoom with a small overshoot.
    Pop,
    /// Comes in from (or leaves to) a canvas edge.
    Slide { edge: Direction },
    /// A moving edge reveals (or hides) the item, travelling in `direction`.
    Wipe { direction: Direction },
}

impl AnimationKind {
    /// One of each kind, with default directions (the order pickers show).
    pub const ALL: [AnimationKind; 5] = [
        AnimationKind::Fade,
        AnimationKind::Zoom,
        AnimationKind::Pop,
        AnimationKind::Slide {
            edge: Direction::Left,
        },
        AnimationKind::Wipe {
            direction: Direction::Right,
        },
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AnimationKind::Fade => "Fade",
            AnimationKind::Zoom => "Zoom",
            AnimationKind::Pop => "Pop",
            AnimationKind::Slide { .. } => "Slide",
            AnimationKind::Wipe { .. } => "Wipe",
        }
    }

    pub fn direction(&self) -> Option<Direction> {
        match self {
            AnimationKind::Slide { edge } => Some(*edge),
            AnimationKind::Wipe { direction } => Some(*direction),
            _ => None,
        }
    }

    /// The same kind with another direction (kinds without one are returned unchanged).
    pub fn with_direction(self, d: Direction) -> Self {
        match self {
            AnimationKind::Slide { .. } => AnimationKind::Slide { edge: d },
            AnimationKind::Wipe { .. } => AnimationKind::Wipe { direction: d },
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemAnimation {
    pub kind: AnimationKind,
    pub duration_ms: u32,
    pub ease: Ease,
}

impl Default for ItemAnimation {
    fn default() -> Self {
        Self {
            kind: AnimationKind::Fade,
            duration_ms: 400,
            ease: Ease::Smooth,
        }
    }
}

impl ItemAnimation {
    pub fn duration_ns(&self) -> u64 {
        u64::from(self.duration_ms) * 1_000_000
    }
}

/// An item's show and hide animations, and whether it moves between scenes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemAnimations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show: Option<ItemAnimation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide: Option<ItemAnimation>,
    /// "Move between scenes": off makes the item take part in the scene transition.
    pub moves: bool,
}

impl Default for ItemAnimations {
    fn default() -> Self {
        Self {
            show: None,
            hide: None,
            moves: true,
        }
    }
}

impl ItemAnimations {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The animation for becoming visible (`true`) or hidden (`false`).
    pub fn for_visibility(&self, visible: bool) -> Option<&ItemAnimation> {
        if visible {
            self.show.as_ref()
        } else {
            self.hide.as_ref()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (what, a) in [("show", &self.show), ("hide", &self.hide)] {
            if let Some(a) = a
                && !(MIN_ANIMATION_MS..=MAX_ANIMATION_MS).contains(&a.duration_ms)
            {
                return Err(format!(
                    "{what} animation duration {} ms is outside {MIN_ANIMATION_MS}..={MAX_ANIMATION_MS}",
                    a.duration_ms
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn eases_start_at_zero_and_end_at_one() {
        for ease in Ease::ALL {
            assert!(ease.apply(0.0).abs() < 1e-6, "{ease:?}");
            assert!((ease.apply(1.0) - 1.0).abs() < 1e-6, "{ease:?}");
        }
        assert!(
            (Ease::Smooth.apply(0.5) - 0.5).abs() < 1e-6,
            "smooth is symmetric"
        );
        assert!(Ease::Smooth.apply(0.1) < 0.1, "smooth starts slowly");
        assert!(Ease::Smooth.apply(0.9) > 0.9, "and ends slowly");
        let peak = (0..=100)
            .map(|i| Ease::Spring.apply(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(
            peak > 1.05 && peak < 1.2,
            "spring overshoots a little: {peak}"
        );
        assert_eq!(Ease::Linear.apply(2.0), 1.0, "clamped");
        assert_eq!(Ease::Linear.apply(f32::NAN), 0.0);
    }

    #[test]
    fn serde_defaults_and_round_trip() {
        let d: ItemAnimations = serde_json::from_str("{}").unwrap();
        assert!(d.is_default());
        assert!(d.moves, "items move between scenes unless turned off");
        let a = ItemAnimations {
            show: Some(ItemAnimation {
                kind: AnimationKind::Slide {
                    edge: Direction::Up,
                },
                duration_ms: 250,
                ease: Ease::Spring,
            }),
            hide: Some(ItemAnimation::default()),
            moves: false,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(
            json.contains(r#""kind":{"type":"slide","edge":"up"}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<ItemAnimations>(&json).unwrap(), a);
        let partial: ItemAnimation = serde_json::from_str(r#"{"kind":{"type":"pop"}}"#).unwrap();
        assert_eq!(partial.kind, AnimationKind::Pop);
        assert_eq!(partial.duration_ms, 400);
    }

    #[test]
    fn items_save_animations_only_when_set() {
        let mut item = crate::SceneItem::source(estream_core::SourceId::new());
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("animations"), "{json}");
        item.animations.moves = false;
        item.animations.show = Some(ItemAnimation::default());
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(
            serde_json::from_str::<crate::SceneItem>(&json).unwrap(),
            item
        );
    }

    #[test]
    fn validation_bounds_durations() {
        let mut a = ItemAnimations {
            hide: Some(ItemAnimation {
                duration_ms: 10,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(a.validate().is_err());
        a.hide = Some(ItemAnimation::default());
        assert!(a.validate().is_ok());
    }
}
