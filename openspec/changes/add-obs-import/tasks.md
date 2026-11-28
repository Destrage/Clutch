# Tasks: add-obs-import

## 1. Parsing and mapping
- [ ] 1.1 Locate OBS config folders per platform; list scene collections and profiles
- [ ] 1.2 Parse scene collection JSON into an intermediate model (fixtures from real OBS 29–31 exports)
- [ ] 1.3 Map scenes, items, transforms, groups and nested scenes
- [ ] 1.4 Map sources by kind; match devices and windows by name; placeholders for the rest
- [ ] 1.5 Map audio settings and the supported filters
- [ ] 1.6 Parse the profile: canvas, recording, encoder, bitrate, stream service; optional stream key
- [ ] 1.7 Import report (imported, changed, skipped with reasons)

## 2. App
- [ ] 2.1 Create a new setup from the import; name collisions; blocked while live or recording
- [ ] 2.2 Import dialog: found collections/profiles, Choose file…, Import my stream key
- [ ] 2.3 Entry points: first-run wizard link, setup menu, command palette
- [ ] 2.4 Report dialog after import

## 3. Check
- [ ] 3.1 Tests per mapping; import real collections on Windows, macOS and Linux
