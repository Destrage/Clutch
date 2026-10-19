# Tasks: add-recordings-view

## 1. Library
- [x] 1.1 `Library::open_flat`: scans only the folder's own video files
- [x] 1.2 `Recording::paths`: every file the recording is writing

## 2. App
- [x] 2.1 A recordings library on the recording folder; rescanned with the clips, reopened when
      the folder changes; the recording in progress is left out
- [x] 2.2 Background details for recordings without them: length, thumbnail, marker events
- [x] 2.3 "Recordings" in the Show filter; the actions use the shown library; delete also removes
      the marker file; "Save as new clip" goes to the clip library

## 3. UI
- [x] 3.1 Show filter option, empty state and summary for recordings; game filter hidden

## 4. Check
- [ ] 4.1 Tests for the flat scan; clippy; in-app check of the list, details, play, trim to a clip
