# Design: add-source-audio-tracks

## Where the sound is taken
The mixer already reads and processes each source once per 10 ms block: read → sync delay →
filters → mono → balance → volume/mute (smoothed). The resulting `Channel::block` is exactly what
the source adds to a mix, so a source track is that block. Monitor-only sources are not in the mix,
so they get no source track.

## Mixer → output
- `TrackSink` gains `fn sources(&self) -> &[SourceId]` (default empty) and
  `fn push_source(&mut self, slot: usize, block: &Block)` (default no-op). Each block the mixer
  calls `push_source` for each subscribed source in that sink's canvas mix (silence for a source
  that is gone or not in this canvas), then `push` with the tracks as today.
- No allocation on the audio thread: `ChannelSink` fills source blocks into chunk buffers from a
  fixed pool that the encoder thread hands back after use. If the pool is ever empty the chunk is
  dropped and the encoder's existing realignment pads silence, as for any lost block.

## Which sources
- Fixed when the output starts (a container can't add streams later): every source with audio
  that is in the canvas's mix and not monitor-only, in mixer order, at most 16. Sources added
  later reach the mixed tracks only.
- Stream metadata: `title` (MKV) and `handler_name` (MP4/MOV) = the source name. Mixed tracks are
  named "Mix" / "Track N" and keep the default disposition; source tracks don't have it.

## Leaving out silent tracks
- The encoder thread marks, per source track, the 1 s spans in which the source was heard (block
  peak above −60 dBFS).
- `PacketRing` keeps those spans; `save_range` writes a source track only if a span overlaps the
  clip window.
- Recordings: the spans of the whole file go to the MP4 remux, which leaves out source tracks never
  heard. A recording kept as MKV or written as MP4 directly keeps all its source tracks.

## Clip engine
`clip_engine` takes the studio project's `audio_input` sources (device, filters, volume, mute,
sync offset, mono, balance) instead of a bare default mic, and falls back to the default mic when
the studio has none. Game and desktop audio stay as today. With source tracks on, each is its own
track automatically.

## Clips player
- `Player` decodes one or more audio streams. By default it plays the first (mix) stream. With a
  selection of source tracks it decodes those and sums them (same 48 kHz conversion as today).
- A Tracks button next to the volume opens a menu: "Full mix" plus one check item per source track
  (names from stream metadata). The choice applies to the open clip only.
- Trim (lossless and precise) copies stream titles and dispositions; precise trim re-encodes each
  audio stream as it does now.

## Cost
One AAC encoder per source track on the encoder thread (≈0.2% of a core each at 192 kbps). Replay
memory grows by about 0.7 MB per source per 30 s.
