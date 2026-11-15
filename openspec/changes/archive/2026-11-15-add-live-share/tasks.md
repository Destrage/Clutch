# Tasks: add-live-share

## 1. Links and message
- [x] 1.1 Watch links per streamed destination (Twitch, Kick from the channel name; YouTube from
      the applied broadcast or the channel field), with the reason when one is missing
- [x] 1.2 Template filling with `{title}`, `{category}`, `{links}`, `{twitch}`, `{youtube}`,
      `{kick}`; empty lines from missing values dropped; unit tests
- [x] 1.3 Share page URLs (X, Bluesky, Reddit, WhatsApp) with URL encoding; unit tests

## 2. Settings
- [x] 2.1 `share_template` and `discord_webhooks` (name, protected URL, post on go-live, mention
      everyone) in settings; old settings files load unchanged
- [x] 2.2 Settings → Stream → Sharing: template editor listing the placeholders (no separate
      preview: Send test and the Share window show the filled message), webhook list with add,
      remove, the two toggles and Send test; URL check on entry

## 3. Discord
- [x] 3.1 Webhook sender (`ureq` on a worker thread): `allowed_mentions`, 10 s timeout, one 429 retry,
      error messages per status; URL never logged; tests against a local HTTP server
- [x] 3.2 Announce once per stream session when the first destination is live; skip reconnects
      and new sessions within 10 minutes of the last announcement to that webhook

## 4. Share window
- [x] 4.1 Share button (Live deck, Live panel, palette) and Share window: links with Copy,
      editable message with Copy, share page buttons, Post to Discord (per webhook when there are several)

## 5. Verification
- [x] 5.1 `cargo test` and `cargo clippy --all-targets` for the app
- [x] 5.2 In the app: go live to Twitch and YouTube with a test webhook; check the post arrives (checked 2026-11-15: test post, go-live post, share text)
      once, the links open the right pages, and a reconnect doesn't post again
