//! What the game detector needs from the OS.

use crate::games::Foreground;

/// The foreground window, its process and whether it covers its monitor.
#[cfg(windows)]
pub fn foreground() -> Option<Foreground> {
    use windows::Win32::Foundation::{CloseHandle, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    };
    // SAFETY: read-only queries about the foreground window, its monitor and its process;
    // every buffer outlives the call that fills it.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
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
        let mut rect = RECT::default();
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let fullscreen = GetWindowRect(hwnd, &mut rect).is_ok()
            && GetMonitorInfoW(monitor, &mut info).as_bool()
            && rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom;
        Some(Foreground {
            pid,
            program,
            title: String::from_utf16_lossy(&title[..n]),
            fullscreen,
        })
    }
}

/// The active window on X11 (EWMH `_NET_ACTIVE_WINDOW`). Wayland doesn't tell apps which
/// window is in front, so there it is `None`.
#[cfg(target_os = "linux")]
pub fn foreground() -> Option<Foreground> {
    x11::foreground()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn foreground() -> Option<Foreground> {
    None
}

/// Whether [`foreground`] can work in this session.
pub fn foreground_available() -> bool {
    cfg!(windows)
        || (cfg!(target_os = "linux")
            && std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "x11"))
}

#[cfg(target_os = "linux")]
mod x11 {
    use super::Foreground;
    use std::sync::{Mutex, OnceLock};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};
    use x11rb::rust_connection::RustConnection;

    struct Display {
        conn: RustConnection,
        root: Window,
        atoms: [Atom; 6],
    }

    const NAMES: [&str; 6] = [
        "_NET_ACTIVE_WINDOW",
        "_NET_WM_NAME",
        "UTF8_STRING",
        "_NET_WM_PID",
        "_NET_WM_STATE",
        "_NET_WM_STATE_FULLSCREEN",
    ];

    /// One connection, opened on first use (only in X11 sessions).
    fn display() -> Option<&'static Mutex<Display>> {
        static DISPLAY: OnceLock<Option<Mutex<Display>>> = OnceLock::new();
        DISPLAY
            .get_or_init(|| {
                if !super::foreground_available() {
                    return None;
                }
                let (conn, screen) = x11rb::connect(None).ok()?;
                let root = conn.setup().roots.get(screen)?.root;
                let mut atoms = [0; 6];
                for (atom, name) in atoms.iter_mut().zip(NAMES) {
                    *atom = conn
                        .intern_atom(false, name.as_bytes())
                        .ok()?
                        .reply()
                        .ok()?
                        .atom;
                }
                Some(Mutex::new(Display { conn, root, atoms }))
            })
            .as_ref()
    }

    fn property(d: &Display, window: Window, prop: Atom, kind: impl Into<Atom>) -> Option<Vec<u8>> {
        let reply = d
            .conn
            .get_property(false, window, prop, kind, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        (reply.format != 0).then_some(reply.value)
    }

    fn u32s(bytes: &[u8]) -> Vec<u32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_ne_bytes(*b))
            .collect()
    }

    pub fn foreground() -> Option<Foreground> {
        let d = display()?.lock().ok()?;
        let [active, wm_name, utf8, wm_pid, wm_state, fullscreen] = d.atoms;
        let window = *u32s(&property(&d, d.root, active, AtomEnum::WINDOW)?).first()?;
        if window == 0 {
            return None;
        }
        let title = property(&d, window, wm_name, utf8)
            .or_else(|| property(&d, window, AtomEnum::WM_NAME.into(), AtomEnum::STRING))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let pid = property(&d, window, wm_pid, AtomEnum::CARDINAL)
            .and_then(|b| u32s(&b).first().copied())
            .unwrap_or(0);
        let program = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        if title.is_empty() && program.is_empty() {
            return None; // not an app window (or one X11 can't see, under Wayland)
        }
        let fullscreen = property(&d, window, wm_state, AtomEnum::ATOM)
            .is_some_and(|b| u32s(&b).contains(&fullscreen));
        Some(Foreground {
            pid,
            program,
            title,
            fullscreen,
        })
    }
}

/// Whether process `pid` is still running.
#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle is closed before returning; `code` outlives the call.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(process, &mut code).is_ok();
        let _ = CloseHandle(process);
        ok && code == STILL_ACTIVE.0 as u32
    }
}

#[cfg(target_os = "linux")]
pub fn is_alive(pid: u32) -> bool {
    pid != 0 && std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn is_alive(_pid: u32) -> bool {
    false
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_a_bogus_one_is_not() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(0xFFFF_FFF0));
    }
}
