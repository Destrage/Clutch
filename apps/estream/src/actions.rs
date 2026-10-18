//! The action registry: one list of user actions shared by the command palette,
//! keyboard shortcuts and menus, plus fuzzy matching for the palette.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    CommandPalette,
    Undo,
    Redo,
    Save,
    AddScene,
    DuplicateScene,
    RemoveScene,
    RenameScene,
    AddSource,
    AddColorSource,
    AddTestPattern,
    RemoveSelected,
    DuplicateSelected,
    CopySelected,
    PasteItem,
    ToggleVisibility,
    ToggleLock,
    MoveUp,
    MoveDown,
    GroupSelected,
    FitToCanvas,
    StretchToCanvas,
    CenterOnCanvas,
    ResetTransform,
    ZoomFit,
    Zoom100,
    ToggleSnapping,
    ThemeDark,
    ThemeLight,
    ThemeSystem,
    ResetLayout,
    Deselect,
    ToggleRecording,
    ToggleReplay,
    ToggleStream,
    GoLiveSetup,
    ToggleStudio,
    ToggleVirtualCamera,
    TransitionGo,
    TransitionCut,
    TransitionShader,
    SaveReplay,
    RevealOutput,
    OpenSettings,
    OpenAutomation,
    OpenStats,
    OpenClips,
    ToggleClipMode,
    OpenMontages,
}

/// Keyboard shortcut. `key` is the lowercase character, or a named key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    F2,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
}

impl Shortcut {
    const fn ctrl(c: char) -> Self {
        Self {
            ctrl: true,
            shift: false,
            alt: false,
            key: Key::Char(c),
        }
    }
    const fn ctrl_shift(c: char) -> Self {
        Self {
            ctrl: true,
            shift: true,
            alt: false,
            key: Key::Char(c),
        }
    }
    const fn key(key: Key) -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        }
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        parts.push(match self.key {
            Key::Char(c) => c.to_ascii_uppercase().to_string(),
            Key::F2 => "F2".into(),
            Key::Delete => "Del".into(),
            Key::Escape => "Esc".into(),
            Key::Up => "↑".into(),
            Key::Down => "↓".into(),
            Key::Left => "←".into(),
            Key::Right => "→".into(),
        });
        parts.join("+")
    }
}

pub struct ActionInfo {
    pub action: Action,
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub shortcut: Option<Shortcut>,
    /// Shown in the palette (some actions only make sense via keys/menus).
    pub in_palette: bool,
}

macro_rules! actions {
    ($( $action:ident, $id:literal, $title:literal, $cat:literal, $shortcut:expr, $palette:literal; )*) => {
        pub const ACTIONS: &[ActionInfo] = &[
            $( ActionInfo { action: Action::$action, id: $id, title: $title, category: $cat, shortcut: $shortcut, in_palette: $palette }, )*
        ];
    };
}

