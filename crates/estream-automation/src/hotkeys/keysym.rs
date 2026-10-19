//! Key names (see [`Chord`]) as XKB keysyms: the name the GlobalShortcuts portal's triggers
//! use, and the value an X11 key grab looks up.

use estream_project::Chord;

/// The keysym name and value for a key name.
pub fn keysym(name: &str) -> Option<(String, u32)> {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_uppercase() {
        let lower = bytes[0].to_ascii_lowercase();
        return Some(((lower as char).to_string(), lower as u32));
    }
    if bytes.len() == 1 && bytes[0].is_ascii_digit() {
        return Some((upper.clone(), bytes[0] as u32));
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some((format!("F{n}"), 0xFFBE + n - 1));
    }
    if let Some(n) = upper
        .strip_prefix("NUM")
        .and_then(|n| n.parse::<u32>().ok())
        && n <= 9
    {
        return Some((format!("KP_{n}"), 0xFFB0 + n));
    }
    let (name, value) = match upper.as_str() {
        "NUMMULTIPLY" => ("KP_Multiply", 0xFFAA),
        "NUMADD" => ("KP_Add", 0xFFAB),
        "NUMSUBTRACT" => ("KP_Subtract", 0xFFAD),
        "NUMDECIMAL" => ("KP_Decimal", 0xFFAE),
        "NUMDIVIDE" => ("KP_Divide", 0xFFAF),
        "SPACE" => ("space", 0x20),
        "ENTER" => ("Return", 0xFF0D),
        "TAB" => ("Tab", 0xFF09),
        "ESCAPE" => ("Escape", 0xFF1B),
        "BACKSPACE" => ("BackSpace", 0xFF08),
        "INSERT" => ("Insert", 0xFF63),
        "DELETE" => ("Delete", 0xFFFF),
        "HOME" => ("Home", 0xFF50),
        "END" => ("End", 0xFF57),
        "PAGEUP" => ("Page_Up", 0xFF55),
        "PAGEDOWN" => ("Page_Down", 0xFF56),
        "LEFT" => ("Left", 0xFF51),
        "UP" => ("Up", 0xFF52),
        "RIGHT" => ("Right", 0xFF53),
        "DOWN" => ("Down", 0xFF54),
        "PAUSE" => ("Pause", 0xFF13),
        "SCROLLLOCK" => ("Scroll_Lock", 0xFF14),
        "PRINTSCREEN" => ("Print", 0xFF61),
        "MEDIAPLAYPAUSE" => ("XF86AudioPlay", 0x1008_FF14),
        "MEDIANEXT" => ("XF86AudioNext", 0x1008_FF17),
        "MEDIAPREVIOUS" => ("XF86AudioPrev", 0x1008_FF16),
        "MEDIASTOP" => ("XF86AudioStop", 0x1008_FF15),
        "VOLUMEMUTE" => ("XF86AudioMute", 0x1008_FF12),
        "VOLUMEDOWN" => ("XF86AudioLowerVolume", 0x1008_FF11),
        "VOLUMEUP" => ("XF86AudioRaiseVolume", 0x1008_FF13),
        ";" => ("semicolon", 0x3B),
        "=" => ("equal", 0x3D),
        "," => ("comma", 0x2C),
        "-" => ("minus", 0x2D),
        "." => ("period", 0x2E),
        "/" => ("slash", 0x2F),
        "`" => ("grave", 0x60),
        "[" => ("bracketleft", 0x5B),
        "\\" => ("backslash", 0x5C),
        "]" => ("bracketright", 0x5D),
        "'" => ("apostrophe", 0x27),
        _ => return None,
    };
    Some((name.to_owned(), value))
}

/// A chord as a shortcut trigger (the XDG shortcuts format the portal takes), e.g.
/// `CTRL+SHIFT+F9`.
pub fn trigger(chord: &Chord) -> Option<String> {
    let (key, _) = keysym(&chord.key)?;
    let mut out = String::new();
    for (on, name) in [
        (chord.ctrl, "CTRL+"),
        (chord.alt, "ALT+"),
        (chord.shift, "SHIFT+"),
        (chord.win, "LOGO+"),
    ] {
        if on {
            out.push_str(name);
        }
    }
    out.push_str(&key);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_keysyms() {
        assert_eq!(keysym("a"), Some(("a".into(), 0x61)));
        assert_eq!(keysym("7"), Some(("7".into(), 0x37)));
        assert_eq!(keysym("F1"), Some(("F1".into(), 0xFFBE)));
        assert_eq!(keysym("F24"), Some(("F24".into(), 0xFFD5)));
        assert_eq!(keysym("Num5").map(|k| k.1), Some(0xFFB5));
        assert_eq!(keysym("PageDown").map(|k| k.0), Some("Page_Down".into()));
        assert_eq!(keysym("Nope"), None);
        // Every key the Windows table knows has a keysym too.
        for name in [
            "Space",
            "MediaPlayPause",
            "VolumeUp",
            "[",
            "'",
            "PrintScreen",
        ] {
            assert!(
                super::super::key_vk(name).is_some() && keysym(name).is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn chords_become_triggers() {
        let chord = Chord::parse("Ctrl+Shift+F9").unwrap();
        assert_eq!(trigger(&chord).as_deref(), Some("CTRL+SHIFT+F9"));
        let chord = Chord::parse("Win+Alt+R").unwrap();
        assert_eq!(trigger(&chord).as_deref(), Some("ALT+LOGO+r"));
    }
}
