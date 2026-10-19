# Change: Redesigned studio (Stage, Live deck, sidebar sections)

## Why
The studio still reads like a classic broadcast tool: a fixed grid of Scenes, Sources, Mixer and
Properties panels around the preview. Clutch is aimed at players who stream, clip and make
montages, so each moment should get a screen built for it, in Clutch's own look.

## What Changes
- **Look:** the Clutch palette (slate surfaces, silver text, ice-blue accent, platform colours)
  and bundled fonts (Jost for headings, Figtree for text). Light theme kept, retuned.
- **App shell:** a left sidebar (Studio, Clips, Montages, Chat; setup switcher and Settings at the
  bottom), collapsible to icons. Clips and Montages become full pages in the shell instead of
  dialogs.
- **Stage (Studio, not live):** the preview fills the page.
  - Scenes as a strip of thumbnail cards along the bottom (live thumbnail for the active scene,
    the last frame seen for the others), with New scene at the end.
  - The scene's layers float over the preview's top left; the selected item's settings open over
    the top right, with the most-used options in plain words first and "More settings" for the rest.
  - Sound as round mute buttons with level bars (Mic, Game, Desktop, …); click-and-hold or the
    menu opens the full mixer for volume, filters and tracks.
- **Live deck (Studio, while live):** switches in automatically when a stream starts and back
  when it ends ("Edit layout" switches to Stage while live).
  - Top bar: LIVE and the stream time, viewers (total and per platform), connection health,
    End stream.
  - Program preview with "Clip that (F8)", big scene buttons with their F-key shortcuts, large
    sound faders, Record and clip-mode state.
  - The right column holds the chat (from add-chat-panel; a placeholder until then).
- **Clips and Montages pages** in the sidebar style: cards, recent clips, "Make a montage".

## Impact
- Affected specs: studio-ui.
- Affected code: `apps/estream/ui/*` (theme, shell, new stage and deck components, clips and
  montage pages), `apps/estream/src/app.rs` (page state, scene thumbnails, live switch),
  fonts under `apps/estream/ui/fonts` (OFL).
- No change to the engine, outputs or saved setups.
