//! Picks a color from anywhere on screen for the color picker's eyedropper.
//!
//! Windows: while picking, low-level mouse and keyboard hooks catch the next click anywhere
//! (and Esc or a right click to cancel). The hook procedures only record what happened; the
//! app's tick reads it with [`poll`]. The click that picks is swallowed, so the app under the
//! pointer never gets it. Pressing the eyedropper and dragging also works: the release picks.
//! The hooks exist only between [`start`] and [`stop`].
//!
//! macOS (`NSColorSampler`) and Linux (the `PickColor` portal) come in their platform branches;
//! until then [`AVAILABLE`] is false and the eyedropper picks from the Preview only.

use estream_core::Rgba8;

/// Whether this system can pick colors from the screen.
pub const AVAILABLE: bool = cfg!(windows);

/// What happened since picking started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Pick {
    /// Still picking.
    Waiting,
    /// A click picked this color.
    Picked(Rgba8),
    /// Esc or a right click.
    Cancelled,
}

/// Starts catching the next click anywhere. `false` if it couldn't.
pub fn start() -> bool {
    imp::start()
}

/// Stops picking (removes the hooks). Safe to call when not picking.
pub fn stop() {
    imp::stop()
}

/// The result so far. A pick or cancel is reported once.
pub fn poll() -> Pick {
    imp::poll()
}

/// The color under the pointer right now.
pub fn under_pointer() -> Option<Rgba8> {
    imp::under_pointer()
}

/// How far (in pixels) the pointer may move between pressing the eyedropper and releasing it
/// for the release to count as a click on the button rather than a drag-and-pick.
#[cfg_attr(not(windows), allow(dead_code))]
pub const DRAG_SLOP: i32 = 8;

/// Whether a release at `(x, y)` after pressing the eyedropper at `press` picks there (a drag)
/// or only finishes the click on the button.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn release_picks(press: (i32, i32), x: i32, y: i32) -> bool {
    (x - press.0).abs() > DRAG_SLOP || (y - press.1).abs() > DRAG_SLOP
}

