# eStream

A modern, from-scratch streaming/recording studio (OBS alternative) written in Rust
(wgpu compositor, Slint UI, FFmpeg encoding).

- Spec-driven: read `openspec/AGENTS.md` and `openspec/project.md` before making changes.
- New features or architecture changes need an OpenSpec change proposal in `openspec/changes/` first.
- Implement approved changes task by task and tick items in that change's `tasks.md`.
- Follow the conventions in `openspec/project.md` (real-time thread rules, command bus for all mutations).
