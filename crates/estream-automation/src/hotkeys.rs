//! Global hotkeys: a thread that owns `RegisterHotKey` registrations and reports presses.

use estream_project::Chord;

/// Virtual-key code for a key name (see [`Chord`]).
pub fn key_vk(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return Some(bytes[0] as u32);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some(0x70 + n - 1);
    }
    if let Some(n) = upper
        .strip_prefix("NUM")
        .and_then(|n| n.parse::<u32>().ok())
        && n <= 9
    {
        return Some(0x60 + n);
    }
    Some(match upper.as_str() {
        "NUMMULTIPLY" => 0x6A,
        "NUMADD" => 0x6B,
        "NUMSUBTRACT" => 0x6D,
        "NUMDECIMAL" => 0x6E,
        "NUMDIVIDE" => 0x6F,
        "SPACE" => 0x20,
        "ENTER" => 0x0D,
        "TAB" => 0x09,
        "ESCAPE" => 0x1B,
        "BACKSPACE" => 0x08,
        "INSERT" => 0x2D,
        "DELETE" => 0x2E,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "PAUSE" => 0x13,
        "SCROLLLOCK" => 0x91,
        "PRINTSCREEN" => 0x2C,
        "MEDIAPLAYPAUSE" => 0xB3,
        "MEDIANEXT" => 0xB0,
        "MEDIAPREVIOUS" => 0xB1,
        "MEDIASTOP" => 0xB2,
        "VOLUMEMUTE" => 0xAD,
        "VOLUMEDOWN" => 0xAE,
        "VOLUMEUP" => 0xAF,
        ";" => 0xBA,
        "=" => 0xBB,
        "," => 0xBC,
        "-" => 0xBD,
        "." => 0xBE,
        "/" => 0xBF,
        "`" => 0xC0,
        "[" => 0xDB,
        "\\" => 0xDC,
        "]" => 0xDD,
        "'" => 0xDE,
        _ => return None,
    })
}

#[cfg(windows)]
pub use imp::HotkeyListener;

#[cfg(windows)]
mod imp {
    use super::key_vk;
    use crossbeam_channel::{Receiver, Sender};
    use estream_project::Chord;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
        UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
        WM_QUIT, WM_USER,
    };

    enum Cmd {
        /// Replace all registrations; reply with the ids that couldn't be registered.
        Set(Vec<(u32, Chord)>, Sender<Vec<u32>>),
    }

    /// Owns the registrations on a dedicated thread (hotkeys belong to the registering thread).
    pub struct HotkeyListener {
        commands: Sender<Cmd>,
        thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
        /// Ids of pressed hotkeys.
        pub presses: Receiver<u32>,
    }

    const WM_COMMANDS: u32 = WM_APP + 1;

    impl HotkeyListener {
        pub fn start() -> Option<Self> {
            let (commands, rx) = crossbeam_channel::unbounded::<Cmd>();
            let (press_tx, presses) = crossbeam_channel::unbounded();
            let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
            let thread = std::thread::Builder::new()
                .name("estream-hotkeys".into())
                .spawn(move || run(rx, press_tx, ready_tx))
                .ok()?;
            let thread_id = ready_rx.recv().ok()?;
            Some(Self {
                commands,
                thread_id,
                thread: Some(thread),
                presses,
            })
        }

        /// Registers exactly these hotkeys (ids are the caller's). Returns the ids another
        /// program already holds (or that name an unknown key).
        pub fn set(&self, hotkeys: Vec<(u32, Chord)>) -> Vec<u32> {
            let (reply, result) = crossbeam_channel::bounded(1);
            if self.commands.send(Cmd::Set(hotkeys, reply)).is_err() {
                return Vec::new();
            }
            // SAFETY: wakes the listener thread's message loop.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_COMMANDS, WPARAM(0), LPARAM(0));
            }
            result
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap_or_default()
        }
    }

    impl Drop for HotkeyListener {
        fn drop(&mut self) {
            // SAFETY: asks the listener thread to leave its message loop.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn modifiers(chord: &Chord) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if chord.ctrl {
            m |= MOD_CONTROL;
        }
        if chord.alt {
            m |= MOD_ALT;
        }
        if chord.shift {
            m |= MOD_SHIFT;
        }
        if chord.win {
            m |= MOD_WIN;
        }
        m
    }

    fn run(commands: Receiver<Cmd>, presses: Sender<u32>, ready: Sender<u32>) {
        let mut msg = MSG::default();
        // SAFETY: creates this thread's message queue before anyone posts to it.
        unsafe {
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
            let _ = ready.send(GetCurrentThreadId());
        }
        let mut registered: Vec<i32> = Vec::new();
        // SAFETY: standard message loop on our own queue.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            match msg.message {
                WM_HOTKEY => {
                    let _ = presses.send(msg.wParam.0 as u32);
                }
                WM_COMMANDS => {
                    while let Ok(Cmd::Set(list, reply)) = commands.try_recv() {
                        for id in registered.drain(..) {
                            // SAFETY: releases a hotkey this thread registered.
                            unsafe {
                                let _ = UnregisterHotKey(None, id);
                            }
                        }
                        let mut failed = Vec::new();
                        for (id, chord) in list {
                            let ok = key_vk(&chord.key).is_some_and(|vk| {
                                // SAFETY: registers for this thread (no window).
                                unsafe { RegisterHotKey(None, id as i32, modifiers(&chord), vk) }
                                    .is_ok()
                            });
                            if ok {
                                registered.push(id as i32);
                            } else {
                                failed.push(id);
                            }
                        }
                        let _ = reply.send(failed);
                    }
                }
                _ => {}
            }
        }
        for id in registered {
            // SAFETY: releases our hotkeys on the way out.
            unsafe {
                let _ = UnregisterHotKey(None, id);
            }
        }
    }
}

