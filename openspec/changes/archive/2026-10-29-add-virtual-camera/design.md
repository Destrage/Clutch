# Design: add-virtual-camera

## Processes

```
eStream.exe                                    svchost (Frame Server, LOCAL SERVICE)
  engine → NV12 canvas frames                    estream_vcam.dll
  vcam feed thread ── scale/letterbox ──▶ Global\estream-vcam-v1 ◀── MediaStream::RequestSample
  IMFVirtualCamera (session lifetime)            (creates the mapping; reads frames)
                                                    │
                                     Zoom / Teams / Chrome ◀── frames via Frame Server
```

- **The DLL creates the shared mapping** in the `Global\` namespace. Services hold
  `SeCreateGlobalPrivilege`; standard users don't.
  - Its DACL grants SYSTEM, LOCAL SERVICE, Administrators and interactive users full access, so
    eStream can open it without elevation.
  - The mapping exists only while the source is active, i.e. while some app opened the camera.
- **eStream opens the mapping** about every 500 ms while the camera is on:
  - When the consumer heartbeat is fresh (under 1.5 s), it attaches a video consumer to the
    canvas and starts writing frames.
  - When the heartbeat goes stale (over 3 s), it detaches. GPU readback runs only while the
    camera is in use.

## Shared memory layout (`estream_vcam::shared`)

| Offset | Field | Writer |
|---|---|---|
| 0 | magic `EVC1`, version 1 (u32 each) | DLL |
| 8 | wanted width, height (u32) | DLL (from the negotiated media type) |
| 16 | consumer heartbeat, `GetTickCount64` ms (u64) | DLL, every RequestSample |
| 24 | seq (u64, seqlock: odd while writing) | eStream |
| 32 | frame width, height (u32) | eStream |
| 40 | producer heartbeat ms (u64) | eStream |
| 4096 | NV12 frame (up to 1920×1080) | eStream |

- `GetTickCount64` is system-wide, so both processes compare heartbeats directly.
- The DLL copies a frame only when `seq` is even and unchanged across the copy, retrying a few
  times. Otherwise it reuses its last good frame.
- If the producer heartbeat is older than 2 s, the DLL shows a dark placeholder.
- Pure helpers are unit tested:
  - `fit_nv12(src, sw, sh, dst, dw, dh)` — bilinear scaling, letterboxed with black (Y 16,
    UV 128), with a memcpy fast path when the sizes match;
  - the seqlock writer and reader.

## COM objects (`estream_vcam::source`, `#[implement]`)
- **`ClassFactory`** creates an `Activator`.
  - `DllGetClassObject` serves CLSID `{8A5F2E61-3C4B-4D7E-9B1A-6E2D0C7F5A93}`.
  - `DllCanUnloadNow` returns `S_OK` once no activator, source or stream is alive.
  - `DllRegisterServer` (elevated) **installs a copy** and registers that, not the build output.
    1. It creates `%ProgramData%\eStream\VirtualCamera` with the DACL "SYSTEM and Administrators
       full, LOCAL SERVICE modify, Users read".
    2. It copies itself there as `estream_vcam-<version>p<protocol>-<hash>.dll` and removes older
       copies that aren't locked.
    3. It writes `HKLM\SOFTWARE\Classes\CLSID\{…}\InprocServer32` (the copy's path, ThreadingModel
       Both).

    Frame Server keeps the DLL it loaded open, so registering the build output would lock
    `target\…\estream_vcam.dll` (and an installed eStream's own file) against rebuilds and
    updates.
  - `DllUnregisterServer` removes the CLSID key.
  - **Diagnostics:** the source appends lifecycle lines and errors to `vcam.log` in the same
    folder, which Frame Server may write. It also sends them to `OutputDebugString`.
  - **Name in apps:** Windows lists the camera as "eStream Virtual Camera (Windows Virtual
    Camera)".
- **`Activator`** (`IMFActivate`):
  - `IMFAttributes` calls are forwarded to an inner store;
  - `ActivateObject` creates the media source once and queries it for the requested interface;
  - `ShutdownObject` shuts the source down.
