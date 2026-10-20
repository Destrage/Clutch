# Change: One chat for every platform

## Why
The chat panel shows one tab per platform, each with that platform's own chat page. Streamers on
two or three platforms at once have to keep switching tabs and miss messages on the others. They
asked for everything in one list, and to answer from there, the way multistream chat tools work.

## What Changes
- **Combined chat** becomes the chat panel: one list with messages from Twitch, YouTube and Kick
  as they arrive, each with a small platform badge, the sender's name (in their colour where the
  platform gives one) and the time. Emotes show as their names for now. Unread messages are kept
  while the panel is scrolled up, with a "New messages" button to jump down.
- **Reply box** under the list: send to all platforms at once or pick one. A platform that
  can't take the message is named with the reason ("Connect Kick in Settings → Stream to reply
  there").
- **Platform pages stay** one click away (a "Pages" menu in the panel header) for things the
  combined list doesn't do: moderation, polls, signing in to the platform's own chat.
- **Reading chat:**
  - Twitch: Twitch's chat connection signed in anonymously, which reads any channel without an
    account or extra permission (replying uses the connected account).
  - YouTube: with a connected account, the live chat of the current broadcast through
    `liveChatMessages.streamList` over HTTPS (`liveChat/messages/stream`, one long-lived call
    that streams a JSON array), falling back to `liveChatMessages.list` at the interval YouTube
    asks for (at least 5 s).
  - Kick: Kick's public chat feed (the realtime feed its own website uses). This is unofficial:
    Kick rotates the feed's app key, so the known keys are tried in turn and the one that
    works is remembered; refusals back off and retry, and a clear message shows when none works. The chatroom
    comes from the channel name.
- **Sending:**
  - Twitch: Helix `POST chat/messages` (`user:write:chat`, added to the Twitch sign-in;
    connected users connect again once).
  - YouTube: `liveChatMessages.insert`. The YouTube sign-in moves from `youtube.readonly` to
    `youtube.force-ssl` (which also covers reading); connected users connect again once.
  - Kick: new **Connect Kick** in Settings → Stream (OAuth 2.1 with PKCE in the system browser,
    loopback redirect), then `POST /public/v1/chat` (`chat:write`).
- Facebook Live keeps no chat (its API needs Meta's app review).
- Moderation (delete, timeout, ban) is out of scope.

## Impact
- Affected specs: chat (combined chat and replying added; per-platform tabs become pages).
- Affected code: `apps/estream` (`chat.rs` and new readers per platform, `twitch.rs`,
  `youtube.rs`, new `kick.rs` account, chat panel UI, settings), build keys.
- Needs from the project owner:
  - A Kick developer app (client ID and secret, redirect `http://localhost:<port>/callback`).
  - In Google Auth Platform → Data Access, add `youtube.force-ssl`; it is a sensitive scope, so it
    goes into the YouTube verification review (until then: the unverified-app warning and the
    100-user cap).
  - YouTube quota: the default is 10,000 units a day for the whole project. Reading costs 5 units
    per call and sending 20, so a quota increase will be needed once many people stream with it.
- Risk: the Kick feed is not an official API; it may stop working or change without notice, and
  using it may go against Kick's terms. The owner chose it knowingly; Kick sending uses the
  official API.
