# Change: Hotkeys, macros and triggers

## Why
Streamers drive their show with keys and automation. Per-action hotkeys cover the
basics; the rest needs automation built in. Examples:
- switch to "Game" when the game window takes focus;
- start the replay buffer whenever the stream starts;
- cycle "Be right back" scenes on a timer.

eStream builds this in, in one simple model: **macros** (a list of steps) started by **triggers**
(a global hotkey, a window gaining focus, an interval, a scene going on program, or an output
starting or stopping).

## What Changes
- **automation** capability (new). It is saved with the scene collection, because macros refer to
  its scenes and sources, and every edit is a command (undoable).
  - **Macro:** name, enabled, triggers, steps.
  - **Steps:**
    - switch program scene (with the current transition, or a cut);
    - show, hide or toggle a source in the program scene;
    - mute, unmute or toggle a source;
    - start, stop or toggle an output (stream, record, replay buffer, virtual camera);
    - save a replay;
    - studio mode transition;
    - run any command palette action;
    - wait.
  - **Triggers:**
    - a global hotkey, which works while another app is focused;
    - a window gaining focus (by program name or title text);
    - every N seconds/minutes;
    - a scene going on program;
    - an output starting or stopping.
- **Runtime:**
  - Macros run without blocking the UI. Waits are scheduled, not slept.
  - A macro that is already running isn't started again, and fast trigger loops are damped.
  - Steps run through the same paths as the UI and the remote API, using the command bus and
    app actions.
- **Global hotkeys:**
  - Windows `RegisterHotKey` runs on a message-only window thread.
  - A combination already taken by another program is reported in the editor.
- **UI:**
  - an Automation dialog (top bar button and palette action) with a macro list and an editor for
    triggers and steps;
  - hotkeys are recorded by pressing them;
  - a "Run now" button runs a macro immediately.
- **Remote API:** `GetHotkeyList` lists macro names, and `TriggerHotkeyByName` runs a macro by
  name, so Stream Deck "hotkey" buttons work.

## Out of scope
- Conditions (if/else) and variables. Triggers plus steps cover the common cases first.
- Scene-specific hotkeys (a hotkey that works only while a given scene is active).
- Hotkeys on macOS and Linux.

## Impact
- Affected specs: `automation` (new), `remote-api` (hotkey requests)
- Affected code:
  - `estream-project` (automation model);
  - `estream-engine` (`SetAutomation` command);
  - new `crates/estream-automation` (runner, triggers, Windows hotkeys);
  - `apps/estream` (dialog and wiring);
  - `estream-api`.
