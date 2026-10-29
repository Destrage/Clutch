# Contributing to Clutch

Clutch is spec-driven. Before you change anything, read:

1. `CLAUDE.md` (short project rules),
2. `openspec/AGENTS.md` (how changes are proposed, implemented and ticked off),
3. `openspec/project.md` (architecture, code style, real-time thread rules).

Each piece of work belongs to an OpenSpec change in `openspec/changes/<change-id>/`. Work through
its `tasks.md` in order and tick a task (`- [x]`) only when it is done and tested.

## One-time setup

```sh
git clone https://github.com/Destrage/Clutch.git
cd Clutch
git config core.hooksPath tools/git-hooks   # Claude Code also sets this when a session starts
git config user.name  "Your Name"
git config user.email "you@example.com"   # an email on your GitHub account
```

Your commits carry your name and email, so they stay yours in the history and on GitHub. Use an
email that is verified on your GitHub account (or your `<id>+<user>@users.noreply.github.com`
address), or GitHub can't link the commits to your profile.

Then follow the build instructions for your system: `README.md` for Windows and Linux, and the
"Development setup" section of `openspec/changes/add-macos/design.md` for macOS.

## Git rules

`main` is the release line. Releases are built from it, so it only moves when a maintainer
merges a reviewed pull request.

**Always:**

- Start from an up-to-date `main`:
  `git switch main && git pull --ff-only && git switch -c <branch>`.
- Commit on your branch, push it (`git push -u origin <branch>`), and open a pull request into
  `main`.
- Get the latest `main` into your branch by merging: `git fetch origin && git merge origin/main`.
  A normal push then works.
- Keep pull requests small: one task group from `tasks.md` per PR is a good size.

**Never:**

- commit on `main`, push to `main`, or merge anything into `main`;
- force-push, or rebase a branch you have already pushed;
- push tags, delete branches on GitHub, or push several branches at once (`--all`, `--mirror`);
- skip hooks (`--no-verify`) or change `core.hooksPath`;
- merge pull requests, create releases, or change repository settings (`gh pr merge`,
  `gh release`, `gh repo edit`);
- bump the version in `Cargo.toml`, or edit release, website or Discord workflows;
- add `Co-Authored-By` or "Generated with" lines to commits or pull requests.

Maintainers review, test, merge with `--no-ff`, and release. Your part ends when the PR is open
and CI passes. After review, push more commits to the same branch.

## Branch names

Work that changes only one platform gets that platform as a prefix. Work that changes every
platform has no prefix.

| Work | Branch | Example |
|---|---|---|
| Windows only | `windows/<change-id>` | `windows/fix-game-capture-hdr` |
| Linux only | `linux/<change-id>` | `linux/add-linux-packages` |
| macOS only | `mac/<change-id>` | `mac/add-macos` |
| All platforms | `<change-id>` | `add-chat-overlay` |

A large change can go in several PRs. Add the step to the branch name:
`mac/add-macos-build`, `mac/add-macos-capture`. The OpenSpec change id itself stays plain and
verb-led (`add-macos`), without a prefix.

## Commits and PR titles

Use [Conventional Commits](https://www.conventionalcommits.org/). Platform work uses the
platform as its scope; shared work uses the area it touches.

```
feat(mac): ScreenCaptureKit display capture
fix(windows): game capture keeps the HDR flag
feat(linux): .deb and .rpm packages
fix(audio): meter peak hold resets on mute
docs(openspec): tick add-macos 2.1
```

Every commit should build. A PR title follows the same format.

## Keep the other platforms working

- Code for one platform goes behind `cfg`, in that platform's module (for example
  `crates/estream-capture/src/macos/`, next to `windows/` and `linux/`).
- If you must change shared code, list it in the PR under "Shared code touched".
- CI builds, lints and tests on Windows and Linux for every pull request (macOS joins once
  `add-macos` adds its job). A PR is ready to merge only when every job passes.
- Don't change how Windows or Linux behave in a macOS PR, or the other way round. Open a
  separate PR for that.

## Secrets and binaries

Never commit `.secrets/`, `third_party/` contents, built binaries, keys or tokens. Contributor
builds don't have the platform app IDs (YouTube, Kick, Jamendo), so those features say they
"aren't available in this build". That is expected.

## Public text

README, UI strings, release notes and store text describe what Clutch does. They never compare
Clutch with OBS or other products, and they are written plainly, without decorative emoji.
Naming the obs-websocket protocol for compatibility is fine.

## The guards

Four layers keep `main` safe. If one of them stops you, it is working as intended: change your
approach rather than working around it, and ask a maintainer when unsure.

1. **GitHub ruleset on `main`**: changes arrive only through a pull request approved by the code
   owner (`.github/CODEOWNERS`); force pushes and deleting `main` are refused.
2. **Claude Code hook** (`.claude/settings.json`, `.claude/hooks/git-guard.sh`): blocks the
   commands listed under "Never" before they run.
3. **Git hooks** (`tools/git-hooks/`): `pre-commit` and `pre-merge-commit` refuse commits on
   `main`; `pre-push` refuses pushes to `main`, tag pushes, force pushes and branch deletes.
4. **Main guard workflow** (`.github/workflows/main-guard.yml`): a direct push to `main` by anyone
   but the maintainer fails CI and opens an issue for the maintainer.

Maintainer clones set `git config clutch.maintainer true`, which turns layers 2 and 3 off.
Contributors never set it.
