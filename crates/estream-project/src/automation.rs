//! Automation: macros (steps) started by triggers (hotkeys, window focus, timers, events).
//! Saved with the scene collection because macros refer to its scenes and sources.

use estream_core::{MacroId, SceneId, SourceId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Automation {
    pub macros: Vec<Macro>,
}

impl Automation {
    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    pub fn find(&self, id: MacroId) -> Option<&Macro> {
        self.macros.iter().find(|m| m.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub id: MacroId,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

fn yes() -> bool {
    true
}

impl Macro {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: MacroId::new(),
            name: name.into(),
            enabled: true,
            triggers: Vec::new(),
            steps: Vec::new(),
        }
    }
}

/// An output a step or trigger refers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    #[default]
    Stream,
    Record,
    ReplayBuffer,
    VirtualCamera,
}

impl OutputKind {
    pub const ALL: [OutputKind; 4] = [
        OutputKind::Stream,
        OutputKind::Record,
        OutputKind::ReplayBuffer,
        OutputKind::VirtualCamera,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputKind::Stream => "Stream",
            OutputKind::Record => "Recording",
            OutputKind::ReplayBuffer => "Replay buffer",
            OutputKind::VirtualCamera => "Virtual camera",
        }
    }
}

/// On, off or flip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Toggle {
    On,
    Off,
    #[default]
    Toggle,
}

impl Toggle {
    pub const ALL: [Toggle; 3] = [Toggle::On, Toggle::Off, Toggle::Toggle];

    /// The new state given the current one.
    pub fn apply(self, current: bool) -> bool {
        match self {
            Toggle::On => true,
            Toggle::Off => false,
            Toggle::Toggle => !current,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// A global hotkey (works while other apps are focused).
    Hotkey { chord: Chord },
    /// A window becomes the foreground window; `pattern` matches its program file name or
    /// title, case-insensitively.
    WindowFocus { pattern: String },
    /// Every `seconds`.
    Interval { seconds: u32 },
    /// A scene goes on program (on its canvas).
    SceneActivated { scene: SceneId },
    /// An output starts (`started`) or stops.
    Output { output: OutputKind, started: bool },
    /// A game reports an event (kill, win…) or hype is detected. `game` limits it to one
    /// game (by name); `None` is any game.
    GameEvent {
        #[serde(default)]
        game: Option<String>,
        event: GameEventKind,
    },
}

/// The kinds of game event a trigger can wait for (see the clip integrations).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameEventKind {
    #[default]
    Kill,
    /// Two or more kills in a row (as the game counts them), including an ace.
    MultiKill,
    Ace,
    Death,
    RoundWin,
    MatchWin,
    Objective,
    /// A loudness spike in game audio or the mic.
    Hype,
}

impl GameEventKind {
    pub const ALL: [GameEventKind; 8] = [
        GameEventKind::Kill,
        GameEventKind::MultiKill,
        GameEventKind::Ace,
        GameEventKind::Death,
        GameEventKind::RoundWin,
        GameEventKind::MatchWin,
        GameEventKind::Objective,
        GameEventKind::Hype,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GameEventKind::Kill => "Kill",
            GameEventKind::MultiKill => "Multi-kill",
            GameEventKind::Ace => "Ace",
            GameEventKind::Death => "Death",
            GameEventKind::RoundWin => "Round win",
            GameEventKind::MatchWin => "Match win",
            GameEventKind::Objective => "Objective",
            GameEventKind::Hype => "Hype",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    /// Puts `scene` on program (its canvas) with the current transition, or cuts.
    SwitchScene {
        scene: SceneId,
        #[serde(default)]
        cut: bool,
    },
    /// Shows/hides the items of `source` in the program scenes.
    SourceVisible {
        source: SourceId,
        mode: Toggle,
    },
    /// Mutes/unmutes `source` (`On` = muted).
    SourceMute {
        source: SourceId,
        mode: Toggle,
    },
    Output {
        output: OutputKind,
        mode: Toggle,
    },
    SaveReplay,
    /// Saves a clip of `before` seconds before now and `after` seconds after (waits for the
    /// after part), through the same path as the clip hotkey.
    SaveClip {
        before: u32,
        after: u32,
    },
    /// Studio mode: send the Preview to Program.
    StudioTransition,
    /// Any command palette action, by id (e.g. `"studio.toggle"`).
    Action {
        id: String,
    },
    Wait {
        ms: u32,
    },
}

/// A key combination, e.g. Ctrl+Shift+F9. `key` is a key name: `A`–`Z`, `0`–`9`, `F1`–`F24`,
/// `Num0`–`Num9`, `Space`, `Enter`, `PageUp`, `MediaPlayPause`, … (see the hotkey module).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: String,
}

impl Chord {
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    /// Parses `"Ctrl+Shift+F9"` (modifier names are case-insensitive).
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Chord::default();
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "win" | "super" | "meta" => chord.win = true,
                "" => return None,
                _ if chord.key.is_empty() => chord.key = part.to_owned(),
                _ => return None,
            }
        }
        (!chord.key.is_empty()).then_some(chord)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mods = [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ];
        for (on, name) in mods {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_through_text() {
        let c = Chord::parse("ctrl+Shift+F9").unwrap();
        assert!(c.ctrl && c.shift && !c.alt && !c.win);
        assert_eq!(c.key, "F9");
        assert_eq!(c.to_string(), "Ctrl+Shift+F9");
        assert_eq!(Chord::parse(&c.to_string()), Some(c));
        assert_eq!(Chord::parse("Ctrl+"), None);
        assert_eq!(Chord::parse("A+B"), None);
    }

    #[test]
    fn macros_serialize_readably() {
        let mut m = Macro::new("BRB");
        m.triggers.push(Trigger::Hotkey {
            chord: Chord::parse("Ctrl+F9").unwrap(),
        });
        m.steps.push(Step::SwitchScene {
            scene: SceneId::new(),
            cut: false,
        });
        m.steps.push(Step::Wait { ms: 500 });
        m.steps.push(Step::Output {
            output: OutputKind::ReplayBuffer,
            mode: Toggle::On,
        });
        let a = Automation { macros: vec![m] };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"type\":\"switch_scene\""), "{json}");
        assert!(json.contains("\"output\":\"replay_buffer\""), "{json}");
        assert_eq!(serde_json::from_str::<Automation>(&json).unwrap(), a);
    }

    #[test]
    fn toggles() {
        assert!(Toggle::On.apply(false));
        assert!(!Toggle::Off.apply(true));
        assert!(Toggle::Toggle.apply(false));
    }
}
