//! Enumeration of monitors and capturable top-level windows.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub handle: HMONITOR,
    /// Stable device name, e.g. `\\.\DISPLAY1`.
    pub id: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
}

impl MonitorInfo {
    pub fn label(&self, index: usize) -> String {
        format!(
            "Display {}: {} × {}{}",
            index + 1,
            self.width,
            self.height,
            if self.primary { " (primary)" } else { "" }
        )
    }
}

pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn callback(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<MonitorInfo>` passed below, alive for the call.
        let out = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: `info` is a properly sized MONITORINFOEXW.
        if unsafe { GetMonitorInfoW(m, &mut info.monitorInfo) }.as_bool() {
            let r = info.monitorInfo.rcMonitor;
            out.push(MonitorInfo {
                handle: m,
                id: from_wide(&info.szDevice),
                width: r.right - r.left,
                height: r.bottom - r.top,
                x: r.left,
                y: r.top,
                primary: info.monitorInfo.dwFlags & 1 != 0,
            });
        }
        BOOL(1)
    }
    let mut out: Vec<MonitorInfo> = Vec::new();
    // SAFETY: the callback only touches `out` through the LPARAM during the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut out as *mut _ as isize),
        );
    }
    // Primary first, then left-to-right.
    out.sort_by_key(|m| (!m.primary, m.x, m.y));
    out
}

pub fn monitor_by_id(id: &str) -> Option<MonitorInfo> {
    monitors().into_iter().find(|m| m.id == id)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub class: String,
    /// Executable file name, e.g. `obs64.exe`.
    pub exe: String,
    /// The owning process.
    pub pid: u32,
}

// SAFETY: HWND is a Win32 window handle that can be safely referenced across threads.
unsafe impl Send for WindowInfo {}
unsafe impl Sync for WindowInfo {}

impl WindowInfo {
    /// Stable match key stored in settings: `exe:class:title`.
    pub fn key(&self) -> String {
        format!("{}\u{1f}{}\u{1f}{}", self.exe, self.class, self.title)
    }

    pub fn label(&self) -> String {
        format!("[{}] {}", self.exe, self.title)
    }

    /// True if the window covers its whole monitor (fullscreen or borderless).
    pub fn is_fullscreen(&self) -> bool {
        let mut rect = RECT::default();
        // SAFETY: plain Win32 queries on a window handle.
        unsafe {
            if GetWindowRect(self.hwnd, &mut rect).is_err() {
                return false;
            }
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return false;
            }
            let m = info.rcMonitor;
            rect.left <= m.left
                && rect.top <= m.top
                && rect.right >= m.right
                && rect.bottom >= m.bottom
        }
    }
}

/// Parses a key produced by [`WindowInfo::key`] into `(exe, class, title)`.
pub fn parse_key(key: &str) -> (String, String, String) {
    let mut parts = key.splitn(3, '\u{1f}');
    let exe = parts.next().unwrap_or_default().to_owned();
    let class = parts.next().unwrap_or_default().to_owned();
    let title = parts.next().unwrap_or_default().to_owned();
    (exe, class, title)
}

/// Finds the best match for a stored key: exact, then exe+class, then exe+title, then exe.
pub fn find_window(key: &str, windows: &[WindowInfo]) -> Option<WindowInfo> {
    let (exe, class, title) = parse_key(key);
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let rules: [&dyn Fn(&WindowInfo) -> bool; 4] = [
        &|w| eq(&w.exe, &exe) && w.class == class && w.title == title,
        &|w| eq(&w.exe, &exe) && w.class == class,
        &|w| eq(&w.exe, &exe) && w.title == title,
        &|w| eq(&w.exe, &exe),
    ];
    if exe.is_empty() {
        return None;
    }
    rules
        .iter()
        .find_map(|rule| windows.iter().find(|w| rule(w)).cloned())
}

/// Windows that can be captured now (minimized ones can't).
pub fn windows() -> Vec<WindowInfo> {
    list_windows(false)
}

/// Windows to offer in the window pickers: also minimized ones (fullscreen games minimize
/// while another window has focus), which are captured once restored.
pub fn picker_windows() -> Vec<WindowInfo> {
    list_windows(true)
}

fn list_windows(minimized: bool) -> Vec<WindowInfo> {
    unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut (Vec<WindowInfo>, bool)` passed below.
        let (out, minimized) = unsafe { &mut *(data.0 as *mut (Vec<WindowInfo>, bool)) };
        if let Some(info) = describe(hwnd, *minimized) {
            out.push(info);
        }
        BOOL(1)
    }
    let mut state: (Vec<WindowInfo>, bool) = (Vec::new(), minimized);
    // SAFETY: the callback only touches `state` through the LPARAM during the call.
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut state as *mut _ as isize));
    }
    state.0
}