actions! {
    CommandPalette, "palette", "Command palette", "General", Some(Shortcut::ctrl('k')), false;
    Undo, "undo", "Undo", "Edit", Some(Shortcut::ctrl('z')), true;
    Redo, "redo", "Redo", "Edit", Some(Shortcut::ctrl_shift('z')), true;
    Save, "save", "Save setup", "File", Some(Shortcut::ctrl('s')), true;
    AddScene, "scene.add", "New scene", "Scenes", Some(Shortcut::ctrl_shift('n')), true;
    DuplicateScene, "scene.duplicate", "Duplicate scene", "Scenes", None, true;
    RemoveScene, "scene.remove", "Delete scene", "Scenes", None, true;
    RenameScene, "scene.rename", "Rename scene", "Scenes", Some(Shortcut::key(Key::F2)), true;
    AddSource, "source.add", "Add source…", "Sources", Some(Shortcut::ctrl('n')), true;
    AddColorSource, "source.add.color", "Add color source", "Sources", None, true;
    AddTestPattern, "source.add.test_pattern", "Add test pattern", "Sources", None, true;
    RemoveSelected, "item.remove", "Delete selected item", "Sources", Some(Shortcut::key(Key::Delete)), true;
    DuplicateSelected, "item.duplicate", "Duplicate selected item", "Sources", Some(Shortcut::ctrl('d')), true;
    CopySelected, "item.copy", "Copy item", "Edit", Some(Shortcut::ctrl('c')), true;
    PasteItem, "item.paste", "Paste item", "Edit", Some(Shortcut::ctrl('v')), true;
    ToggleVisibility, "item.visibility", "Show / hide selected item", "Sources", Some(Shortcut::ctrl('h')), true;
    ToggleLock, "item.lock", "Lock / unlock selected item", "Sources", Some(Shortcut::ctrl('l')), true;
    MoveUp, "item.up", "Move item up", "Sources", Some(Shortcut::ctrl(']')), true;
    MoveDown, "item.down", "Move item down", "Sources", Some(Shortcut::ctrl('[')), true;
    GroupSelected, "item.group", "Group selected item", "Sources", Some(Shortcut::ctrl('g')), true;
    FitToCanvas, "transform.fit", "Fit to canvas", "Transform", Some(Shortcut::ctrl('f')), true;
    StretchToCanvas, "transform.stretch", "Stretch to canvas", "Transform", Some(Shortcut::ctrl_shift('s')), true;
    CenterOnCanvas, "transform.center", "Center on canvas", "Transform", Some(Shortcut::ctrl('e')), true;
    ResetTransform, "transform.reset", "Reset transform", "Transform", Some(Shortcut::ctrl('r')), true;
    ZoomFit, "view.fit", "Zoom to fit", "View", Some(Shortcut::ctrl('0')), true;
    Zoom100, "view.100", "Zoom to 100%", "View", Some(Shortcut::ctrl('1')), true;
    ToggleSnapping, "view.snapping", "Toggle snapping", "View", None, true;
    ThemeDark, "theme.dark", "Theme: Dark", "Appearance", None, true;
    ThemeLight, "theme.light", "Theme: Light", "Appearance", None, true;
    ThemeSystem, "theme.system", "Theme: Follow system", "Appearance", None, true;
    ResetLayout, "layout.reset", "Reset layout", "View", None, true;
    ToggleRecording, "record.toggle", "Start / stop recording", "Output", None, true;
    ToggleReplay, "replay.toggle", "Start / stop replay buffer", "Output", None, true;
    ToggleStream, "stream.toggle", "Go live / end stream", "Output", None, true;
    GoLiveSetup, "go-live-setup", "Add a streaming platform (Twitch, YouTube, Kick…)", "Output", None, true;
    ToggleStudio, "studio.toggle", "Studio mode", "Scenes", Some(Shortcut::ctrl_shift('t')), true;
    ToggleVirtualCamera, "vcam.toggle", "Start / stop virtual camera", "Output", None, true;
    TransitionGo, "transition.go", "Transition Preview to Program", "Scenes", None, true;
    TransitionCut, "transition.cut", "Cut Preview to Program", "Scenes", None, true;
    TransitionShader, "transition.shader", "Edit custom transition shader…", "Scenes", None, true;
    SaveReplay, "replay.save", "Save clip (from the replay buffer)", "Output", Some(Shortcut::ctrl_shift('r')), true;
    RevealOutput, "output.reveal", "Show last recording in folder", "Output", None, true;
    OpenSettings, "settings", "Settings…", "General", Some(Shortcut::ctrl(',')), true;
    OpenAutomation, "automation", "Automation: hotkeys and macros…", "General", None, true;
    OpenStats, "stats", "Stats and health…", "General", None, true;
    OpenClips, "clips.open", "Clips…", "General", None, true;
    ToggleClipMode, "clips.mode", "Clip mode on / off", "Output", None, true;
    OpenMontages, "montage.open", "Montages: make a highlight video…", "General", None, true;
    Deselect, "selection.clear", "Clear selection", "Edit", Some(Shortcut::key(Key::Escape)), false;
}

#[cfg(test)]
pub fn info(action: Action) -> &'static ActionInfo {
    ACTIONS
        .iter()
        .find(|a| a.action == action)
        .unwrap_or(&ACTIONS[0])
}

pub fn by_id(id: &str) -> Option<Action> {
    ACTIONS.iter().find(|a| a.id == id).map(|a| a.action)
}

/// Resolves a key press to an action. Ctrl+Y is accepted as an alternative redo.
pub fn for_key(key: Key, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
    if ctrl && !alt && key == Key::Char('y') {
        return Some(Action::Redo);
    }
    ACTIONS
        .iter()
        .find(|a| {
            a.shortcut
                .is_some_and(|s| s.key == key && s.ctrl == ctrl && s.shift == shift && s.alt == alt)
        })
        .map(|a| a.action)
}

/// Subsequence fuzzy match. Returns a score (higher is better), or `None`.
/// Rewards matches at word starts and consecutive runs.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev_match: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if qi < query.len() && c == query[qi] {
            score += 1;
            let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
            if word_start {
                score += 8;
            }
            if prev_match == Some(i.wrapping_sub(1)) {
                score += 5;
            }
            prev_match = Some(i);
            qi += 1;
        }
    }
    (qi == query.len()).then(|| score - chars.len() as i32 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_shortcuts_are_unique() {
        for (i, a) in ACTIONS.iter().enumerate() {
            for b in &ACTIONS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.action, b.action);
                if let (Some(x), Some(y)) = (a.shortcut, b.shortcut) {
                    assert_ne!(x, y, "{} and {} share a shortcut", a.id, b.id);
                }
            }
        }
    }

    #[test]
    fn key_lookup() {
        assert_eq!(
            for_key(Key::Char('k'), true, false, false),
            Some(Action::CommandPalette)
        );
        assert_eq!(
            for_key(Key::Char('z'), true, true, false),
            Some(Action::Redo)
        );
        assert_eq!(
            for_key(Key::Char('y'), true, false, false),
            Some(Action::Redo)
        );
        assert_eq!(
            for_key(Key::Delete, false, false, false),
            Some(Action::RemoveSelected)
        );
        assert_eq!(for_key(Key::Char('z'), false, false, false), None);
        assert_eq!(info(Action::Redo).shortcut.unwrap().label(), "Ctrl+Shift+Z");
    }

    #[test]
    fn fuzzy_ranks_word_starts() {
        assert!(fuzzy_score("brb", "Scene: BRB").is_some());
        assert!(fuzzy_score("xyz", "Scene: BRB").is_none());
        let a = fuzzy_score("fc", "Fit to canvas").unwrap();
        let b = fuzzy_score("fc", "Toggle effects").unwrap_or(i32::MIN);
        assert!(a > b);
        assert!(fuzzy_score("add col", "Add color source").is_some());
    }
}
