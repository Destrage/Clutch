//! What made a clip.

use estream_core::Timestamp;
use serde::{Deserialize, Serialize};

/// A moment worth clipping.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// The clip hotkey (or a macro step).
    Manual,
    Kill,
    /// An enemy downed but not yet eliminated (battle royales).
    Knock,
    /// `n` kills in quick succession (as the game counts them).
    MultiKill {
        n: u8,
    },
    /// The whole enemy team taken out by the player.
    Ace,
    Death,
    RoundWin,
    MatchWin,
    /// A goal scored by the player.
    Goal,
    /// A game objective, e.g. `"dragon"` or `"baron"`.
    Objective {
        name: String,
    },
    /// A loudness spike in game audio or the mic.
    Hype,
}

impl EventKind {
    /// A short name for file names and the UI.
    pub fn label(&self) -> String {
        match self {
            EventKind::Manual => "Clip".into(),
            EventKind::Kill => "Kill".into(),
            EventKind::Knock => "Knock".into(),
            EventKind::MultiKill { n } => match n {
                2 => "Double kill".into(),
                3 => "Triple kill".into(),
                4 => "Quadra kill".into(),
                5 => "Penta kill".into(),
                n => format!("{n} kills"),
            },
            EventKind::Ace => "Ace".into(),
            EventKind::Death => "Death".into(),
            EventKind::RoundWin => "Round win".into(),
            EventKind::MatchWin => "Victory".into(),
            EventKind::Goal => "Goal".into(),
            EventKind::Objective { name } => {
                let mut c = name.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
            EventKind::Hype => "Hype".into(),
        }
    }

    /// Seconds before and after the event a highlight of it covers (the auto-clip defaults).
    pub fn window(&self) -> (f64, f64) {
        match self {
            EventKind::Manual => (30.0, 0.0),
            EventKind::Kill | EventKind::Knock | EventKind::Death => (10.0, 3.0),
            EventKind::MultiKill { .. } | EventKind::Ace => (15.0, 4.0),
            EventKind::RoundWin | EventKind::MatchWin => (15.0, 5.0),
            EventKind::Goal => (12.0, 4.0),
            EventKind::Objective { .. } => (12.0, 3.0),
            EventKind::Hype => (12.0, 4.0),
        }
    }

    /// How highlight-worthy the event is, 0..=1 (used to pick a clip's name and by montages).
    pub fn weight(&self) -> f32 {
        match self {
            EventKind::Ace => 1.0,
            EventKind::MultiKill { n } => (0.7 + 0.1 * *n as f32).min(1.0),
            EventKind::MatchWin => 0.9,
            EventKind::Objective { .. } | EventKind::RoundWin | EventKind::Goal => 0.8,
            EventKind::Manual => 0.75,
            EventKind::Kill => 0.6,
            EventKind::Knock => 0.55,
            EventKind::Hype => 0.5,
            EventKind::Death => 0.2,
        }
    }
}

/// An event at an engine-clock time, from a game (or none for manual and hype clips).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipEvent {
    pub kind: EventKind,
    pub at: Timestamp,
    pub game: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(EventKind::MultiKill { n: 2 }.label(), "Double kill");
        assert_eq!(EventKind::MultiKill { n: 7 }.label(), "7 kills");
        assert_eq!(
            EventKind::Objective {
                name: "baron".into()
            }
            .label(),
            "Baron"
        );
    }

    #[test]
    fn serializes_with_a_kind_tag() {
        let json = serde_json::to_string(&EventKind::MultiKill { n: 3 }).unwrap_or_default();
        assert_eq!(json, r#"{"kind":"multi_kill","n":3}"#);
    }
}
