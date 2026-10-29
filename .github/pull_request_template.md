## What and why

<!-- One or two sentences. Link the OpenSpec change: openspec/changes/<change-id>/ -->

OpenSpec change: `<change-id>`, tasks:

## Platforms

- [ ] Windows
- [ ] Linux
- [ ] macOS

<!-- Tick the platforms this PR changes. Say what you ran on each one you could test. -->

## Checks

- [ ] Branch is `<windows|linux|mac>/<change-id>` (or `<change-id>` for shared work), based on current `main`
- [ ] `cargo fmt -p <crate>`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass on my machine
- [ ] Code for one platform sits behind `cfg`; shared code changes are listed below
- [ ] Ticked tasks in the change's `tasks.md` are done and tested
- [ ] No version bump, release file, secret or `third_party/` binary in the diff

## Shared code touched

<!-- Files outside platform modules that this PR changes, and why. "None" is a fine answer. -->

## How it was tested

