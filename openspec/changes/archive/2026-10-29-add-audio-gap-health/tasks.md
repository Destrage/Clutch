# Tasks: add-audio-gap-health

## 1. Media
- [x] 1.1 Realign decision as a function, with tests (in sync, gap, stale block, jitter)
- [x] 1.2 Encoder test: after `pad_silence`, the next audio packet's timestamp includes the gap
- [x] 1.3 `OutputStats::audio_gap_ms` counts the silence filled

## 2. Health and Stats
- [x] 2.1 `OutputCounters`/`OutputRate` carry audio gaps; alert raised on any gap, problem over
      1 s in 10 s, cleared after 10 quiet seconds; monitor test
- [x] 2.2 Stats output table: Audio gaps column

## 3. Check
- [x] 3.1 Clippy, tests
