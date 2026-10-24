# Change: Keep settings readable across versions

## Why
Settings are one JSON file read as a whole. When an older build meets a value it doesn't know (a
newer build's game integration, say `pubg_replay`), the whole file fails to read, Clutch starts
with defaults and then saves them over the file: destinations, encrypted stream keys and account
logins are gone. This happened to a tester on 2026-10-24 after installing an older beta, and it
will happen to anyone who goes back a version once 0.5 adds new settings.

## What Changes
- **Skip what isn't understood:** when the file doesn't read as a whole, Clutch drops only the
  entries it can't read (an unknown list item, or the smallest setting around it) and keeps the
  rest. Each dropped entry is logged.
- **Keep the original:** whenever anything is dropped, or the file can't be read at all, the file
  is first copied to `settings.invalid-<date>-<time>.json` next to it, so nothing is lost even
  when Clutch saves its settings afterwards.

## Impact
- Affected code: `apps/estream/src/settings.rs` (loading), new dependency `serde_path_to_error`.
- No change to the file format.
