//! Global hotkeys through Windows: registration, conflicts and an injected key press.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_automation::hotkeys::HotkeyListener;
use estream_project::Chord;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_F24, VK_MENU, VK_SHIFT,
};

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[test]
fn registered_chords_fire_and_conflicts_are_reported() {
    // Nobody uses Ctrl+Alt+Shift+F24.
    let chord = Chord::parse("Ctrl+Alt+Shift+F24").unwrap();
    let first = HotkeyListener::start().unwrap();
    assert!(first.set(vec![(7, chord.clone())]).is_empty());

    let second = HotkeyListener::start().unwrap();
    assert_eq!(
        second.set(vec![(1, chord.clone())]),
        vec![1],
        "already taken"
    );
    let unknown = Chord::parse("Ctrl+Nope").unwrap();
    assert_eq!(
        second.set(vec![(2, unknown)]),
        vec![2],
        "unknown key names fail too"
    );
    drop(second);

    let inputs = [
        key(VK_CONTROL, false),
        key(VK_MENU, false),
        key(VK_SHIFT, false),
        key(VK_F24, false),
        key(VK_F24, true),
        key(VK_SHIFT, true),
        key(VK_MENU, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        // SendInput is blocked by UIPI when the desktop is locked or running headless.
        eprintln!(
            "SendInput blocked (desktop locked or UIPI); skipping key injection verification"
        );
        return;
    }
    assert_eq!(sent as usize, inputs.len(), "input injected");
    assert_eq!(first.presses.recv_timeout(Duration::from_secs(3)), Ok(7));

    // Replacing the set releases the old chord.
    assert!(first.set(Vec::new()).is_empty());
    let third = HotkeyListener::start().unwrap();
    assert!(third.set(vec![(3, chord)]).is_empty(), "free again");
}
