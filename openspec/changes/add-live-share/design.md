# Design: add-live-share

## Watch links
| Platform | Link | Source |
|---|---|---|
| Twitch | `https://www.twitch.tv/<name>` | the destination's channel name (`chat::channel_name`) |
| Kick | `https://kick.com/<name>` | the destination's channel name |
| YouTube | `https://www.youtube.com/watch?v=<id>` | the broadcast applied in the Go live window, else a live link in the destination's channel field |

A platform with no link (no channel name, or YouTube without a broadcast) is left out of the
links, and the Share window says where to add it, as the chat tab does. With several destinations on
the same platform, each gets its own line, labeled with the destination's name.

## Message
The template is plain text with placeholders, filled when the Share window opens or a post is sent:
- `{title}`: the title applied in the Go live window (the first platform's, in destination
  order), else empty.
- `{category}`: the category or game applied, else empty.
- `{links}`: one link per line, prefixed with the platform when there is more than one.
- `{twitch}`, `{youtube}`, `{kick}`: that platform's link, else empty.

Default: `Live now: {title}\n{links}`. Lines left empty by missing values are dropped, so a stream
without a title doesn't post a bare "Live now: ".

## Share pages
Opened with the system browser, text URL-encoded:
- X: `https://x.com/intent/post?text=`
- Bluesky: `https://bsky.app/intent/compose?text=`
- Reddit: `https://www.reddit.com/submit?url=<first link>&title=<title>`
- WhatsApp: `https://wa.me/?text=`

## Discord webhooks
- Accepted URLs: `https://(discord.com|discordapp.com|ptb.discord.com|canary.discord.com)/api/webhooks/<id>/<token>`.
  Anything else is refused when it's entered.
- Stored like stream keys: `secret::protect` in settings, decrypted in memory, never in logs (log
  lines name the webhook by its label).
- Request: `POST <url>?wait=true` with `{"content": <message>, "allowed_mentions": {"parse": []}}`,
  or `{"parse": ["everyone"]}` and `@everyone` before the message when Mention @everyone is on.
  `allowed_mentions` stops a title containing `@everyone` from pinging anyone it shouldn't.
- Sent with the app's `ureq` client on a worker thread (as the Go live window's platform calls
  are), with a 10 s timeout; the result comes back to the UI thread as a message. A 429 waits `retry_after` once (up to 10 s),
  then gives up. Errors become a toast naming the webhook: "Couldn't post to Discord (<name>): …".
  A 401/404 says the webhook was deleted and asks for a new URL.

## When to announce
- Trigger: the first destination of a stream session reaches "live" (connected and sending; for
  YouTube with the broadcast started). Later destinations in the same session don't post again.
  If YouTube is the only destination and never reports its broadcast as started, the post goes
  out 90 s after the video started flowing, so a missed status check doesn't skip it.
- Reconnects within the session don't post. A new session within 10 minutes of the last
  announcement to the same webhook doesn't post either (covers a crash or a quick restart); the
  Share window's Post to Discord still works for a manual post.
- The time of the last announcement per webhook is kept in memory only; a restart of Clutch
  forgets it, which is fine for the 10-minute window.
