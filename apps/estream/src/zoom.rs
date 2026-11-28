//! Interface zoom: the window's scale factor is the OS factor times the zoom, so Slint lays
//! out and draws everything (text, icons, controls, popups) larger without any change to the
//! `.slint` files.

use crate::settings::UiZoom;

/// The studio's minimum layout in logical pixels (`min-width` / `min-height` in `app.slint`).
pub const MIN_LOGICAL: (f32, f32) = (980.0, 620.0);

/// Whether `zoom` leaves room for the minimum layout in a work area of `area` physical pixels
/// on a monitor with OS scale factor `os_scale`.
pub fn fits(zoom: UiZoom, area: (u32, u32), os_scale: f32) -> bool {
    let scale = os_scale * zoom.factor();
    area.0 as f32 / scale >= MIN_LOGICAL.0 && area.1 as f32 / scale >= MIN_LOGICAL.1
}

/// The zoom in effect: the chosen one, or the largest level that fits when it doesn't.
/// 100% is always allowed. An unknown work area allows every level.
pub fn effective(chosen: UiZoom, area: Option<(u32, u32)>, os_scale: f32) -> UiZoom {
    let Some(area) = area else { return chosen };
    (0..=chosen.index())
        .rev()
        .map(UiZoom::from_index)
        .find(|z| fits(*z, area, os_scale))
        .unwrap_or_default()
}

/// The next level up (or down) from `from`, if there is one and it fits.
pub fn step(from: UiZoom, up: bool, area: Option<(u32, u32)>, os_scale: f32) -> Option<UiZoom> {
    let i = from.index();
    let next = if up {
        (i + 1 < UiZoom::LEVELS.len()).then(|| UiZoom::from_index(i + 1))?
    } else {
        UiZoom::from_index(i.checked_sub(1)?)
    };
    (!up || area.is_none_or(|a| fits(next, a, os_scale))).then_some(next)
}

/// Labels for the Settings list: levels that don't fit this screen say so.
pub fn labels(area: Option<(u32, u32)>, os_scale: f32) -> Vec<String> {
    UiZoom::LEVELS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let z = UiZoom::from_index(i);
            if area.is_none_or(|a| fits(z, a, os_scale)) {
                format!("{p}%")
            } else {
                format!("{p}% (too large for this screen)")
            }
        })
        .collect()
}

/// Where the window is and what the zoom last set, kept between ticks.
#[derive(Default)]
pub struct ZoomState {
    /// The scale factor last dispatched to the window.
    pub applied: Option<f32>,
    /// The zoom in effect (the chosen one, or stepped down to fit this monitor).
    pub effective: UiZoom,
    /// The window's monitor: work area in physical pixels and OS scale factor.
    pub monitor: Option<((u32, u32), f32)>,
    pub checked_at: Option<std::time::Instant>,
}

impl ZoomState {
    pub fn area(&self) -> Option<(u32, u32)> {
        self.monitor.map(|m| m.0)
    }

    pub fn os_scale(&self) -> f32 {
        self.monitor.map_or(1.0, |m| m.1)
    }
}

/// The work area (monitor minus taskbar or dock) of the window's monitor in physical pixels,
/// and the OS scale factor, read from the winit window. `None` before the window exists.
pub fn monitor_of(window: &slint::Window) -> Option<((u32, u32), f32)> {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|ww| {
        let os_scale = ww.scale_factor() as f32;
        let area = work_area(ww).or_else(|| {
            // No work area API here: allow for a taskbar or dock of about 48 logical px.
            let size = ww.current_monitor()?.size();
            let bar = (48.0 * os_scale) as u32;
            Some((size.width, size.height.saturating_sub(bar)))
        })?;
        Some((area, os_scale))
    })?
}

#[cfg(windows)]
fn work_area(ww: &slint::winit_030::winit::window::Window) -> Option<(u32, u32)> {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    let RawWindowHandle::Win32(handle) = ww.window_handle().ok()?.as_raw() else {
        return None;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut core::ffi::c_void);
    // SAFETY: `hwnd` is the live window winit owns; MonitorFromWindow with DEFAULTTONEAREST
    // always returns a monitor, and `info` is sized and owned here.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcWork;
        Some((
            (r.right - r.left).max(0) as u32,
            (r.bottom - r.top).max(0) as u32,
        ))
    }
}

#[cfg(not(windows))]
fn work_area(_ww: &slint::winit_030::winit::window::Window) -> Option<(u32, u32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(p: u16) -> UiZoom {
        UiZoom::try_from(p).unwrap()
    }

    #[test]
    fn fit_rule_per_monitor() {
        // 1920×1080 at 100% OS scaling, 48 px taskbar: up to 150%.
        let fhd = Some((1920, 1032));
        assert_eq!(effective(z(200), fhd, 1.0), z(150));
        assert_eq!(effective(z(150), fhd, 1.0), z(150));
        assert_eq!(effective(z(125), fhd, 1.0), z(125));
        // 1366×768 laptop: only 100%.
        assert_eq!(effective(z(175), Some((1366, 728)), 1.0), z(100));
        // 4K at 150% OS scaling: 2560×1387 logical, up to 200%.
        assert_eq!(effective(z(200), Some((3840, 2080)), 1.5), z(200));
        // Unknown monitor: the chosen level.
        assert_eq!(effective(z(175), None, 1.0), z(175));
    }

    #[test]
    fn steps_stop_at_what_fits() {
        let fhd = Some((1920, 1032));
        assert_eq!(step(z(100), true, fhd, 1.0), Some(z(125)));
        assert_eq!(step(z(150), true, fhd, 1.0), None);
        assert_eq!(step(z(150), false, fhd, 1.0), Some(z(125)));
        assert_eq!(step(z(100), false, fhd, 1.0), None);
        assert_eq!(step(z(200), true, None, 1.0), None);
    }

    #[test]
    fn labels_mark_levels_that_dont_fit() {
        let l = labels(Some((1920, 1032)), 1.0);
        assert_eq!(l[2], "150%");
        assert_eq!(l[3], "175% (too large for this screen)");
    }
}
