# Change: Rename eStream to Clutch

## Why
"eStream" is generic and hard to search for. The app now centres on clipping and montages, and
"Clutch" fits that better. The new Faceted C logo is designed around the name.

## What Changes
- **branding** capability (new): the product is called Clutch everywhere a user can see it:
  - the window title, welcome and help text, toasts, and remote-API messages;
  - the executables (`clutch.exe`, `clutch-browser.exe`) and their file properties;
  - the installer, the install folder, the Start menu and desktop shortcuts, and Apps & features;
  - the virtual camera ("Clutch Virtual Camera");
  - log files, the CS2 game-state config file, the montage file extension (`.clutch-montage`),
    and the default recordings folder (`Videos\Clutch`) for new users.
- **Identity:** a new AppUserModelID (`Clutch.Studio`). The installer keeps its UpgradeCode, so
  installing Clutch upgrades and replaces an existing eStream install.
- **Migration (one time, at first start):**
  - The settings folder `%APPDATA%\eStream` is moved to `%APPDATA%\Clutch`, and the saved
    "last scene collection" path is pointed at the new folder.
  - The browser helper's cache moves the same way.
  - An existing CS2 config (`gamestate_integration_estream.cfg`) is replaced by the Clutch one.
  - Old `.estream-montage` files still open, and old `estream-*.log` files are still cleaned up.
  - `ESTREAM_HOME` is still honoured when `CLUTCH_HOME` is not set.
- **Not changed:** the workspace crate names and folders (`estream-*`), the camera DLL's file
  name and CLSID, and the remote-API custom fields, which are internal or used by tools.

## Impact
- Affected specs: branding (new). The "eStream Virtual Camera" name in `add-virtual-camera` is
  superseded by this change.
- Affected code: `apps/estream` (main, settings, secret, UI text), `apps/estream-browser`,
  `estream-capture`, `estream-clips` (GSI), `estream-engine` (logs), `estream-media` (defaults),
  `estream-montage` (extension), `estream-vcam` (name), `estream-api` and `estream-health`
  (messages), `xtask` (installer).
- Users who pinned eStream to Start or the taskbar need to pin Clutch again, because the app ID
  changes.
