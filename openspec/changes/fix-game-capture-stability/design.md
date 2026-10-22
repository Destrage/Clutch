# Design: Game capture stability and GPU starvation protection

## Windows Process Priority
On Windows startup, call `SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS)`.
This gives Clutch's threads and GPU submissions priority over normal background work and
prevents the Windows graphics scheduler from starving the compositor when games run uncapped.
`ABOVE_NORMAL_PRIORITY_CLASS` does not require Administrator privileges.

## Target Latching in Game Capture
Currently `enumerate::foreground_fullscreen()` is queried on each attempt when `mode == "auto"`.
When a target is found, `CaptureSource` records the target window handle (`HWND`) and process ID.
While that process is still running and the window remains valid (`IsWindow`), `CaptureSource`
does not search for a new foreground window even if the game loses focus, is minimized, or
the user alt-tabs.

When `item.Closed()` fires or `IsWindow` returns false, the latch is cleared, allowing `auto`
mode to acquire the next foreground game.

## DXGI Device Removal Diagnostics
When DXGI operations fail with `DXGI_ERROR_DEVICE_REMOVED`, query `GetDeviceRemovedReason` on
the D3D12 device and log the translated HRESULT (e.g., `DXGI_ERROR_DEVICE_HUNG`,
`DXGI_ERROR_DEVICE_RESET`) to help diagnose driver crashes and TDR timeouts.
