## 1. Storage
- [ ] 1.1 Details per setup and destination in settings, keyed by the setup's file; tests that old
      settings files load unchanged
- [ ] 1.2 First start: copy each destination's remembered details into the open setup
- [ ] 1.3 Setup rename keeps the details, delete removes them, new and duplicate start empty

## 2. Go live window
- [ ] 2.1 Pre-fill order: the open setup's details, then the channel as it is now, then the
      destination's last details (Twitch, Kick); YouTube's new broadcast from the setup's details
- [ ] 2.2 Save button: store in the open setup and close, no platform calls
- [ ] 2.3 Offline "Stream details" (in the live panel where Edit details is while live, and the
      Edit stream details action) opens the window with Save only
- [ ] 2.4 Going live stores the applied details in the open setup

## 3. Starts without the window
- [ ] 3.1 Hotkey, macro, remote and "Go live without asking" use the open setup's details
- [ ] 3.2 Switching setups while live: Edit details shows the new setup's details; nothing is sent
      until Save

## 4. Docs and check
- [ ] 4.1 Website streaming page: details per setup, Save, Stream details while offline (written in
      clutch-site, not pushed until the release); testing guide
- [ ] 4.2 In-app check:
  - two setups with different Twitch and Kick categories and YouTube descriptions, switched back
    and forth;
  - Save while offline, then Go live;
  - a hotkey go-live in each setup;
  - a new setup falling back to the channel's details;
  - rename and delete a setup.
