## ADDED Requirements

### Requirement: Setup warnings while editing
The app SHALL check the scene collection after every change and show a warning on each source
involved in a setup problem, in the Sources list, the Inspector and the mixer, without waiting for
a recording or stream. Muted sources SHALL NOT count. When an edit adds a new warning, the app
SHALL show it once as a toast.

#### Scenario: Two sources on one camera
- **WHEN** the user adds a camera source for a camera another camera source already uses
- **THEN** the new source shows a warning suggesting to add the existing source to the scene
  instead, and a toast says so

#### Scenario: Same app captured twice
- **WHEN** two unmuted Application audio sources capture the same app
- **THEN** both show that the app is heard twice

#### Scenario: Desktop audio can't leave the app out
- **WHEN** Desktop audio uses a chosen output device and an Application audio source is present
- **THEN** the Application audio source shows that its app is also in Desktop audio

#### Scenario: Muting resolves it
- **WHEN** the user mutes one of the sources involved
- **THEN** the warning goes away

### Requirement: A camera that sends nothing explains why
A camera source that has not delivered a picture within a few seconds of starting SHALL say that
the camera isn't sending pictures and name the likely cause, instead of staying on "Starting".

#### Scenario: Camera held by another app
- **WHEN** another app holds the webcam on Windows 10 and the user adds a camera source for it
- **THEN** within a few seconds the source says the camera isn't sending pictures and that
  another app may be using it

### Requirement: Unsupported features are disabled up front
Features the system can't run SHALL be disabled with the reason shown, rather than failing each
time they are used.

#### Scenario: Virtual camera on Windows 10
- **WHEN** the app runs on Windows 10
- **THEN** the virtual camera button is disabled and its tooltip says it needs Windows 11

### Requirement: Logs explain source and stream problems
The log SHALL record a source's status changes after start (once per change), why a camera failed
to open, each destination's encoder, resolution, frame rate and bitrate when going live, and which
window Game capture picks in "Any fullscreen application" mode.

#### Scenario: Camera fails to open
- **WHEN** a camera can't be opened
- **THEN** the log has one line with the camera and the reason, not one per retry
