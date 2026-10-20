# Tasks: add-unified-chat

## 1. Messages
- [ ] 1.1 Common message type (platform, id, sender, colour, text, time) and a bounded buffer
      shared by the readers and the UI
- [ ] 1.2 Kick reader: chatroom from the channel name, feed key looked up with fallback, subscribe,
      parse chat events, reconnect with backoff on refusal; tests on recorded events
- [ ] 1.3 Twitch reader: EventSub WebSocket with a connected account (welcome, subscribe,
      keepalive, reconnect); anonymous read-only chat without one; tests on recorded events
- [ ] 1.4 YouTube reader: live chat of the current broadcast through streamList, polling list as
      fallback; tests on recorded responses

## 2. Sending
- [ ] 2.1 Twitch: add `user:read:chat user:write:chat` to the sign-in; send through Helix
- [ ] 2.2 YouTube: sign-in scope `youtube.force-ssl`; send with liveChatMessages.insert
- [ ] 2.3 Kick: Connect Kick (OAuth 2.1 + PKCE, loopback redirect, encrypted tokens, renewal);
      client ID and secret from build keys; send through the public chat API
- [ ] 2.4 "All platforms" sends to each connected platform and reports the ones that failed

## 3. Panel
- [ ] 3.1 Combined list with platform badges, names, colours, times; stays put while scrolled up
      with a "New messages" button
- [ ] 3.2 Reply box with the platform picker and per-platform state
- [ ] 3.3 Pages menu opening a platform's own chat page, and back to the list
- [ ] 3.4 Status lines for platforms that can't be read (not connected, Kick feed down)

## 4. Docs and check
- [ ] 4.1 Website chat page, privacy page (Kick), testing guide scenarios
- [ ] 4.2 Tests; in-app check with live channels on each platform (read, reply to one, reply to all)
