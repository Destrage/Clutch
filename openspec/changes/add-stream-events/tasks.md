# Tasks: add-stream-events

## 1. Events in the chat
- [ ] 1.1 Feed lines can be events (highlighted, text built per event type); panel row style
- [ ] 1.2 Chime on events with a bell toggle in the chat header, kept in settings

## 2. Twitch
- [ ] 2.1 Sign-in adds `moderator:read:followers channel:read:subscriptions bits:read`
- [ ] 2.2 EventSub WebSocket reader for the connected channel (welcome, subscribe the six types,
      keepalive, reconnect message, backoff); tests on recorded notifications

## 3. YouTube
- [ ] 3.1 Super Chat, Super Sticker, new member, milestone and gifted memberships become event
      lines; tests on recorded items

## 4. Kick relay
- [ ] 4.1 Worker: `/kick/webhook` with signature check (public key fetched and cached), Durable
      Object per channel, `/kick/live` WebSocket with token check against Kick; tests
- [ ] 4.2 wrangler config for `events.destrosphere.com`; deploy steps in the service README
- [ ] 4.3 Clutch: `events:subscribe` in the Kick sign-in; subscribe the five event types
      (skip existing); relay connection with reconnect and backoff; event lines; tests

## 5. Docs and check
- [ ] 5.1 Website chat page (events), privacy page (relay), testing guide scenarios
- [ ] 5.2 In-app check: Twitch follow, YouTube Super Chat (or recorded), Kick follow through the
      deployed relay, forged webhook rejected
