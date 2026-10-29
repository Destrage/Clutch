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

## Git rules (read `CONTRIBUTING.md`)

Unless `git config clutch.maintainer` prints `true` in this clone, you are working as a
contributor, and these rules are not negotiable:

- Never commit on `main`, push to `main`, or merge into `main`. Branch from an up-to-date `main`,
  push the branch, and open a pull request into `main` with `gh pr create --base main`. Stop
  there: maintainers review, merge and release.
- Branch names carry the platform: `windows/<change-id>`, `linux/<change-id>`, `mac/<change-id>`;
  shared work is plain `<change-id>`. Commits are Conventional Commits with the platform as
  scope (`feat(mac): …`, `fix(windows): …`).
- No force pushes, no rebasing pushed branches (merge `origin/main` in instead), no tag pushes,
  no branch deletes, no `--no-verify`, no `gh pr merge` / `gh release`, no version bumps.
- No `Co-Authored-By` or "Generated with" lines in commits or PRs.
- If a guard (Claude Code hook or git hook) blocks a command, don't work around it. Change the
  approach or ask the user.

Maintainer clones still ask the user before pushing or merging into `main`.
