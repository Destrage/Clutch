# Change: Share the stream and announce it on Discord

## Why
When streamers go live, they tell friends and their community where to watch. Today that means
finding each platform's channel link (and, for YouTube, the link of this broadcast), writing a
message, and pasting it into Discord and other apps by hand, while the stream is already running.

Clutch already knows what's needed: the Twitch and Kick channel names (set for the chat tab), the
YouTube watch link of the broadcast it created or chose in the Go live window, and the title and
category the streamer just set.

## What Changes
- **Share while live.** A Share button in the Live panel opens a popover with:
  - one watch link per platform being streamed to (Twitch, Kick, YouTube), each with Copy;
  - a ready-made message (by default `Live now: <title>` and the links), editable before
    copying, with Copy;
  - buttons that open a pre-filled post on X, Bluesky, Reddit and WhatsApp in the browser
    (their public share pages, so no accounts or API keys);
  - Post to Discord, when a Discord webhook is set up.
- **Discord announcements via webhooks.** In Settings → Stream → Sharing the user adds one or
  more Discord webhooks (name and webhook URL, from the server's channel settings →
  Integrations → Webhooks), with Send test. Each webhook has "Post when I go live" (on by
  default) and "Mention @everyone" (off by default).
  - Clutch posts once per stream, after the video is on the air, and doesn't post again for
    reconnects or for a restart within 10 minutes.
  - A failed post shows a toast and never affects the stream.
  - Webhook URLs are secrets: saved encrypted like stream keys, and never logged.
- **Message template.** One template in Settings → Stream → Sharing, used for the share popover
  and Discord, with `{title}`, `{category}`, `{links}`, `{twitch}`, `{youtube}` and `{kick}`.

## Not included
- A Discord bot. A bot needs a server running all the time and a bot token, and webhooks post to
  a channel just as well. Webhooks can only post to servers where the user (or an admin) can
  create one; direct messages to friends go through the copied message.
- Posting through platform accounts (X, Bluesky, Reddit APIs). The share pages need no sign-in.
- Watch links for custom servers and Facebook; there is no channel name to build one from.
- Editing the Discord post when the stream ends. This could come later: webhooks can edit
  their own messages.
- A macro action for posting; automation could reuse the same sender later.

## Impact
- Affected specs: `streaming`.
- Affected code: `apps/estream` (Live panel Share popover, Settings → Stream → Sharing, settings
  fields for the template and webhooks, a Discord webhook sender on a worker thread, posting on
  go-live).
- No new dependencies: posts use the app's existing `ureq` client.
