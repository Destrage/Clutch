# Change: Music source (Creative Commons music from Jamendo)

## Why
Streamers need background music that won't get their streams muted or struck. Finding it,
checking licenses and keeping a playlist going is work Clutch can do. Jamendo offers a large
catalog of independent music under Creative Commons licenses, with a public API for apps.
A tester built a browser page to do this by hand; it should be a source type.

## What Changes
- **music-source** capability (new): a `music` source type.
  - Pick a **genre or mood** (electronic, lo-fi, chill, rock, hip-hop, jazz, classical,
    ambient, acoustic, cinematic) or type your own tags.
  - It fetches a playlist from Jamendo and plays it through the same player as media files,
    into the mix with its own knob, filters and tracks. Shuffle, **Skip**, **Previous**,
    play/pause, **Repeat this track**, and an **Up next** list (the playing track and the next
    14) where a click plays a track. Jamendo sometimes answers a search with no tracks: Clutch
    asks again before saying nothing was found, and a new genre stops the old one's track.
  - **Safe for monetized streams** (on by default): only tracks whose license allows
    commercial use (CC BY, CC BY-SA, CC0). Off, NonCommercial tracks are included too, and the
    settings say they're for non-monetized streams only.
  - **Credit:** every Creative Commons license needs the artist credited. The source shows
    "Title by Artist (CC BY 3.0)" in its media controls, and writes "Now playing: …" to a text
    file that a Text source (Read text from file) shows on screen with its own styling. The
    settings say where the file is.
  - The wording is "Creative Commons music from Jamendo", never "guaranteed DMCA-free": an
    artist may also register a track elsewhere.
- **Client ID:** Clutch is registered as a Jamendo app. The ID is public (like the Twitch one)
  and comes from `.secrets/clutch.env` (`CLUTCH_JAMENDO_CLIENT_ID`) or the environment at build
  time. Builds without it show the source as unavailable with a plain explanation.
- Network trouble: the source keeps playing what it has, retries the playlist in the
  background, and says so in its status.

## Impact
- Affected specs: music-source (new), media-sources (shares the player).
- Affected code: new `crates/estream-media/src/music_source.rs` (Jamendo client, playlist,
  credit), the player (HTTPS stream URLs), `apps/estream/build.rs` (client ID), source
  registration, the source picker and docs.
