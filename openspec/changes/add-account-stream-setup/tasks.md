# Tasks: add-account-stream-setup

## 1. Stream details from the account
- [ ] 1.1 YouTube: pick the stream (bound to the current/upcoming broadcast, else default) and read
      its address and key; tests on recorded responses
- [ ] 1.2 Twitch: `channel:read:stream_key` in the sign-in; Helix streams/key
- [ ] 1.3 Kick: `streamkey:read` in the sign-in; channels → stream url and key
- [ ] 1.4 Settings → Stream: Get stream key on a connected destination; fills, saves, toasts

## 2. YouTube live status
- [ ] 2.1 Status from liveStreams + liveBroadcasts for the key being streamed; tests for each state
- [ ] 2.2 Checker while streaming (20 s, 2 min once live); Live deck row text and colour; toast on
      change
- [ ] 2.3 Start broadcast (transition to live) from the toast

## 3. Docs and check
- [ ] 3.1 Website streaming page; testing guide
- [ ] 3.2 In-app check on YouTube with auto-start off and on, and Get stream key on each platform
