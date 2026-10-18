# Tasks: rename-to-clutch

## 1. Identity
- [x] 1.1 Executables `clutch.exe` and `clutch-browser.exe`; file properties; AppUserModelID `Clutch.Studio`
- [x] 1.2 Installer: product name, folder, shortcuts, MSI file name; same UpgradeCode

## 2. Migration
- [x] 2.1 Move `%APPDATA%\eStream` to `%APPDATA%\Clutch` at first start and rebase the last collection path; `CLUTCH_HOME` with `ESTREAM_HOME` fallback
- [x] 2.2 Browser cache folder under `%LOCALAPPDATA%\Clutch`
- [x] 2.3 CS2 config: write `gamestate_integration_clutch.cfg`, remove the eStream one
- [x] 2.4 Montage extension `.clutch-montage` (old extension still opens); log names `clutch-*.log` (old logs still pruned)

## 3. Text
- [x] 3.1 UI, toasts, remote-API and health messages say Clutch
- [x] 3.2 Virtual camera name "Clutch Virtual Camera"; default recordings folder and file name
- [x] 3.3 README

## 4. Check
- [ ] 4.1 Tests, clippy, release build and installer; upgrade over an eStream 0.2.0 install keeps settings and scenes
