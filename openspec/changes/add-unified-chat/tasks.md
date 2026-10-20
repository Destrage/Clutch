# Tasks: add-unified-chat

## 1. Messages
- [x] 1.1 Common message type (platform, id, sender, colour, text, time) and a bounded buffer
      shared by the readers and the UI
- [x] 1.2 Kick reader: chatroom from the channel name, known feed keys tried in turn, subscribe,
      parse chat events, reconnect with backoff on refusal; tests on recorded events
- [x] 1.3 Twitch reader: anonymous chat connection (tags, PING, RECONNECT, /me); tests on
      recorded lines
- [x] 1.4 YouTube reader: live chat of the current broadcast through streamList, polling list as
      fallback; tests on recorded responses

## 2. Sending
- [x] 2.1 Twitch: add `user:write:chat` to the sign-in; send through Helix
- [x] 2.2 YouTube: sign-in scope `youtube.force-ssl`; send with liveChatMessages.insert
- [x] 2.3 Kick: Connect Kick (OAuth 2.1 + PKCE, loopback redirect, encrypted tokens, renewal);
      client ID and secret from build keys; send through the public chat API
- [x] 2.4 "All platforms" sends to each connected platform and reports the ones that failed

## 3. Panel
- [x] 3.1 Combined list with platform badges, names, colours, times; stays put while scrolled up
      with a "New messages" button
- [x] 3.2 Reply box with the platform picker and per-platform state
- [x] 3.3 Pages menu opening a platform's own chat page, and back to the list
- [x] 3.4 Status lines for platforms that can't be read (not connected, Kick feed down)

## 4. Docs and check
- [x] 4.1 Website chat page and privacy page
- [ ] 4.1b Testing guide scenarios
- [ ] 4.2 Tests; in-app check with live channels on each platform (read, reply to one, reply to all)
