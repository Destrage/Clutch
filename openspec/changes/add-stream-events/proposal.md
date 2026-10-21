# Change: Follows, subs and other stream events in the chat

## Why
Streamers want to notice who followed, subscribed, gifted or raided while they're live, on every
platform they stream to, without watching each platform's dashboard. Chat alone doesn't show these.

## What Changes
- **Events in the combined chat:** follows, subs, resubs, gifted subs, cheers and raids (Twitch);
  follows, subs, resubs, gifted subs and Kicks (Kick); Super Chats, Super Stickers, new members,
  milestones and gifted memberships (YouTube) appear in the chat list as highlighted lines with the
  platform badge ("ali followed", "sam subscribed for 3 months", "jo gifted 5 subs"), with an
  optional chime (a bell button in the chat header turns it off; remembered).
- **Twitch:** EventSub over WebSocket with the connected account (official, no server): it
  subscribes to `channel.follow` (v2), `channel.subscribe`, `channel.subscription.message`,
  `channel.subscription.gift`, `channel.cheer` and `channel.raid` for the account's own channel.
  The sign-in adds `moderator:read:followers channel:read:subscriptions bits:read`.
- **YouTube:** the special message kinds already in the live chat feed (`superChatEvent`,
  `superStickerEvent`, `newSponsorEvent`, `memberMilestoneChatEvent`, `membershipGiftingEvent`)
  become event lines. New subscribers aren't available live from YouTube.
- **Kick:** Kick delivers events only by webhook to one public address per app, so a small relay
  is added:
  - **Relay** (`services/events-relay`): a Cloudflare Worker on `events.destrosphere.com` with a
    Durable Object per Kick channel. `POST /kick/webhook` verifies Kick's signature (RSA PKCS#1
    v1.5 / SHA-256 over `message-id.timestamp.body`, with Kick's public key fetched and cached,
    not hardcoded) and passes the event to the channel's open connections. `GET /kick/live` is
    a WebSocket for Clutch: the request carries the streamer's Kick access token (Authorization
    header), which the relay checks with Kick before accepting, to learn the channel, so a connection only ever gets its own channel's
    events. Nothing is stored; events with no listener are dropped.
  - **Sign-in tokens (Kick, Google):** `POST /kick/token`, `/kick/refresh`, `/google/token` and
    `/google/refresh` swap Clutch's sign-in codes and refresh tokens for tokens, adding the app's
    secret, which lives only in the relay (Worker secrets); neither secret is built into Clutch.
    Changing one is a new secret on the platform, then `wrangler secret put`.
  - Only Clutch can use the relay: sign-ins must come from Clutch's Kick app (token
    introspection), webhooks must belong to the app's own subscriptions, and connection and
    token attempts are rate-limited per address.
  - Clutch's service addresses (relay, update repository) have one default each and can be set
    at build or run time (`CLUTCH_EVENTS_URL`, `CLUTCH_RELEASES_REPO`).
  - **Clutch:** with Kick connected, subscribes the channel to `channel.followed`,
    `channel.subscription.new`, `channel.subscription.renewal`, `channel.subscription.gifts` and
    `kicks.gifted` (skipping ones it already has), and keeps a connection to the relay with
    reconnect and backoff. The Kick sign-in adds `events:subscribe`.
- Raids/hosts on Kick and Facebook events are not available; the Alerts source (events shown on
  the stream itself) is a separate change.

## Impact
- Affected specs: chat (stream events).
- Affected code: `apps/estream` (`livechat` events for each platform, chat panel event rows and
  chime, `twitch.rs`/`kick.rs` scopes), new `services/events-relay` (Worker, wrangler config).
- Needs from the project owner:
  - Deploy the relay: a Cloudflare login for `wrangler`, the `events.destrosphere.com` route.
  - On the Kick app: tick "Subscribe to events", turn on webhooks with
    `https://events.destrosphere.com/kick/webhook`.
  - Twitch and Kick users connect again once (new permissions).
- Runs online: the relay is the one part of Clutch that isn't on the user's PC. On Cloudflare's
  free tier at beta scale; Kick allows 10,000 subscriptions per event type per app.
- Privacy: the relay sees which Kick account follows or subscribes to which channel, only passes
  it on, and keeps nothing.
