# Tasks: add-setup-checks

## 1. Checks
- [ ] 1.1 `checks.rs`: one device in two camera sources; audio device twice; app twice; Desktop
      audio on a chosen device; more than one app or capture sound with Desktop audio; the
      monitoring check moved in; tests per rule
- [ ] 1.2 Warnings in the Sources list, Inspector and mixer (separate from problems)
- [ ] 1.3 A toast for each new warning after an edit

## 2. Sources
- [ ] 2.1 Camera with no picture after a few seconds: "isn't sending pictures" with the cause
- [ ] 2.2 Virtual camera button disabled on Windows 10 with the reason on hover

## 3. Logs
- [ ] 3.1 Source status changes after start; camera open failures with their reason
- [ ] 3.2 Encoder, resolution, frame rate and bitrate per destination when going live
- [ ] 3.3 Game capture's choice in "Any fullscreen application" mode, and windows passed over

## 4. Check
- [ ] 4.1 Tests, clippy; in-app check of each warning
