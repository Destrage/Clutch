# Change: Game capture stability and GPU starvation protection

## Why
When running GPU-intensive games (such as Valorant) on mid-range hardware (like a GTX 1650 Ti
on Windows 10), users encounter severe rendering lag, clip saving timeouts, and GPU device
loss crashes (`0x887A0005 DXGI_ERROR_DEVICE_REMOVED`). This happens because Clutch runs at
default priority on Windows, so the OS graphics scheduler starves Clutch's DX12 compositor
whenever an uncapped game claims 100% of the GPU.

Additionally, Game capture in "Any fullscreen application" mode repeatedly polls the foreground
window. When the user alt-tabs or the game transitions between match and menus, capture drops
or attaches to unintended background windows.

## What Changes
- **Windows process priority:** On Windows, Clutch elevates its process priority to Above Normal
  at startup. This ensures the Windows GPU and DWM scheduler maintain compositing intervals
  even when a fullscreen game runs at 100% GPU utilization.
- **Game capture latching in auto mode:** Once "Any fullscreen application" mode acquires a game
  window, it latches onto that window until the window is destroyed or the process exits,
  rather than dropping or switching targets whenever foreground focus changes.
- **Device loss diagnostics:** When DXGI device removal occurs, log the specific reason via
  `GetDeviceRemovedReason` to provide clear support diagnostic information.

## Impact
- Affected specs: `video-sources`.
- Affected code: `apps/estream` (Windows process priority), `estream-capture` (Game capture
  target latching, interop resilience), `estream-gpu` (device loss diagnostics).
