# Change: Updates inside the app

## Why
Every new version means finding the site, downloading a 220 MB installer and running it. Most
people won't, so testers and users stay on old builds. Clutch should notice new versions itself
and install them, and users should decide how much it does on its own.

## What Changes
- **updates** capability (new).
- **Where updates come from:** the releases of the public releases repo on GitHub (the same ones
  the website offers). Clutch reads the newest release for its platform: the `.msi` on Windows,
  the `.AppImage` on Linux.
- **Settings → Updates** (a new page):
  - **Updates:** *Install automatically* (default), *Tell me when one is out*, or *Off*.
  - **Include beta versions:** on by default while the installed version is itself a beta.
  - **Check now**, the installed version, and when Clutch last checked.
- **Checking:** at startup and every 6 hours while Clutch runs (not when Updates is Off).
  Failures are quiet (a line on the Updates page), never a pop-up.
- **Downloading:** in the background, at low priority, into a cache folder. The file's
  SHA-256 must match the checksum GitHub publishes for it, or it's thrown away.
- **Installing:** never while recording, streaming or the replay buffer is saving, and never
  by surprise:
  - *Install automatically:* the update downloads, then installs **when you close Clutch**.
    The top bar shows "Update ready", which can also **Restart now**.
  - *Tell me when one is out:* a notice says which version is out, with **What's new**,
    **Install** and **Later**.
  - **Windows:** Clutch closes and runs the installer with a progress window (Windows asks
    for permission, since Clutch is installed for all users), then starts again.
  - **Linux:** the new AppImage replaces the old one in place (next to it, then renamed over
    it), and Clutch offers to restart.
- **Versions:** the app's version carries its beta number (`0.3.5-beta.4`), so betas compare
  correctly; the Windows installer still gets the numbers only (`0.3.5`), as Windows needs.
- **Privacy:** checking sends one request to GitHub's public API; nothing about the user.
  The privacy page says so.

## Impact
- Affected specs: updates (new), studio-ui (Settings → Updates page, top-bar notice).
- Affected code: new `apps/estream/src/update.rs` (check, compare, download, verify, install),
  `settings.rs` (update settings), `app.rs` (timer, notice, install on exit), `overlays.slint`
  (Updates page), `xtask` (installer version from the numeric part), website privacy page at
  release.
