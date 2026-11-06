# Change: Going live on several canvases at once

## Why
A user duplicated their 1920 × 1080 canvas as 2560 × 1440, set Kick and Twitch to the 1080p
canvas and YouTube to the 1440p one, and went live. Three things went wrong:

- **Scene switches only reach one stream.** Switching a scene (from the scene cards, the Live
  deck, studio mode, hotkeys or the remote API) changes only the canvas being edited. Picking
  "Be right back" put Kick and Twitch on it while YouTube stayed on "Gameplay".
- **Audio leaks between canvases.** All canvases share one audio mix made from every canvas's
  current scene. "Application audio" (Discord) was only in the two "Gameplay" scenes, but
  while either canvas was on Gameplay it was heard on every stream and showed in the mixer on
  every scene. One canvas on "Be right back" still sent the game and the voice chat.
- **Studio mode is hard to reach and covers one canvas.** While live, the only way in is
  Edit layout, then Studio mode. Transition only takes the edited canvas to program.

The same user heard Discord and the game twice on every platform. Their "Apex" collection has
two separate Discord Application audio sources: one in the gameplay scenes, one in the others.
Because the two canvases sat on different scenes, both were in the shared mix. Desktop Audio
was also on. Windows lets Desktop Audio leave out only one program, and Clutch picks the
captured game, so Discord was in Desktop Audio as well. Nothing in the mixer said so.

The Go live window also offers one canvas for every destination ("Pick one to stream it
everywhere"). It can't show or change a per-destination setup like Kick and Twitch on 1080p
with YouTube on 1440p.

## What Changes
- **Scenes switch together.** A scene switch on one canvas also switches every other canvas
  that has a scene with the same name, with the same transition. Duplicate canvas and Copy to
  canvas already keep scene names, so copies follow without setup. Each canvas tab menu has
  "Switch scenes with other canvases" (on by default) to let a canvas switch on its own. The
  switch happens in the engine, so the scene cards, Live deck, studio mode, hotkeys,
  automation and the remote API all get it.
- **Each canvas has its own audio mix.** The mix that a canvas's outputs encode holds only the
  global sources plus the sources visible in that canvas's current scene (and in the scene
  being left during a transition). Each source is still read and filtered once and then added
  to the mix of every canvas that shows it. The mixer and its master meter show the canvas
  being edited. Monitoring is unchanged: it plays every source that is in any canvas's mix.
- **Studio mode across canvases.** The Preview shows the edited canvas. Transition takes every
  canvas that switches together to its scene of the same name, so all streams change at once.
- **Studio mode in the Live deck.** The deck header gets a "Studio mode" switch. With it on,
  the deck shows Preview and Program side by side, the scene buttons pick the Preview, and
  large Transition and Cut buttons send it to program. With more than one canvas, a canvas
  picker in the deck header chooses which canvas the deck shows (live ones are marked).
- **Sound captured twice gets a warning.** The mixer marks a strip when its sound reaches the
  same mix twice:
  - two Application audio sources capture the same app;
  - Desktop Audio already carries an app that another source captures, because Desktop Audio
    is leaving out a different program.

  Each warning says what to change ("Desktop Audio also carries Discord. It can leave out one
  program, and it leaves out Apex. Mute Desktop Audio, or remove the Discord source.").
- **Go live picks a canvas per destination.** With more than one canvas, each destination row
  in the Go live window gets its own canvas choice, saved to that destination. The "all
  destinations" choice stays as a shortcut that sets every row.

- **Old update downloads are deleted.** At startup Clutch removes downloaded installers for
  the running or an older version and unfinished downloads. One user's updates folder held
  0.88 GB of old installers.

## Impact
- Affected specs: multi-canvas, studio-mode, video-sources.
- Affected code: `crates/estream-project/src/model.rs` (canvas follow flag),
  `crates/estream-engine/src/engine.rs` (linked transitions), `crates/estream-engine/src/graph.rs`
  and `crates/estream-audio/src/mixer.rs` (per-canvas buses, canvas-tagged sinks),
  `crates/estream-media/src/output.rs` (sinks for the output's canvas), `apps/estream/src/app.rs`
  (mixer strips, studio across canvases, deck studio mode, tab menu toggle),
  `apps/estream/src/golive.rs` and `apps/estream/ui/golive.slint` (per-destination canvas),
  `apps/estream/ui/deck.slint`, `apps/estream/ui/panels.slint`,
  `crates/estream-capture/src/windows/app_audio.rs` (which processes Desktop Audio carries),
  `apps/estream/src/update.rs` (download cleanup).
- Real-time rules: canvas buses are allocated in `sync`, never in `process`. A source on N
  canvases costs N block additions per track, not N reads.
- Saved projects gain one field with a default (`follow_scenes: true`); nothing is migrated.
  Existing duplicated canvases switch together right away, because their scenes share names.
