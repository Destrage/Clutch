## 1. Storage
- [x] 1.1 Details per setup on each destination (`setup_details`, by setup name); tests that old
      settings files load unchanged
- [x] 1.2 No migration needed: a setup without its own details fills in as before
- [x] 1.3 Setup rename keeps the details, delete removes them, new and duplicate start empty

## 2. Go live window
- [x] 2.1 Pre-fill order: the open setup's details, then the channel as it is now, then the
      destination's last details (Twitch, Kick); YouTube's new broadcast from the setup's details,
      and a waiting broadcast gets the setup's title, description, privacy and category
- [x] 2.2 Save button: store in the open setup and close, no platform calls
- [x] 2.3 Offline "Stream details" (in the live panel where Edit details is while live, and the
      Edit stream details action) opens the window with Save only
- [x] 2.4 Going live (and Save while live) stores the applied details in the open setup

## 3. Starts without the window
- [x] 3.1 Hotkey, macro, remote and "Go live without asking" use the open setup's details
- [x] 3.2 Setups can't be switched while streaming (already enforced), so there's no live case

## 4. Docs and check
- [x] 4.1 Website streaming page: details per setup, Save, Stream details while offline (written in
      clutch-site, not pushed until the release); testing guide
- [ ] 4.2 In-app check:
  - two setups with different Twitch and Kick categories and YouTube descriptions, switched back
    and forth;
  - Save while offline, then Go live;
  - a hotkey go-live in each setup;
  - a new setup falling back to the channel's details;
  - a YouTube broadcast waiting on the key from the last stream;
  - rename and delete a setup.
