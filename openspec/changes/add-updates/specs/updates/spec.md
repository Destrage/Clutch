## ADDED Requirements

### Requirement: Update checks
Unless updates are Off, the studio SHALL check the public releases for a newer version than the
installed one at startup and every 6 hours, skipping beta versions unless beta versions are
included. A failed check SHALL NOT interrupt the user.

#### Scenario: A newer version is out
- **WHEN** a release with a higher version than the installed one exists for this platform
- **THEN** the studio knows about it and shows it on the Updates page

#### Scenario: Betas left out
- **WHEN** "Include beta versions" is off and the newest release is a beta
- **THEN** that release is ignored

### Requirement: Update modes
The user SHALL choose between installing updates automatically, being told about them, or no
checks at all. Automatic updates SHALL download in the background and install only when Clutch
closes or the user chooses to restart.

#### Scenario: Install automatically
- **WHEN** the mode is Install automatically and an update finishes downloading
- **THEN** the top bar shows "Update ready", and the update installs when Clutch closes

#### Scenario: Tell me
- **WHEN** the mode is Tell me when one is out and an update is found
- **THEN** a notice names the version with What's new, Install and Later, and nothing is
  downloaded until Install is chosen

#### Scenario: Off
- **WHEN** the mode is Off
- **THEN** no request is made unless the user clicks Check now

### Requirement: Safe installs
An update SHALL install only after its download matches the SHA-256 checksum published for it,
and never while recording or streaming.

#### Scenario: Corrupt download
- **WHEN** a downloaded file's checksum doesn't match
- **THEN** the file is deleted, nothing installs, and the Updates page says the download failed

#### Scenario: Closing while live
- **WHEN** an update is ready and the user stops streaming and closes Clutch
- **THEN** the update installs after Clutch has closed, not during the stream
