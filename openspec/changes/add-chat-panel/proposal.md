# Change: Chat panel, viewer widget and connected accounts

## Why
Streamers want to read and answer chat, and see how many people are watching, without leaving
Clutch. Most platforms have no single official way to read chat without a server, but each one
has its own pop-out chat page. Showing that page is reliable, allowed, and keeps working when the
platform changes. Twitch and YouTube also offer official APIs for viewer numbers and (for YouTube,
whose sign-in is blocked in embedded browsers) sending messages, through a connected account.

## What Changes
- **Interactive browser panels:** the browser helper takes mouse, wheel and keyboard input and
  opens sign-in pop-ups in the same view; sign-ins persist (the existing browser profile).
- **Chat panel:** one tab per platform the user streams to (Kick, Twitch, YouTube), each showing
  the platform's own pop-out chat. Users sign in once inside the tab to chat as themselves.
  Separate tabs by design: each platform's chat stays as it is.
  - The channel comes from the destination (a "Channel" field) or the connected account. YouTube
    needs the live video, which comes from the connected account or a pasted link.
- **Viewer widget** in the top bar: total viewers while live, and a breakdown per platform when
  clicked (with "Active in chat" for Twitch when connected).
  - Kick: from Kick's public channel page (no sign-in).
  - Twitch and YouTube: from their official APIs, once the account is connected.
- **Connected accounts** (Settings → Stream, per destination), like other streaming apps:
  - Twitch: sign in with the device code flow (no secret); viewer count and chatters list.
  - YouTube: OAuth in the system browser; finds the live broadcast and its chat, viewer count,
    and a message box under the chat tab that sends through the API.
  - Tokens are stored encrypted (DPAPI), like stream keys.

## Impact
- Affected specs: chat (new).
- Affected code: `apps/estream-browser` (input, pop-ups), `estream-capture` browser host (input
  commands, panel pages), new `estream-chat` crate (accounts, viewer counts), `apps/estream` (chat
  panel, top-bar widget, settings).
- Needs developer apps registered by the project owner: a Twitch application (client ID) and a
  Google Cloud project with the YouTube Data API and an OAuth client. Without them, the chat tabs
  and Kick's viewer count still work.
