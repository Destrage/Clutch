## ADDED Requirements

### Requirement: Native Linux packages
`cargo xtask package` on Linux SHALL also produce an x86_64 .deb and .rpm. They install Clutch in
`/opt/clutch` with a `clutch` command, a desktop entry, an icon and AppStream metadata, and
declare the system libraries they need. Removing the package SHALL remove every file it
installed.

#### Scenario: Install on Ubuntu
- **WHEN** a user installs `Clutch-<version>-amd64.deb` on Ubuntu 24.04 with `apt install` or the
  software centre
- **THEN** missing libraries are installed with it, Clutch appears in the app menu and search,
  and `clutch` starts it

#### Scenario: Install on Fedora
- **WHEN** a user installs `Clutch-<version>-x86_64.rpm` with `dnf install`
- **THEN** its library requirements resolve, and Clutch appears in the app menu

#### Scenario: Earlier AppImage run
- **WHEN** a user who ran the AppImage before installs the package
- **THEN** the hidden entry Clutch wrote is removed on the next start, and the menu shows the
  installed app

### Requirement: Updating a package install
An installed package SHALL update to the same package format. The update SHALL be installed
through the system's administrator prompt. If that isn't possible, Clutch SHALL keep the
downloaded file and say how to install it.

#### Scenario: Update on Fedora
- **WHEN** a newer release is out and the user restarts to update a Clutch installed from the
  .rpm
- **THEN** Clutch downloads the .rpm, asks for the administrator password, installs it and
  restarts
