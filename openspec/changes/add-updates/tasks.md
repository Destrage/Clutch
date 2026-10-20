# Tasks: add-updates

## 1. Versions
- [ ] 1.1 Version with its beta number in Cargo.toml; the installer uses the numeric part
- [ ] 1.2 Version parsing and comparison (betas before the release), with tests

## 2. Updater
- [ ] 2.1 Check: newest matching release and its asset for this platform, beta filter, tests
- [ ] 2.2 Download in the background to a cache folder; verify SHA-256 against GitHub's digest
- [ ] 2.3 Install: Windows runs the installer after Clutch exits and restarts it; Linux replaces
      the AppImage and offers a restart
- [ ] 2.4 Never while recording, streaming or saving a clip

## 3. Studio
- [ ] 3.1 Update settings (mode, include betas, last check) with defaults
- [ ] 3.2 Settings → Updates page: mode, betas, Check now, version, last check, status
- [ ] 3.3 Checks at startup and every 6 hours; "Update ready" in the top bar with Restart now;
      the notice for Tell me (What's new, Install, Later)
- [ ] 3.4 Install on exit in automatic mode

## 4. Check
- [ ] 4.1 Tests; an end-to-end update from one local build to a newer one
