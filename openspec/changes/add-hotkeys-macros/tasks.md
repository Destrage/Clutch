# Tasks: add-hotkeys-macros

## 1. Model
- [x] 1.1 `Automation`, `Macro`, `Trigger`, `Step`, `Chord` in `estream-project` (serde, defaults)
- [x] 1.2 `Command::SetAutomation` + `Change::AutomationChanged` (undoable)

## 2. Runner (`crates/estream-automation`)
- [x] 2.1 `Host` trait (Studio + actions); step execution
- [x] 2.2 Triggers: hotkeys, window focus, interval, scene activated, output started/stopped
- [x] 2.3 Scheduling: waits, no re-entry, loop damping, run by name
- [x] 2.4 Windows global hotkeys: listener thread, register/unregister, conflicts, key names

## 3. Studio
- [x] 3.1 Automation dialog: macro list, trigger and step editors, chord recorder, run now
- [x] 3.2 Wiring: runner in the app tick, top bar button, palette action
- [x] 3.3 Remote API: `GetHotkeyList`, `TriggerHotkeyByName`

## 4. Verification
- [x] 4.1 Model, command and runner tests
- [x] 4.2 Hotkey tests (parsing, injected chord)
- [x] 4.3 In-app check: a hotkey macro switches the scene while another window has focus
