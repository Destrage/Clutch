# branding Specification

## Purpose
How the app names and presents itself (Clutch, formerly eStream), and how an install of
eStream becomes Clutch without the user setting anything up again.

## Requirements

### Requirement: Product name
The app SHALL present itself as "Clutch" everywhere a user can see its name: the window title, UI
text, executables, installer, shortcuts, the virtual camera, and messages sent to remote-API
clients.

#### Scenario: Installed app
- **WHEN** the user installs the app and opens the Start menu
- **THEN** a "Clutch" shortcut with the Faceted C icon is listed and launches `clutch.exe`

#### Scenario: Virtual camera
- **WHEN** the virtual camera is on
- **THEN** video call apps list it as "Clutch Virtual Camera"

### Requirement: Upgrade from eStream
Installing Clutch over eStream SHALL replace the eStream install, and the first start SHALL move
the user's settings and scene collections from the eStream settings folder to the Clutch one, so
that nothing has to be set up again.

#### Scenario: First start after upgrading
- **WHEN** `%APPDATA%\eStream` exists and `%APPDATA%\Clutch` does not
- **THEN** the folder is moved to `%APPDATA%\Clutch`
- **AND** the last scene collection that was open in eStream opens in Clutch

#### Scenario: Both folders exist
- **WHEN** `%APPDATA%\Clutch` already exists
- **THEN** nothing is moved and `%APPDATA%\eStream` is left as it is

#### Scenario: Old montage files
- **WHEN** the montage folder holds `.estream-montage` files
- **THEN** they are listed and open like `.clutch-montage` files
