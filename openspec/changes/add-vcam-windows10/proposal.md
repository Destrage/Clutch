# Change: Virtual camera on Windows 10 (DirectShow)

## Why
The virtual camera is built on Windows 11's `MFCreateVirtualCamera`, so on Windows 10 Clutch
says "needs Windows 11" and the camera can't be used. Many streamers and testers are still on
Windows 10, and call apps there (Zoom, Discord, Teams, browsers) find cameras through
DirectShow.

## What Changes
- `estream_vcam.dll` also contains a DirectShow video source filter, "Clutch Virtual Camera",
  registered under the video input device category. It reads frames from the same shared
  memory protocol as the Media Foundation source.
- The filter is registered on Windows 10 only. On Windows 11 the Media Foundation camera is used
  as before, and registering both would list the camera twice. Registration always clears an
  earlier DirectShow registration first, so a PC upgraded to Windows 11 drops it on the next
  install.
- The filter runs inside the app using the camera, as the user, who can't create `Global\`
  objects. Its shared memory is session-local (`Local\estream-vcam-dshow-v1`), and Clutch writes
  to that name when the Media Foundation camera isn't available.
- On Windows 10 the camera is listed in apps whenever it's installed. While Clutch isn't sending,
  it shows the same placeholder picture as the Media Foundation source.

## Impact
- Affected specs: `virtual-camera`.
- Affected code: `estream-vcam` (`dshow.rs`, `dll.rs`, `control.rs`, `shared.rs`),
  `estream-media` (`vcam.rs`: which shared memory to write).
