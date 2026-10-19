# Change: Recordings in the Clips view

## Why
The Clips view lists only the clip library (the `Clips` folder). Full recordings are saved one
level up, in the recording folder, so after a session there is no way to find, play or cut them
inside Clutch; the only way in is "Show last recording in folder", which opens the file manager.

## What Changes
- The Clips view's **Show** filter gets a **Recordings** choice. It lists the video files saved
  directly in the recording folder (not the `Clips` or `Montages` subfolders), newest first, with
  thumbnail, length and date. The recording being written right now is left out.
- A recording without details gets them in the background, one at a time: its length, a
  thumbnail, and the game events from its marker file. They are saved next to it like a clip's
  (`.json` and `.jpg`), so this happens once.
- The clip actions work on recordings: play, rename, favorite, show in folder, copy, delete (also
  removes its marker file) and trim. **Save as new clip** cuts the chosen part into the clip
  library with the events inside it; **Replace** trims the recording in place.
- The game filter is hidden while recordings are shown (recordings aren't sorted by game).

## Impact
- Affected specs: clips (new requirement).
- Affected code: `estream-clips` (`Library` can scan only a folder's own files),
  `estream-media` (`Recording` lists the files it is writing), `apps/estream/src` (a recordings
  library next to the clip library, background details, the actions), `apps/estream/ui/clips.slint`.
