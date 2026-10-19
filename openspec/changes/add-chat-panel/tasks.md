# Tasks: add-chat-panel

## 1. Interactive browser
- [ ] 1.1 Helper: mouse, wheel, keyboard and focus commands; pop-ups open in the same view
- [ ] 1.2 Host: panel pages (not sources) with input, shown in the UI

## 2. Chat panel
- [ ] 2.1 "Channel" field per destination; pop-out chat URLs for Kick, Twitch and YouTube
- [ ] 2.2 Chat panel with a tab per platform; sign-in inside the tab

## 3. Viewer widget
- [ ] 3.1 `estream-chat` crate: viewer counts (Kick public channel page)
- [ ] 3.2 Top-bar widget with total and per-platform breakdown

## 4. Connected accounts
- [ ] 4.1 Twitch: device code sign-in, viewer count, chatters ("Active in chat")
- [ ] 4.2 YouTube: OAuth sign-in, live chat lookup, viewer count, message box
- [ ] 4.3 Settings → Stream: Connect / Disconnect per destination; encrypted tokens

## 5. Check
- [ ] 5.1 Tests, clippy; in-app check while live on each platform
