## Context
The AppImage stages everything under `usr/bin` with an `$ORIGIN` rpath (FFmpeg, CEF), so the
same tree works from any folder. Clutch writes a hidden (`NoDisplay`) desktop entry in
`~/.local/share/applications` because the portal registry needs one; it already skips writing
when a system entry exists, but an entry left by an earlier AppImage run would stay and, being in
the user's data dir, take precedence over `/usr/share/applications` and hide the app.

## Decisions
- **Tool: nfpm**, pinned and downloaded into `third_party/nfpm` like `appimagetool`. One config
  builds both formats without `rpmbuild` or distro containers, so the WSL/CI build stays one
  command. Alternatives: `dpkg-deb` + `rpmbuild` (two code paths, `rpmbuild` absent on Ubuntu),
  `cargo-deb`/`cargo-generate-rpm` (built around one binary, not a staged tree with CEF).
- **Layout:**
  - `/opt/clutch` holds the AppImage's `usr/bin` contents unchanged (rpath keeps working);
  - `/usr/bin/clutch` is a symlink to `/opt/clutch/clutch`;
  - the desktop entry, icon, metainfo and docs go under `/usr/share`.

  `/opt` because the bundled FFmpeg and CEF are private copies that must not mix with the
  system's libraries.
- **Dependencies:** found from the staged binaries.
  - `ldd` lists the shared libraries that aren't bundled.
  - .deb: the list is mapped to package names, kept in the xtask with `a | b` alternatives for
    the 24.04 `t64` renames.
  - .rpm: plain soname requirements (`libasound.so.2()(64bit)`), which dnf and zypper resolve on
    any RPM distro.
  - `v4l2loopback-dkms` (deb) / `v4l2loopback` (rpm) are Suggests/Recommends.
- **Marker:** the package writes `/opt/clutch/package` with `deb` or `rpm`. The updater reads it
  next to the executable to know how it was installed. A missing marker means AppImage or a
  source build.
- **Updates:**
  - The updater picks the release asset matching the marker (`.deb`, `.rpm`, else `.AppImage`)
    and checks its sha256 as today.
  - After exit it runs `pkexec apt-get install -y <file>` (or `dnf install -y`,
    `zypper --non-interactive install --allow-unsigned-rpm`), then restarts. If `pkexec` is
    missing or cancelled, the update stays downloaded and the message says how to install it
    by hand.
- **Desktop entry cleanup:** at start, if a system entry for the app id exists and the user
  entry carries `X-Clutch-Generated`, the user entry is deleted.
- **CEF sandbox:** runs without the setuid sandbox, as in the AppImage. Making `chrome-sandbox`
  setuid root in `/opt` is possible later, but it is a security-sensitive change of its own.

## Risks
- **Dependency names drift between distro releases.** Mitigation: alternatives in the .deb list,
  and soname requirements in the .rpm. A clean-container install test runs on Ubuntu 24.04,
  Debian 13 and Fedora before each release.
- **Unsigned packages.** apt installs a local .deb without a signature. dnf needs `--nogpgcheck`
  for a local unsigned rpm only when gpgcheck is forced for local files; the docs give the
  command.