#[cfg(windows)]
mod imp {
    use super::{Pick, release_picks};
    use estream_core::Rgba8;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT,
        SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    };

    const WAITING: u8 = 0;
    const PICKED: u8 = 1;
    const CANCELLED: u8 = 2;

    static EVENT: AtomicU8 = AtomicU8::new(WAITING);
    static AT_X: AtomicI32 = AtomicI32::new(0);
    static AT_Y: AtomicI32 = AtomicI32::new(0);
    /// Picking started while the eyedropper was pressed and that button is still down.
    static PRESSED: AtomicBool = AtomicBool::new(false);
    static PRESS_X: AtomicI32 = AtomicI32::new(0);
    static PRESS_Y: AtomicI32 = AtomicI32::new(0);
    /// A left press was swallowed; its release picks.
    static DOWN: AtomicBool = AtomicBool::new(false);

    thread_local! {
        /// The installed (mouse, keyboard) hooks. Installed and removed on the UI thread.
        static HOOKS: Cell<Option<(HHOOK, HHOOK)>> = const { Cell::new(None) };
    }

    pub fn start() -> bool {
        stop();
        let mut p = POINT::default();
        // SAFETY: plain Win32 calls with a valid out pointer.
        let pressed = unsafe {
            let _ = GetCursorPos(&mut p);
            GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0
        };
        EVENT.store(WAITING, Ordering::SeqCst);
        DOWN.store(false, Ordering::SeqCst);
        PRESS_X.store(p.x, Ordering::SeqCst);
        PRESS_Y.store(p.y, Ordering::SeqCst);
        PRESSED.store(pressed, Ordering::SeqCst);
        // SAFETY: the procedures are `extern "system"` functions that live for the whole
        // program; the hooks are removed in `stop` on this same thread.
        let hooks = unsafe {
            let module = GetModuleHandleW(None).ok();
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module.map(Into::into), 0);
            let keys = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_proc),
                module.map(Into::into),
                0,
            );
            match (mouse, keys) {
                (Ok(m), Ok(k)) => Some((m, k)),
                (m, k) => {
                    if let Ok(m) = m {
                        let _ = UnhookWindowsHookEx(m);
                    }
                    if let Ok(k) = k {
                        let _ = UnhookWindowsHookEx(k);
                    }
                    tracing::warn!("eyedropper: couldn't hook the mouse and keyboard");
                    None
                }
            }
        };
        HOOKS.with(|h| h.set(hooks));
        hooks.is_some()
    }

    pub fn stop() {
        if let Some((mouse, keys)) = HOOKS.with(|h| h.take()) {
            // SAFETY: the hooks were installed by `start` on this thread and not yet removed.
            unsafe {
                let _ = UnhookWindowsHookEx(mouse);
                let _ = UnhookWindowsHookEx(keys);
            }
        }
        PRESSED.store(false, Ordering::SeqCst);
        DOWN.store(false, Ordering::SeqCst);
    }

    pub fn poll() -> Pick {
        match EVENT.swap(WAITING, Ordering::SeqCst) {
            PICKED => match color_at(AT_X.load(Ordering::SeqCst), AT_Y.load(Ordering::SeqCst)) {
                Some(c) => Pick::Picked(c),
                None => Pick::Cancelled,
            },
            CANCELLED => Pick::Cancelled,
            _ => Pick::Waiting,
        }
    }

    pub fn under_pointer() -> Option<Rgba8> {
        let mut p = POINT::default();
        // SAFETY: plain Win32 call with a valid out pointer.
        unsafe { GetCursorPos(&mut p).ok()? };
        color_at(p.x, p.y)
    }

    fn color_at(x: i32, y: i32) -> Option<Rgba8> {
        // SAFETY: the screen DC is released before returning.
        unsafe {
            let dc = GetDC(None);
            if dc.is_invalid() {
                return None;
            }
            let c = GetPixel(dc, x, y);
            ReleaseDC(None, dc);
            if c.0 == CLR_INVALID {
                return None;
            }
            // COLORREF is 0x00bbggrr.
            let v = c.0;
            Some(Rgba8::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, 255))
        }
    }

    fn record(event: u8, x: i32, y: i32) {
        AT_X.store(x, Ordering::SeqCst);
        AT_Y.store(y, Ordering::SeqCst);
        EVENT.store(event, Ordering::SeqCst);
    }

    /// Swallowing returns 1; passing on calls the next hook.
    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for WH_MOUSE_LL with HC_ACTION, lparam points at an MSLLHOOKSTRUCT.
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let (x, y) = (info.pt.x, info.pt.y);
            match wparam.0 as u32 {
                // The release of the press that started picking: a drag picks where it ends;
                // a plain click on the button just starts picking. Either way the release goes
                // on, so the window that has the mouse sees it.
                WM_LBUTTONUP if PRESSED.swap(false, Ordering::SeqCst) => {
                    let press = (
                        PRESS_X.load(Ordering::SeqCst),
                        PRESS_Y.load(Ordering::SeqCst),
                    );
                    if release_picks(press, x, y) {
                        record(PICKED, x, y);
                    }
                }
                WM_LBUTTONDOWN => {
                    DOWN.store(true, Ordering::SeqCst);
                    return LRESULT(1);
                }
                WM_LBUTTONUP if DOWN.swap(false, Ordering::SeqCst) => {
                    record(PICKED, x, y);
                    return LRESULT(1);
                }
                WM_RBUTTONDOWN | WM_RBUTTONUP => {
                    record(CANCELLED, x, y);
                    return LRESULT(1);
                }
                _ => {}
            }
        }
        // SAFETY: passing the event on unchanged.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points at a KBDLLHOOKSTRUCT.
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if info.vkCode == VK_ESCAPE.0 as u32 {
                record(CANCELLED, 0, 0);
                return LRESULT(1);
            }
        }
        // SAFETY: passing the event on unchanged.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Pick;
    use estream_core::Rgba8;

    pub fn start() -> bool {
        false
    }

    pub fn stop() {}

    pub fn poll() -> Pick {
        Pick::Waiting
    }

    pub fn under_pointer() -> Option<Rgba8> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_on_the_button_starts_picking_and_a_drag_picks() {
        assert!(!release_picks((100, 100), 100, 100), "click: same spot");
        assert!(!release_picks((100, 100), 104, 97), "click: a small wobble");
        assert!(release_picks((100, 100), 100, 140), "drag down");
        assert!(release_picks((100, 100), 40, 100), "drag left");
    }
}
