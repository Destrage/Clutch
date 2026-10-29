## ADDED Requirements

### Requirement: macOS support
Clutch SHALL build and run on macOS 13 or later on Apple Silicon (arm64), using Metal for
rendering. CI SHALL build, lint and test the workspace on macOS next to Windows and Linux.
Adding macOS SHALL NOT change behaviour on Windows or Linux.

#### Scenario: CI covers macOS
- **WHEN** a pull request is opened
- **THEN** formatting, clippy and the test suite run on Windows, Linux and macOS, and all must
  pass

#### Scenario: App starts on a Mac
- **WHEN** Clutch is started on an Apple Silicon Mac with macOS 13 or later
- **THEN** the main window opens with a live preview, and an empty scene can be recorded to a
  file

### Requirement: Capture on macOS
Display capture, window capture, desktop audio and per-app audio SHALL be available on macOS
through ScreenCaptureKit. Cameras SHALL be available through AVFoundation and microphones
through CoreAudio. They SHALL use the same source kinds as on Windows. Game capture SHALL be
shown as not available on this system.

#### Scenario: Display capture
- **WHEN** the user adds a display capture source on a Mac with Screen Recording permission
- **THEN** the chosen display appears in the scene

#### Scenario: Per-app audio
- **WHEN** the user adds an application audio source for a running app
- **THEN** only that app's audio reaches the mixer, and Clutch's own output is not captured

#### Scenario: Project from Windows
- **WHEN** a project made on Windows with a game capture source is opened on a Mac
- **THEN** the project loads, the source is listed as "Not available on this system", and it is
  still there when the project is opened on Windows again

### Requirement: macOS privacy permissions
Clutch SHALL ask for Screen Recording, Camera and Microphone permission when a source first
needs it. When a permission is denied, the source SHALL show what is blocked and offer to open
the matching page in System Settings. The app bundle SHALL include the usage descriptions that
macOS requires.

#### Scenario: Screen Recording denied
- **WHEN** the user adds a display capture source and Screen Recording permission is denied
- **THEN** the source shows that Screen Recording permission is needed, with a button that opens
  System Settings at Privacy & Security → Screen Recording

### Requirement: Stream keys stored in the Keychain
On macOS, stream keys SHALL be stored in the user's Keychain and SHALL NOT be written to the
settings file in plain text.

#### Scenario: Key survives a restart
- **WHEN** a stream key is saved on a Mac and Clutch is restarted
- **THEN** the key is filled in again from the Keychain

### Requirement: Hardware encoding on macOS
On macOS, the VideoToolbox H.264 and HEVC encoders SHALL be offered, and VideoToolbox H.264
SHALL be the default. Encoders that don't exist on macOS (NVENC, AMF, QuickSync) SHALL NOT be
listed.

#### Scenario: Record with VideoToolbox
- **WHEN** the user records 1080p60 on an Apple Silicon Mac with the default encoder
- **THEN** the file is H.264 encoded by VideoToolbox, with audio in sync and no dropped frames

### Requirement: Global hotkeys on macOS
Global hotkeys SHALL work on macOS while another app is in front, without asking for
Accessibility or Input Monitoring permission. A key combination that macOS refuses SHALL be
reported in the hotkey editor.

#### Scenario: Hotkey while another app is focused
- **WHEN** a "Start recording" hotkey is set on a Mac and pressed while another app is in front
- **THEN** recording starts

#### Scenario: Refused combination
- **WHEN** the user assigns a hotkey whose only modifier is Option
- **THEN** the hotkey editor says that macOS doesn't allow this combination as a global hotkey

### Requirement: Virtual camera on macOS
On macOS, the virtual camera SHALL be a CoreMediaIO Camera Extension inside the app bundle.
While the extension is not available in the build, starting the virtual camera SHALL fail with
a message saying that it isn't available on this Mac yet.

#### Scenario: Camera extension active
- **WHEN** the virtual camera is started from an installed, signed `Clutch.app`
- **THEN** other apps list "Clutch Virtual Camera" and show the program output

#### Scenario: Not available in this build
- **WHEN** the user starts the virtual camera in a build without the camera extension
- **THEN** the output reports that the virtual camera isn't available on this Mac yet

### Requirement: macOS app and disk image
`cargo xtask package` on macOS SHALL produce `Clutch.app` and `Clutch-<version>-arm64.dmg`. The
app SHALL bundle FFmpeg and the browser-source helper with its CEF framework, and SHALL run
without Homebrew or any other installed libraries.

#### Scenario: Clean Mac
- **WHEN** the disk image is opened on a Mac without Homebrew or FFmpeg, and Clutch is dragged
  to Applications and started
- **THEN** Clutch starts, records and streams, and browser sources render

### Requirement: Updates on macOS
When a newer release is available, Clutch on macOS SHALL download its `-arm64.dmg`, verify the
published digest, replace the app and relaunch. When the app can't be replaced, it SHALL open
the release page instead.

#### Scenario: Update installs
- **WHEN** the user accepts an update on a Mac where `Clutch.app` is writable
- **THEN** the new version replaces the app and starts

#### Scenario: Digest mismatch
- **WHEN** the downloaded disk image doesn't match the published digest
- **THEN** the update is not installed and the user is told the download was damaged