/// Key name for a Slint key event's text (`None` for modifier keys and unknown keys).
pub fn key_name_from_slint(text: &str) -> Option<String> {
    let c = text.chars().next()?;
    if text.chars().count() != 1 {
        return None;
    }
    let name = match c as u32 {
        0xF704..=0xF71B => format!("F{}", c as u32 - 0xF704 + 1), // F1–F24
        0x0020 => "Space".into(),
        0x000A | 0x000D => "Enter".into(),
        0x0009 => "Tab".into(),
        0x0008 => "Backspace".into(),
        0x007F => "Delete".into(),
        0xF727 => "Insert".into(),
        0xF729 => "Home".into(),
        0xF72B => "End".into(),
        0xF72C => "PageUp".into(),
        0xF72D => "PageDown".into(),
        0xF700 => "Up".into(),
        0xF701 => "Down".into(),
        0xF702 => "Left".into(),
        0xF703 => "Right".into(),
        0xF730 => "Pause".into(),
        0xF72F => "ScrollLock".into(),
        // Modifiers, Escape and other control keys aren't bindable keys.
        0x0010..=0x0018 | 0x001B | 0xF700..=0xF8FF => return None,
        _ if c.is_ascii_alphanumeric() => c.to_ascii_uppercase().to_string(),
        _ if ";=,-./`[\\]'".contains(c) => c.to_string(),
        _ => return None,
    };
    key_vk(&name).map(|_| name)
}

/// The chord for a key press, if it names a bindable key.
pub fn chord_from_slint(
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
) -> Option<Chord> {
    // Slint reports modifier keys pressed on their own as U+0010–U+0018 (Shift, Control,
    // Alt, AltGr, CapsLock, right Shift/Control, Meta): not a chord yet.
    let mut chars = text.chars();
    let first = chars.clone().next();
    if text.chars().count() == 1 && first.is_some_and(|c| (0x10..=0x18).contains(&(c as u32))) {
        return None;
    }
    // With Ctrl held, letters can arrive as control characters (Ctrl+A = U+0001).
    let key = match (chars.next(), chars.next()) {
        (Some(c), None) if ctrl && (1..=26).contains(&(c as u32)) => {
            ((b'A' + c as u8 - 1) as char).to_string()
        }
        _ => key_name_from_slint(text)?,
    };
    Some(Chord {
        ctrl,
        alt,
        shift,
        win,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_map_to_virtual_keys() {
        assert_eq!(key_vk("A"), Some(0x41));
        assert_eq!(key_vk("a"), Some(0x41));
        assert_eq!(key_vk("7"), Some(0x37));
        assert_eq!(key_vk("F1"), Some(0x70));
        assert_eq!(key_vk("F24"), Some(0x87));
        assert_eq!(key_vk("F25"), None);
        assert_eq!(key_vk("Num5"), Some(0x65));
        assert_eq!(key_vk("PageDown"), Some(0x22));
        assert_eq!(key_vk("MediaPlayPause"), Some(0xB3));
        assert_eq!(key_vk("["), Some(0xDB));
        assert_eq!(key_vk("Nope"), None);
    }

    #[test]
    fn slint_keys_become_names() {
        assert_eq!(key_name_from_slint("a").as_deref(), Some("A"));
        assert_eq!(key_name_from_slint("\u{F70C}").as_deref(), Some("F9"));
        assert_eq!(key_name_from_slint(" ").as_deref(), Some("Space"));
        assert_eq!(key_name_from_slint("\u{F72C}").as_deref(), Some("PageUp"));
        assert_eq!(key_name_from_slint("\u{0010}"), None, "shift alone");
        assert_eq!(key_name_from_slint("\u{001B}"), None, "escape cancels");
        let chord = chord_from_slint("\u{F70C}", true, false, true, false).unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Shift+F9");
        // Holding Ctrl alone arrives as U+0011: not a chord (it used to read as Ctrl+Q).
        assert_eq!(chord_from_slint("\u{11}", true, false, false, false), None);
        assert_eq!(
            chord_from_slint("q", true, false, false, false).map(|c| c.to_string()),
            Some("Ctrl+Q".into())
        );
        assert_eq!(
            chord_from_slint("\u{b}", true, false, false, false).map(|c| c.to_string()),
            Some("Ctrl+K".into())
        );
    }
}
