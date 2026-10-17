//! Scene transitions (saved with the scene collection).

use estream_core::Rgba8;
use serde::{Deserialize, Serialize};

/// How the program changes from one scene to the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Fade,
            duration_ms: 300,
        }
    }
}

impl Transition {
    pub const CUT: Transition = Transition {
        kind: TransitionKind::Cut,
        duration_ms: 0,
    };

    /// True if this changes scenes instantly.
    pub fn is_instant(&self) -> bool {
        matches!(self.kind, TransitionKind::Cut) || self.duration_ms == 0
    }
}

/// Direction of movement for Slide, Swipe and Wipe: the way the new scene travels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];

    /// Unit vector of travel in canvas space (y down).
    pub fn vector(self) -> [f32; 2] {
        match self {
            Direction::Left => [-1.0, 0.0],
            Direction::Right => [1.0, 0.0],
            Direction::Up => [0.0, -1.0],
            Direction::Down => [0.0, 1.0],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransitionKind {
    /// Instant switch.
    Cut,
    /// Crossfade.
    #[default]
    Fade,
    /// Fade out to a color, then in to the new scene.
    FadeToColor { color: Rgba8 },
    /// Both scenes move: the new one pushes the old one out.
    Slide { direction: Direction },
    /// The new scene moves in over the old one.
    Swipe { direction: Direction },
    /// A moving edge reveals the new scene.
    Wipe { direction: Direction, softness: f32 },
    /// A user WGSL function `transition(uv, progress) -> vec4<f32>`.
    Shader { code: String },
}

impl TransitionKind {
    pub fn label(&self) -> &'static str {
        match self {
            TransitionKind::Cut => "Cut",
            TransitionKind::Fade => "Fade",
            TransitionKind::FadeToColor { .. } => "Fade to color",
            TransitionKind::Slide { .. } => "Slide",
            TransitionKind::Swipe { .. } => "Swipe",
            TransitionKind::Wipe { .. } => "Wipe",
            TransitionKind::Shader { .. } => "Custom shader",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_and_defaults() {
        let t = Transition {
            kind: TransitionKind::Wipe {
                direction: Direction::Up,
                softness: 0.2,
            },
            duration_ms: 700,
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"type\":\"wipe\""), "{json}");
        assert_eq!(serde_json::from_str::<Transition>(&json).unwrap(), t);
        let d: Transition = serde_json::from_str("{}").unwrap();
        assert_eq!(d, Transition::default());
        assert_eq!(d.kind, TransitionKind::Fade);
        assert_eq!(d.duration_ms, 300);
        assert!(Transition::CUT.is_instant());
        assert!(
            Transition {
                duration_ms: 0,
                ..Default::default()
            }
            .is_instant()
        );
    }
}
