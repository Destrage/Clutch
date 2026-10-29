# Change: Virtual camera (Windows 11, Media Foundation)

## Why
People use their stream layout in calls and in other apps: Zoom, Teams, Discord, Google Meet in
Chrome or Edge, and recording tools. eStream needs
a virtual camera for that.

## What Changes
- **virtual-camera** capability (new): "eStream Virtual Camera" appears as a webcam in every app
  that uses Media Foundation or DirectShow, while the virtual camera is on.
  - It is built on Windows 11's `MFCreateVirtualCamera`.
  - It's a session camera: it disappears when it's stopped or eStream exits.
  - Windows' camera service (Frame Server) hosts the camera source, so a crashing app can't take
    eStream down, and eStream can't take a call down.
- **Camera source DLL** (`estream_vcam.dll`, new crate `estream-vcam`):
  - A COM Media Foundation media source (activator, source, stream). It offers NV12 at
    1920×1080, 1280×720 and 640×360, 30 fps.
  - It reads frames from shared memory written by eStream.
  - When eStream isn't sending frames, it shows a dark placeholder.
- **Output:**
  - eStream sends the chosen canvas (main by default) to the camera, scaled and letterboxed to the
    size the app asked for.
  - It renders and copies frames only while some app is actually using the camera.
- **Installation:** the camera source must be registered for all users once. This needs one UAC
  prompt, which appears the first time the virtual camera is started (or from Settings). After
  that, starting and stopping needs no admin rights.
- **UI and control:**
  - a Virtual camera button in the top bar with on/in-use state;
  - a command palette action;
  - a "Virtual camera" section in Settings: canvas, install state, and a reinstall button;
  - remote API: `GetVirtualCamStatus`, `StartVirtualCam`, `StopVirtualCam`, `ToggleVirtualCam`,
    and the `VirtualcamStateChanged` event.

## Out of scope
- Windows 10, macOS and Linux virtual cameras.
- Other pixel formats (apps convert NV12) and frame rates other than 30 fps.
- A separate "preview only" output or per-camera scene choice beyond the canvas.

## Impact
- Affected specs: `virtual-camera` (new), `remote-api` (virtual camera requests now work)
- Affected code:
  - new `crates/estream-vcam` (cdylib + rlib);
  - `estream-media` (camera feed output);
  - `apps/estream` (button, settings, registration);
  - `estream-api` (virtual camera output).
