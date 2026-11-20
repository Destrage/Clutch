# Tasks: add-source-audio-tracks

## 1. Mixer
- [x] 1.1 `TrackSink::sources` / `push_source`; the mixer feeds each subscribed source's processed block (silence when absent from the canvas mix)
- [x] 1.2 Unit tests: a source track equals the source's part of the mix (filters, volume, mute, balance); monitor-only sources get none

## 2. Outputs
- [x] 2.1 `RecordingSettings::source_tracks` (default on) and the source list fixed at output start: every audio source of the canvas's scenes, max 16, not for streams
- [x] 2.2 Pooled source blocks from `ChannelSink` to the encoder thread; one audio encoder per source track, same realignment as the mixed tracks
- [x] 2.3 Stream names and dispositions in `Muxer` (mixed tracks first and default)
- [x] 2.4 Heard spans per source track; `PacketRing::save_range` leaves out source tracks not heard in the window
- [x] 2.5 `remux` keeps names and dispositions and can leave out silent source tracks; `Recording::stop_all` passes them
- [x] 2.6 Integration tests (ffprobe): a recording with a tone source and a silent source has named source tracks; a saved clip leaves out the silent one; streams have no source tracks

## 3. Clip mode
- [x] 3.1 `clip_engine` takes the studio's microphone sources with their filters and volume (default mic when none)
- [x] 3.2 Unit test: the clip engine project has the studio mic's filters

## 4. Clips player and trim
- [x] 4.1 `Player` audio stream selection: the default stream, or several summed
- [x] 4.2 "Hear" row in the clip player (Full mix + each further named track), keyboard and screen-reader labels
- [x] 4.3 Lossless and precise trim keep stream names and dispositions
- [x] 4.4 Settings → Recording: "Separate track for each source" switch

## 5. Verification
- [x] 5.1 `cargo test` and `cargo clippy --all-targets -- -D warnings` for the touched crates
- [x] 5.2 Record and clip a game with mic and Discord: tracks named, silent ones left out of clips, the player plays the game alone, no new dropped frames or audio gaps while streaming
