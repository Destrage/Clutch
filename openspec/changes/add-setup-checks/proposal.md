# Change: Setup checks while editing

## Why
A Windows 10 user called Clutch "so buggy", and the logs showed problems that were visible in
the setup long before he went live, but that nothing pointed out:
- The first-run Webcam never started for 12 minutes, with no reason on screen or in the log, so
  he added a second camera source for the same webcam. The two then fought over it.
- He clicked the virtual camera button 12 times on Windows 10 and got the same toast each time.
- Desktop audio, Application audio and capture audio can hear the same app twice in some
  combinations, and only the monitoring case says so today.

Problems that can be seen from the setup should show up while editing it, where the user can fix
them, and the logs should say why a source isn't working.

## What Changes
- **Setup checks:** worked out from the scene collection after every change, and shown on the
  source while editing (Sources list, Inspector, mixer strip). When an edit adds a new one (adding
  a source, changing its device), a toast says it once. The checks:
  - **One device, two sources:** two camera sources on the same device. The second shows the
    same picture as the first; the advice is to add the existing source to the scene instead.
  - **Heard twice:**
    - two audio sources on the same input or output device;
    - two Application audio sources on the same app;
    - Desktop audio on a chosen device (not the default) together with Application audio or
      capture audio: it can't leave those apps out;
    - Desktop audio can leave out one app at a time: with more than one Application audio or
      capture audio source, the rest are heard twice.
  - Muted sources don't count.
- **A camera that never starts says so:** a camera that hasn't sent a picture within a few seconds
  shows "The camera isn't sending pictures" with the likely cause (another app using it; Windows
  10 lets one app use a camera at a time) instead of "Starting camera…".
- **Virtual camera on Windows 10:** the button is disabled, with the reason on hover.
- **Logs for support:**
  - a source's status change after start (once per change, not per retry);
  - camera open failures with their reason;
  - each destination's encoder, resolution, frame rate and bitrate when going live;
  - the window Game capture's "Any fullscreen application" mode picks, and once per window the
    fullscreen-looking windows it passes over and why.

## Impact
- Affected specs: setup-checks (new).
- Affected code: `apps/estream` (checks, toasts, rows, Inspector, mixer, virtual camera button,
  go-live log), `estream-capture` (camera start timeout and failure logs, Game capture choice
  log), `estream-engine` (status change log).
