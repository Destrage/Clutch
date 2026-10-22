# Change: Game capture asks before capturing the game's sound

## Why
Game capture turns on its own audio by default. A user who already hears the game through
Desktop audio or an Application audio source gets a second fader for the same sound without
asking for it, and has to find the "Capture audio" switch to undo that. They should decide.

## What Changes
- **Off by default:** a new Game capture starts with "Capture audio" off, including the one in the
  "Streaming games" first-run scene (Desktop audio there already carries the game).
  Window capture is unchanged, and existing sources keep their setting.
- **Asking:** adding a Game capture while Desktop audio or an Application audio source is in the
  collection asks "Capture the game's sound too?" (Capture sound / Leave it off), with a
  "Don't ask again" box. With no such source, the capture's audio turns on without asking, so
  the game is never silent.
- **Remembered answer:** ticking "Don't ask again" saves the button pressed, and later Game
  captures follow it. Settings → Appearance → Sources has "Game capture sound"
  (Ask / Always capture / Leave off) to change or reset it.

## Impact
- Affected specs: video-sources (the Game capture audio default from add-capture-audio).
- Affected code: `estream-capture` (Game capture schema default), `apps/estream` (adding a
  source, the question, settings, first-run preset).
