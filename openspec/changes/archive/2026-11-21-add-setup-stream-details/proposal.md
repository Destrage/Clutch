# Change: Stream details for each setup

## Why
Streamers keep one setup per game: an Apex setup and a PUBG setup, each with its own scenes. The
stream details belong to the game too: the title ("Apex with friends"), the category on Twitch and
Kick, and the YouTube title and description. Today the details are remembered per destination
only, so:
- Switching from the Apex setup to the PUBG setup keeps Apex's details, and the user types PUBG's
  title, category and description again on every platform.
- Switching back to Apex means typing Apex's details again.
- Twitch and Kick pre-fill from the channel as it is now, which is still the last game streamed.
- The Go live window has no Save: details can only be kept by going live with them.

## What Changes
- **Details per setup.** Each setup keeps its own go-live details for each destination (YouTube
  title, description, privacy, category and the rest; Twitch title, category, tags, language; Kick
  title, category). Going live or saving stores them in the setup that is open.
- **Pre-fill from the setup.** The Go live window fills each section from the open setup's saved
  details. Only when the setup has none for that destination does it fall back to today's order
  (the channel as it is now, then the destination's last details). YouTube still lists its
  waiting broadcasts; a new broadcast is filled from the setup's details.
- **Save without going live.** The Go live window gets **Save**, next to Go live: it stores the
  details in the open setup and closes, without changing anything on the platforms. While
  offline, the live panel shows **Stream details** where it shows Edit details while live, and the
  existing "Edit stream details" action works too; both open the same window with Save only.
- **Hotkey, macro and remote starts** use the open setup's details, as "Go live without asking"
  does.
- **YouTube's waiting broadcast.** When a broadcast is already waiting on the key (YouTube Studio
  makes one, copied from the last stream), it gets the setup's title, description, privacy,
  category and made-for-kids; its latency and auto-start stay as they are.
- **While live** Edit details shows the channels as they are, as before; saving there also keeps
  the details in the open setup. Setups can't be switched while streaming, so no other case
  arises.
- **Setup changes.** A renamed setup keeps its details; a deleted setup's details are removed; a
  duplicated or new setup starts with none (and so falls back as above).
- **Existing users.** Nothing to move: a setup without its own details fills in exactly as Clutch
  does today, so the first Go live after updating looks the same as before.

## Out of scope
- Changing the platform details automatically on a setup switch while offline (nothing is sent
  until Go live).
- Per-setup destinations (which destinations are enabled stays app-wide).
- Details templates shared between setups.

## Impact
- Affected specs: streaming (remembered details from `add-go-live-details` become per setup).
- Affected code: `apps/estream`:
  - `settings.rs`: details per setup and destination, migration from `DestinationConfig.details`;
  - `golive.rs`: pre-fill order, a save-only mode;
  - the Go live window (Slint): Save button, offline "Stream details…";
  - `app.rs`: storing on Go live and Save, setup rename and delete, hotkey and remote starts.
- No new platform permissions.