- **`MediaSource`** (`IMFMediaSourceEx`, `IMFGetService`, `IKsControl`, `IMFSampleAllocatorControl`):
  - **Descriptors:** one stream. The stream descriptor carries three NV12 media types plus these
    attributes:
    - `MF_DEVICESTREAM_STREAM_CATEGORY = PINNAME_VIDEO_CAPTURE`;
    - `MF_DEVICESTREAM_STREAM_ID = 0`;
    - `MF_DEVICESTREAM_FRAMESERVER_SHARED = 1`;
    - `MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES = Color`.
  - `Start` queues `MENewStream`/`MEUpdatedStream`, then `MEStreamStarted` (on the stream), then
    `MESourceStarted`. `Stop` queues `MEStreamStopped` and `MESourceStopped`.
  - `Pause` is unsupported, as it is for live sources.
  - `IKsControl` answers "not found" (`ERROR_SET_NOT_FOUND`).
  - `SetD3DManager` is accepted and ignored, so samples stay in system memory.
- **`MediaStream`** (`IMFMediaStream2`, `IKsControl`) handles `RequestSample`:
  1. allocate a sample (the frame server's allocator when one was provided, initialised for the
     current type; otherwise `MFCreate2DMediaBuffer`);
  2. lock it with `IMF2DBuffer2::Lock2DSize`;
  3. write NV12 using the buffer pitch;
  4. set the time (`MFGetSystemTime`) and duration, and attach the token;
  5. queue `MEMediaSample`.
- **Threading:** all state sits behind `Mutex`es. The objects are free-threaded, as
  ThreadingModel Both requires.

## Control (`estream_vcam::control`, used by eStream)
- `registration() -> Registration { Installed, Missing, Elsewhere(path) }` reports `Installed` when
  the registered copy exists and carries this build's tag (eStream version plus shared memory
  protocol version).
  - Rebuilds within a version keep working without another UAC prompt.
  - To load changed source code during development, use Settings → Virtual camera → Reinstall.
- `install(dll) -> Result` runs `regsvr32 /s "<dll>"` through `ShellExecuteExW("runas")` (the UAC
  prompt) and waits for the exit code.
- `MFCreateVirtualCamera` is resolved at run time from `mfsensorgroup.dll`, so eStream still starts
  on Windows 10 and reports "needs Windows 11" there instead.
- Media types declare BT.709 limited range (matrix, range, primaries, transfer), matching the
  engine's NV12.
- `VirtualCamera::create(name)` calls `MFCreateVirtualCamera` with:
  - `MFVirtualCameraType_SoftwareCameraSource`;
  - `MFVirtualCameraLifetime_Session`;
  - `MFVirtualCameraAccess_CurrentUser`;
  - source id = the CLSID string.

  It then calls `Start(None)`; dropping it calls `Remove`. It runs on the feed thread (MTA).

## Output (`estream_media::vcam::VirtualCamOutput`)
- `start(engine, canvas)` spawns the feed thread. That thread:
  1. initialises COM;
  2. creates the camera and reports start errors back;
  3. loops: open the mapping → check heartbeats → attach or detach the consumer → scale and
     write frames.
- `status()` returns Off, Starting, On (idle), InUse or Failed(msg).
- `stop()` ends the thread, which removes the camera.

## App
- **Top bar:** a camera button (on = accent, in use = live dot) and the palette action "Start/Stop
  virtual camera".
- **First start:**
  1. if the source isn't installed, a toast explains why the UAC prompt is coming, and
     `install()` runs on a helper thread;
  2. on success the camera starts; on refusal, a toast says so.
- **Settings → Virtual camera:** canvas picker, install state, "Reinstall" button.
- **Remote API:** `Output::VirtualCam` in `Studio`; `GetVirtualCamStatus`/Start/Stop/Toggle and
  `VirtualcamStateChanged`.

## Testing
- Unit tests: `fit_nv12` (sizes, letterbox colour, fast path) and the seqlock round trip.
- In-process COM test: build the media source directly (rlib) with a `Local\` mapping name.
  - Start it with the first presentation descriptor and request samples.
  - Check the NV12 size and placeholder colour.
  - Write a frame from the producer side; the next sample carries it.
  - Check the event order (`MENewStream`, `MESourceStarted`, `MEMediaSample`).
- System test (needs the one-time UAC approval):
  1. start the camera from eStream;
  2. open "eStream Virtual Camera" with an `IMFSourceReader` in a test program;
  3. read samples, and check they match the canvas (a red colour source).
- In-app: start the camera, add eStream's own camera source showing "eStream Virtual Camera"
  (the infinite mirror), and check the in-use state in the top bar.
