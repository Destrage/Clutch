# Tasks: fix-settings-downgrade

## 1. Loading
- [x] 1.1 Read settings leniently: on an error, drop the entry at the error's path (or its parent when a field goes missing) and retry; log each dropped entry
- [x] 1.2 Copy the file to `settings.invalid-<date>-<time>.json` before anything is dropped or defaults are used
- [x] 1.3 Tests: unknown integration in a list, unknown enum value in a field, unreadable JSON

## 2. Verification
- [x] 2.1 `cargo test -p estream`, `cargo clippy -p estream --all-targets`
