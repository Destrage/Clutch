# Change: Correct colors when capturing an HDR screen

## Why
Testers with Windows HDR turned on report that their captured screen and games look too bright
and washed out in the preview, recordings and streams. Every Windows capture asks Windows
Graphics Capture for 8-bit BGRA. On an HDR display, Windows produces that by clipping its scRGB
desktop. HDR games and Auto HDR games have highlights well above SDR white, so the clipping
blows them out. SDR content comes out at the wrong brightness whenever the "SDR content
brightness" slider is not at its default.

## What Changes
- When the captured monitor (or the monitor a captured window is on) has HDR active, display,
  window and game capture ask Windows for 16-bit float scRGB frames instead of 8-bit BGRA.
- A GPU pass maps those frames to SDR before they reach the canvas:
  - it divides by the monitor's SDR white level (read from Windows), so SDR content keeps the
    brightness the user sees on their own screen;
  - it compresses anything brighter than SDR white into the SDR range with a soft shoulder, up
    to the monitor's peak brightness, so HDR highlights keep detail instead of clipping;
  - it clamps colors outside the BT.709 gamut.
- Captures follow changes: a window moved to another monitor, or HDR switched on or off,
  recreates the frame pool in the matching format.
- Each capture source gets an **HDR screens** option: **Automatic** (default, the mapping above)
  or **As Windows gives them** (today's 8-bit path), in case the mapping looks wrong on a
  particular setup.
- Startup and capture logs record whether HDR was detected, the SDR white level and the peak
  brightness, so tester reports can be diagnosed.

## Out of scope
- HDR output (HDR10 recordings or streams). The canvas stays SDR.
- macOS and Linux HDR capture. They keep their current path and can follow in their own changes.

## Impact
- Affected specs: `video-sources` (ADDED: HDR screen capture)
- Affected code: `crates/estream-capture/src/windows/{wgc,d3d,capture,enumerate}.rs`, a new
  tone-map pass in `crates/estream-gpu`, and the capture inspector options in `apps/estream`
- Platform: Windows. Branch `windows/fix-hdr-capture`, commits `fix(windows): …`
