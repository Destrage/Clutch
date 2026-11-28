# Design: add-interface-zoom

## Scaling the window
Slint lays out in logical pixels and multiplies by the window's scale factor. Zoom sets that
factor to `os_scale × zoom`:

- At runtime: dispatch `WindowEvent::ScaleFactorChanged { scale_factor: os_scale * zoom }`
  to each studio window. Slint re-lays out and redraws, so nothing in the `.slint` files has
  to change, and every component scales the same way.
- A `Resized` event with the window's unchanged physical size in the new logical units
  follows, so the layout fills the window at once and the window keeps its size on screen.
- The winit backend dispatches the OS factor itself on monitor or DPI changes. The app's UI
  tick notices that the factor is no longer the one it set, reads the OS factor from the winit
  window (`unstable-winit-030`), and dispatches the combined value again. The monitor's work
  area is read every 500 ms (Windows: `GetMonitorInfoW`; elsewhere the monitor size less a
  48 px allowance for a taskbar or dock).
- At startup the zoom is applied right after `show()`, before the event loop draws.

Decision (task 1.1): the runtime path works with the winit backend on Windows (verified: start
at 150%, Ctrl+Shift+= and Ctrl+Shift+0 at runtime). Linux and macOS still need the same check;
`SLINT_SCALE_FACTOR` on restart stays the fallback for a backend where it doesn't hold.

## Window width without a layout loop
The compact top bar depends on the window's width, but the window's `width` depends on its
layout, so binding to it is a loop. The app sets `window-width` (physical width ÷ scale factor)
from the UI tick instead.

## The canvas
The preview renders the wgpu canvas texture into an `Image`. Its geometry stays in logical
pixels; "Zoom to 100%" sets the scale to `1phx / 1px` and the zoom label shows
`scale × 1px / 1phx`, so "100%" means one canvas pixel per screen pixel at any OS scaling or
interface zoom.

## Minimum size
The studio window's minimum logical size is 980×620 (`app.slint`). A zoom level is offered only if
`work_area_physical_size / (os_scale × zoom)` is at least that. The work area is the monitor
minus the taskbar or dock. A 1920×1080 monitor at 100% OS scaling allows up to 150%
(1280×~690 logical); a 1366×768 laptop allows only 100%. Panels are never clipped.

The 980 px minimum only holds with the compact top bar: with full labels the bar needs about
1,340 px, which also cut off Go Live in narrow windows at 100% before this change.

## Text size
`TextSize::Largest` is added at 1.5. `Theme.grow` already follows `text-scale`, so controls
holding text grow with it. Older versions don't know `largest`; the lenient parser already
falls back to Default for unknown values (Settings Survive Version Changes).