/// The foreground window if it is a capturable fullscreen application.
pub fn foreground_fullscreen() -> Option<WindowInfo> {
    // SAFETY: simple Win32 query.
    let hwnd = unsafe { GetForegroundWindow() };
    let info = describe(hwnd, false)?;
    let ignored = [
        "explorer.exe",
        "clutch.exe",
        "estream.exe",
        "searchhost.exe",
        "shellexperiencehost.exe",
    ];
    (info.is_fullscreen() && !ignored.iter().any(|e| info.exe.eq_ignore_ascii_case(e)))
        .then_some(info)
}

/// Returns true if the window handle is valid and belongs to the expected process.
pub fn is_window_valid(hwnd: HWND, expected_pid: u32) -> bool {
    // SAFETY: simple Win32 queries on an HWND handle.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == expected_pid
    }
}

/// Describes a window if it's a normal, visible, capturable top-level window (or a minimized
/// one, with `minimized`).
fn describe(hwnd: HWND, minimized: bool) -> Option<WindowInfo> {
    // SAFETY: Win32 queries on a window handle; buffers are sized correctly.
    unsafe {
        if hwnd.is_invalid()
            || !IsWindowVisible(hwnd).as_bool()
            || (!minimized && IsIconic(hwnd).as_bool())
        {
            return None;
        }
        // Skip our own windows *before* any call that may send them a message
        // (GetWindowText sends WM_GETTEXT within a process and would deadlock
        // if our UI thread is busy waiting on the caller).
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok()
            && cloaked != 0
        {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut title = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut title);
        let title = String::from_utf16_lossy(&title[..n as usize]);
        let mut class = [0u16; 256];
        let n = GetClassNameW(hwnd, &mut class);
        let class = String::from_utf16_lossy(&class[..n as usize]);

        let exe = process_name(pid).unwrap_or_default();
        Some(WindowInfo {
            hwnd,
            title,
            class,
            exe,
            pid,
        })
    }
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: the process handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_owned)
    }
}

fn from_wide(s: &[u16]) -> String {
    let end = s.iter().position(|c| *c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(exe: &str, class: &str, title: &str) -> WindowInfo {
        WindowInfo {
            hwnd: HWND::default(),
            title: title.into(),
            class: class.into(),
            exe: exe.into(),
            pid: 0,
        }
    }

    #[test]
    fn window_matching_priority() {
        let list = vec![
            w("game.exe", "Main", "Game — Level 2"),
            w("chrome.exe", "Chrome_WidgetWin_1", "Twitch"),
            w("chrome.exe", "Chrome_WidgetWin_1", "YouTube"),
        ];
        // Exact match.
        let key = w("chrome.exe", "Chrome_WidgetWin_1", "YouTube").key();
        assert_eq!(find_window(&key, &list).unwrap().title, "YouTube");
        // Title changed (e.g. level name): exe + class still matches.
        let key = w("game.exe", "Main", "Game — Level 1").key();
        assert_eq!(find_window(&key, &list).unwrap().title, "Game — Level 2");
        // Case-insensitive exe; unknown app → none.
        assert!(find_window(&w("GAME.EXE", "x", "y").key(), &list).is_some());
        assert!(find_window(&w("other.exe", "x", "y").key(), &list).is_none());
    }

    #[test]
    fn enumeration_runs() {
        // Can't assert specific monitors/windows on CI, but must not crash and
        // there is always at least one monitor on a desktop session.
        let monitors = monitors();
        if !monitors.is_empty() {
            assert!(monitors.iter().any(|m| m.primary));
        }
        let _ = windows();
    }

    #[test]
    fn invalid_window_is_not_valid() {
        assert!(!is_window_valid(HWND::default(), 0));
    }
}
