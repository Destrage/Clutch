# Design: add-hotkeys-macros

## Model (`estream_project::automation`, saved in the collection)

```rust
pub struct Automation { pub macros: Vec<Macro> }
pub struct Macro { id: MacroId, name: String, enabled: bool, triggers: Vec<Trigger>, steps: Vec<Step> }
pub enum Trigger {
    Hotkey { chord: Chord },                    // ctrl/alt/shift/win + key name ("F9", "A", "Num1")
    WindowFocus { pattern: String },            // matches program file name or title text, case-insensitive
    Interval { seconds: u32 },
    SceneActivated { scene: SceneId },
    Output { output: OutputKind, started: bool },
}
pub enum Step {
    SwitchScene { scene: SceneId, cut: bool },
    SourceVisible { source: SourceId, mode: Toggle },   // items of the source in the program scene
    SourceMute { source: SourceId, mode: Toggle },
    Output { output: OutputKind, mode: Toggle },        // stream / record / replay / virtual camera
    SaveReplay,
    StudioTransition,
    Action { id: String },                              // a command palette action id
    Wait { ms: u32 },
}
pub enum Toggle { On, Off, Toggle }
```

`Command::SetAutomation { automation }` replaces the whole automation: one undo step, and
`Change::AutomationChanged`. `Project.automation` is `#[serde(default)]` and skipped when empty,
so older files load and save unchanged.

## Runner (`crates/estream-automation`)
- The `Host` trait extends `estream_api::Studio` (the same app surface the remote API uses) with
  `run_action(id) -> bool`.
- `Runner::tick(&mut self, host, now)` is called from the app tick. It:
  1. takes the automation from the engine snapshot, and re-syncs global hotkeys when it changed;
  2. collects fired triggers:
     - hotkeys from the listener channel;
     - the foreground window (polled every 250 ms, fires on the edge into a match);
     - due intervals;
     - program scene changes (main canvas, plus any canvas whose scene matches);
     - output active flags (edges);
  3. starts each fired, enabled macro that isn't already running (and not more than 4 times a
     second, to damp loops);
  4. advances running macros: steps run back to back until a `Wait`, which sets a wake time.
- Steps run through `Studio`/engine commands; `Action` steps call `host.run_action`. A step that
  fails is logged and recorded in `last_error` (shown in the editor), and the macro continues.
- Macros can also be started directly with `run(name_or_id)` (the Run now button and
  `TriggerHotkeyByName`).

## Global hotkeys (`estream_automation::hotkeys`, Windows)
- A thread owns a message-only window. Commands arrive over a channel and wake it with
  `PostThreadMessage`.
- `set(Vec<(u32 id, Chord)>)` unregisters the old hotkeys, registers the new ones
  (`MOD_NOREPEAT`), and returns the ones that failed, which the editor shows as "in use by
  another program".
- `WM_HOTKEY` sends the id back to the runner.
- Key names map to virtual keys:
  - letters and digits;
  - F1–F24;
  - Num0–Num9 and the numpad operators;
  - Space, Enter, Tab, Escape, Backspace, Insert, Delete, Home, End, PageUp, PageDown;
  - arrows, media keys and punctuation.
- Unit tests check that chords round-trip through text and that VK mapping is right. A system
  test registers an unusual chord (Ctrl+Alt+Shift+F24) and injects it with `SendInput` to check
  the delivery path.

## App
- **Automation dialog** (overlay modal):
  - **Left:** the macros (enabled toggle, name, trigger summary), plus Add and Remove.
  - **Right:** the editor:
    - name;
    - triggers, each with its fields: chord recorder, pattern text, seconds, scene picker,
      output plus started/stopped;
    - steps, each with its fields (scene, source and action pickers, toggle mode, milliseconds),
      with move up/down and remove;
    - "Run now";
    - the last error.
- **Chord recorder:** a FocusScope captures the next key press with modifiers. Slint key text
  maps to key names, and Escape cancels.
- **Edits:** every change builds the new `Automation` and runs `SetAutomation`. Text and number
  fields coalesce into one undo step.
- **Remote API:** `Studio::macro_names()` and `Studio::run_macro(name)`, with defaults for other
  implementers. They back `GetHotkeyList` and `TriggerHotkeyByName`.

## Testing
- Model: serde round trip, and old files without `automation` still load.
- Command: `SetAutomation` and its undo.
- Runner, with a fake host on a headless engine:
  - a hotkey id starts its macro;
  - steps switch scene, mute and toggle visibility;
  - `Wait` delays the next step by the right time (simulated clock);
  - an interval fires repeatedly;
  - a scene-activated trigger fires once per change;
  - a running macro isn't restarted;
  - a disabled macro never runs.
- Windows hotkeys: chord parsing, VK mapping, and an injected-chord system test.
- In-app: create a macro (hotkey plus switch scene) in the dialog, press the hotkey while the
  window isn't focused, and see the program switch.
