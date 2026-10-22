# Tasks: ask-game-capture-audio

## 1. Default and setting
- [x] 1.1 Game capture "Capture audio" off by default (schema, validation, first-run preset);
      sources saved without the key keep their audio on
- [x] 1.2 `game_audio` in settings (ask, capture, skip), default ask; old settings files load

## 2. Asking
- [x] 2.1 Adding a Game capture with Desktop or Application audio present asks first; none
      present turns the audio on
- [x] 2.2 The question: Capture sound / Leave it off, with "Don't ask again" saving the answer
- [x] 2.3 Settings → Appearance → Sources: "Game capture sound" select

## 3. Check
- [ ] 3.1 Tests, clippy (done); in-app check of the three cases
