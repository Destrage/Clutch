## ADDED Requirements

### Requirement: Supported systems
Clutch SHALL build and run on Windows 10 2004+ / Windows 11 and on Linux x86_64 with a Wayland
or X11 session. CI SHALL build, lint and test the workspace on both systems.

#### Scenario: CI covers both systems
- **WHEN** a change is pushed
- **THEN** formatting, clippy and the test suite run on Windows and Linux and all must pass

#### Scenario: App starts on Linux
- **WHEN** Clutch is started on a Linux desktop with a Vulkan or GL driver
- **THEN** the main window opens with a live preview and an empty scene can be recorded to a file

### Requirement: Capture on Linux
Display capture, window capture, camera, desktop audio and per-app audio SHALL be available on
Linux (ScreenCast portal over PipeWire, V4L2, PipeWire audio), using the same source kinds as on
Windows.

#### Scenario: Display capture through the portal
- **WHEN** the user adds a display capture source on Linux
- **THEN** the system screen picker opens, and after a choice the display appears in the scene

#### Scenario: Picker is not shown again
- **WHEN** a project with a Linux display capture source is reopened
- **THEN** the saved portal restore token is used and capture starts without the picker

#### Scenario: Camera
- **WHEN** the user adds a camera source on Linux
- **THEN** the camera list shows the connected V4L2 cameras and the chosen one appears in the
  scene

### Requirement: Missing features are explained
A feature or source kind that is not available on the current system SHALL be shown as
unavailable with a short reason, and SHALL NOT be dropped from the project or cause an error
when the project loads.

#### Scenario: Game capture opened on Linux
- **WHEN** a project made on Windows with a game capture source is opened on Linux
- **THEN** the project loads, the source is listed as "Not available on this system", and it is
  still there when the project is opened on Windows again

#### Scenario: Focus trigger on Wayland
- **WHEN** a macro uses a "window focused" trigger on a Wayland session
- **THEN** the macro editor shows that the trigger is unavailable on this system

### Requirement: Stream keys stored securely on Linux
On Linux, stream keys SHALL be stored through the Secret Service. When no Secret Service is
available the key SHALL NOT be written in plain text.

#### Scenario: Key survives a restart
- **WHEN** a stream key is saved with a Secret Service running and Clutch is restarted
- **THEN** the key is filled in again

#### Scenario: No keyring available
- **WHEN** no Secret Service is running
- **THEN** the key is kept for the session only and the Go Live box says it won't be remembered

### Requirement: Global hotkeys on Linux
Global hotkeys SHALL work on Linux through the GlobalShortcuts portal, or an X11 key grab on X11.
Where neither is available they SHALL work while Clutch has focus, and the hotkey settings SHALL
say so.

#### Scenario: Hotkey while another app is focused on X11
- **WHEN** a "Start recording" hotkey is set on an X11 session and pressed while another app is
  in front
- **THEN** recording starts

### Requirement: Virtual camera on Linux
On Linux the virtual camera SHALL write to a `v4l2loopback` device. When the module is not
loaded, starting the virtual camera SHALL fail with a message explaining how to load it.

#### Scenario: Module not loaded
- **WHEN** the user starts the virtual camera without `v4l2loopback`
- **THEN** the output reports that the `v4l2loopback` module is needed and how to load it

### Requirement: AppImage
`cargo xtask package` on Linux SHALL produce a single x86_64 AppImage that bundles FFmpeg and the
browser-source helper, includes a desktop entry and icon, and runs without installing anything
else beyond the desktop's own PipeWire, portals and GPU driver. Building from source with the
README's packages SHALL also work.

#### Scenario: AppImage runs on a clean system
- **WHEN** the AppImage is made executable and run on an Ubuntu 24.04 desktop with no FFmpeg
  installed
- **THEN** Clutch starts and can record

#### Scenario: Build from source
- **WHEN** a user installs the README's apt packages and the Rust toolchain and runs
  `cargo build --release`
- **THEN** the build succeeds and `target/release/clutch` starts
