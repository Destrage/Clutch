# Tasks: add-linux-packages

## 1. Packaging (`xtask`)
- [x] 1.1 Shared staging for the AppImage and the packages; `nfpm` pinned download
- [x] 1.2 .deb and .rpm: `/opt/clutch`, `/usr/bin/clutch`, desktop entry, icon, metainfo, docs,
      install marker
- [x] 1.3 Dependencies: .deb package list (with `t64` alternatives), .rpm soname requirements,
      v4l2loopback suggested

## 2. App
- [x] 2.1 Remove Clutch's hidden user desktop entry once a system entry exists
- [x] 2.2 Updater: asset by install kind (marker), install through `pkexec` with apt/dnf/zypper,
      hand-install message when that fails

## 3. Docs and release
- [x] 3.1 README and site Linux page: which file to pick, install and remove commands
- [x] 3.2 Testing guide: install, menu, update and remove scenarios

## 4. Verification
- [x] 4.1 Unit tests (asset choice per install kind, dependency lists)
- [x] 4.2 Clean-container installs: Ubuntu 24.04 and Debian 13 (.deb), Fedora (.rpm); `clutch`
      found, libraries resolve, removal leaves nothing
- [ ] 4.3 On a Linux desktop (user): installs from the file manager, shows in the menu and search,
      starts, updates, uninstalls
