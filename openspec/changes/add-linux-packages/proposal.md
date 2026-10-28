# Change: Native Linux packages (.deb and .rpm)

## Why
On Linux, Clutch only ships as an AppImage. An AppImage is a file you run, not an installed app:
- it doesn't appear in the app menu or search;
- it has no uninstaller;
- the system's package manager doesn't know about it.

Users expect to double-click a package or run `apt install` / `dnf install` and get an app like
any other. Snap and Flatpak are not wanted here: the browser source's sandbox doesn't fit them,
and users asked for the native way.

## What Changes
- **Packages:** `cargo xtask package` on Linux also builds `Clutch-<version>-amd64.deb` (Ubuntu,
  Debian, Mint, Pop!_OS) and `Clutch-<version>-x86_64.rpm` (Fedora, openSUSE). They come from the
  same files as the AppImage.
- **Installed like any app:**
  - the files go in `/opt/clutch`;
  - a `clutch` command goes in `/usr/bin`;
  - the menu entry, icon and AppStream info go in `/usr/share`, so Clutch shows in the app menu,
    search and software centres;
  - removing the package removes all of it. User settings in `~/.config/Clutch` stay.
- **Dependencies are declared:**
  - the .deb lists the Debian/Ubuntu packages Clutch needs (with the `t64` names where Ubuntu
    24.04 renamed them);
  - the .rpm requires the shared libraries by name, so it resolves on any RPM distro.

  `apt`/`dnf` then fetch whatever is missing. `v4l2loopback` (virtual camera) is suggested, not
  required.
- **No hidden entry shadowing the real one:** Clutch's own hidden desktop entry, written for the
  screen-sharing portal when run as an AppImage, is removed once a system entry exists.
  Otherwise an earlier AppImage run would hide the installed app from the menu.
- **Updates:**
  - an installed package downloads the matching .deb or .rpm and installs it through the
    system's password prompt (`pkexec` with `apt`, `dnf` or `zypper`);
  - the AppImage keeps replacing itself as before.
- **Release and docs:** releases carry all three Linux files; the docs' Linux page and the
  testing guide explain which one to pick.

## Out of scope
- Snap, Flatpak and the AUR.
- An apt or dnf repository (so updates come through `apt upgrade`). Possible later: it needs a
  signing key and hosting.
- arm64 builds.

## Impact
- Affected specs: `platform-support`
- Affected code:
  - `xtask` (packaging);
  - `apps/estream/src/main.rs` (desktop entry);
  - `apps/estream/src/update.rs` (package updates);
  - README.
