# Design: add-interface-zoom

## Scaling the window
Slint lays out in logical pixels and multiplies by the window's scale factor. Zoom sets that
factor to `os_scale × zoom`:

- At runtime: dispatch `WindowEvent::ScaleFactorChanged { scale_factor: os_scale * zoom }`
  to each studio window. Slint re-lays out and redraws, so nothing in the `.slint` files has
  to change, and every component scales the same way.
- The winit backend reports the OS factor on monitor or DPI changes. We keep the last OS
  factor, and after each such change we dispatch the combined value again, so the zoom survives
  moving between monitors.
- At startup the zoom is applied before the first frame, so the window doesn't flash at 100%.

Fallback if `ScaleFactorChanged` proves unreliable on a backend: set `SLINT_SCALE_FACTOR`
before the backend starts and apply zoom on restart, with a "Restart to apply" button. To be
decided during task 1.1 on all three platforms.

## The canvas
The preview renders the wgpu canvas texture into an `Image`. The preview's own zoom
(Fit/100%) is computed in physical pixels, so "100%" still means one canvas pixel per
screen pixel at any interface zoom.

## Minimum size
The studio window's minimum logical size is 980×620 (`app.slint`). A zoom level is offered only if
`work_area_physical_size / (os_scale × zoom)` is at least that. The work area is the monitor
minus the taskbar or dock. A 1920×1080 monitor at 100% OS scaling allows up to 150%
(1280×~690 logical); a 1366×768 laptop allows only 100%. Panels are never clipped.

## Text size
`TextSize::Largest` is added at 1.5. `Theme.grow` already follows `text-scale`, so controls
holding text grow with it. Older versions don't know `largest`; the lenient parser already
falls back to Default for unknown values (Settings Survive Version Changes).
