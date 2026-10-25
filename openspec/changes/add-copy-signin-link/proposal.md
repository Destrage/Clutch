# Change: Copy the sign-in link when connecting an account

## Why
Connecting YouTube or Kick opens the sign-in page in the default browser. Streamers who use
several browsers are often signed in to their platform account in a different one, so the page
opens where they aren't signed in. They need the link itself to open in a browser of their
choice.

## What Changes
- Settings → Stream, on a destination whose account isn't connected:
  - YouTube and Kick: a **Copy link** button next to **Connect**. It starts the same sign-in
    without opening a browser and puts the sign-in link on the clipboard. Clutch then waits for
    the sign-in exactly as after Connect.
  - While any connect is waiting (YouTube, Kick, or Twitch's code page): **Copy link** next to
    **Open page**, copying the page being waited on.
- A toast confirms the copy ("Sign-in link copied: open it in any browser").
- The link works in any browser on this PC: YouTube and Kick hand the result back to Clutch on a
  local address, and Twitch's page only needs the code shown in Clutch.

## Impact
- Affected specs: `chat` (Connected accounts).
- Affected code: `apps/estream/src/shell.rs` (text on the clipboard), `apps/estream/src/app.rs`
  (account actions), `apps/estream/ui/overlays.slint` (account row).
