//! The foreground window, for window-focus triggers.

/// What's in front: the program's file name and the window title.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Foreground {
    pub program: String,
    pub title: String,
}

impl Foreground {
    /// Case-insensitive match of `pattern` against the program file name or the title.
    pub fn matches(&self, pattern: &str) -> bool {
        let pattern = pattern.trim().to_lowercase();
        !pattern.is_empty()
            && (self.program.to_lowercase().contains(&pattern)
                || self.title.to_lowercase().contains(&pattern))
    }
}

/// The current foreground window (`None` if there is none or it can't be read).
#[cfg(windows)]
pub fn foreground() -> Option<Foreground> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };
    // SAFETY: read-only queries about the foreground window and its process.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut program = String::new();
        if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut path = [0u16; 1024];
            let mut len = path.len() as u32;
            if QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut len,
            )
            .is_ok()
            {
                let full = String::from_utf16_lossy(&path[..len as usize]);
                program = full
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            let _ = CloseHandle(process);
        }
        Some(Foreground {
            program,
            title: String::from_utf16_lossy(&title[..n]),
        })
    }
}

#[cfg(not(windows))]
pub fn foreground() -> Option<Foreground> {
    None
}

/// Why the foreground window can't be known on this system (`None` where it can). On Linux
/// only X11 sessions say which window is in front (the app plugs an X11 probe into the
/// runner, see [`crate::Runner::set_focus_probe`]); Wayland keeps it private.
pub fn unavailable() -> Option<&'static str> {
    let x11 =
        cfg!(target_os = "linux") && std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "x11");
    if cfg!(windows) || x11 {
        None
    } else {
        Some("Not available on this system: it doesn't tell apps which window is in front")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_program_or_title() {
        let fg = Foreground {
            program: "VALORANT-Win64-Shipping.exe".into(),
            title: "VALORANT  ".into(),
        };
        assert!(fg.matches("valorant"));
        assert!(fg.matches("Shipping.exe"));
        assert!(!fg.matches("chrome"));
        assert!(!fg.matches("  "), "empty patterns never match");
    }
}
