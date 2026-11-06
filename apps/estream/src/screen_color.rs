//! Reads the color under the mouse anywhere on screen, for the color picker's eyedropper.
//!
//! The user presses the eyedropper and drags. While the button is held the window keeps the
//! mouse, so the app under the pointer never gets a click; on release the color there is taken.
//! Windows only for now: macOS (`NSColorSampler`) and Linux (the `PickColor` portal) pick with
//! a click of their own and come in their platform branches.

use estream_core::Rgba8;

/// Whether this system can pick colors from the screen.
pub const AVAILABLE: bool = cfg!(windows);

/// The pointer's color and whether the left button is still down, or `None` when it can't be
/// read.
pub fn under_pointer() -> Option<(Rgba8, bool)> {
    imp::under_pointer()
}

#[cfg(windows)]
mod imp {
    use estream_core::Rgba8;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn under_pointer() -> Option<(Rgba8, bool)> {
        let mut p = POINT::default();
        // SAFETY: plain Win32 calls with a valid out pointer; the screen DC is released below.
        unsafe {
            GetCursorPos(&mut p).ok()?;
            let dc = GetDC(None);
            if dc.is_invalid() {
                return None;
            }
            let c = GetPixel(dc, p.x, p.y);
            ReleaseDC(None, dc);
            if c.0 == CLR_INVALID {
                return None;
            }
            // COLORREF is 0x00bbggrr.
            let v = c.0;
            let color = Rgba8::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, 255);
            let held = GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0;
            Some((color, held))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use estream_core::Rgba8;

    pub fn under_pointer() -> Option<(Rgba8, bool)> {
        None
    }
}
