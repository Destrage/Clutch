# Clutch

A modern, from-scratch streaming, recording and clipping studio written in Rust
(wgpu compositor, Slint UI, FFmpeg encoding). Formerly eStream: crate names still use `estream-*`.
Public text (README, UI, store and GitHub descriptions) never positions Clutch as an
alternative to, or comparison with, OBS or other products; describe what Clutch does instead.
Naming the obs-websocket protocol for compatibility is fine.

- Spec-driven: read `openspec/AGENTS.md` and `openspec/project.md` before making changes.
- New features or architecture changes need an OpenSpec change proposal in `openspec/changes/` first.
- Implement approved changes task by task and tick items in that change's `tasks.md`.
- Follow the conventions in `openspec/project.md` (real-time thread rules, command bus for all mutations).
