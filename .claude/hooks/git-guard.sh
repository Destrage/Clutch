#!/usr/bin/env bash
# Claude Code PreToolUse guard for the Clutch git rules (CONTRIBUTING.md).
# Blocks commits on main, pushes to main, force pushes, tag pushes, skipped hooks and
# merge/release commands. Clones marked `git config clutch.maintainer true` are not checked.
# Exit 2 blocks the tool call and shows the reason to Claude.

[ "$(git config --get clutch.maintainer 2>/dev/null)" = "true" ] && exit 0

input=$(cat)
if command -v jq >/dev/null 2>&1; then
  cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)
else
  # No jq (Git Bash on Windows): match against the raw JSON instead.
  cmd=$input
fi
[ -n "$cmd" ] || exit 0

has() { printf '%s\n' "$cmd" | grep -Eq -- "$1"; }

block() {
  echo "Blocked by the Clutch git rules (CONTRIBUTING.md): $1" >&2
  exit 2
}

# Start of a command word, the git binary with optional -C/-c options, and an end of word.
S='(^|[;&|("[:space:]])'
G="${S}git([[:space:]]+-[cC][[:space:]]+[^[:space:];&|]+)*[[:space:]]+"
E='([[:space:];&|)"]|$)'

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)

if has "${G}push${E}"; then
  has "${E}(--force|--force-with-lease|--force-if-includes|-f)${E}|[[:space:]]\+[^[:space:]]+" &&
    block "force pushes are not allowed. Merge main into your branch instead of rebasing, so a normal push works."
  has "[[:space:]:/](main|master)${E}" &&
    block "nothing is pushed to main. Push your own branch and open a pull request."
  has "${E}(--tags|--mirror|--all|--delete|-d)${E}" &&
    block "tag, mirror, all-branch and delete pushes are for maintainers. Push one branch at a time."
  case "$branch" in main | master)
    block "you are on $branch. Create a branch first: git switch -c <prefix>/<change-id>." ;;
  esac
fi

if has "${G}(commit|merge|cherry-pick|revert|am|rebase|reset)${E}"; then
  case "$branch" in main | master)
    block "you are on $branch, which only changes through pull requests. Create a branch first (git switch -c <prefix>/<change-id>), then run the command on its own." ;;
  esac
fi

has "${E}--no-verify${E}" &&
  block "git hooks must not be skipped. Fix what the hook reports instead."

has "${G}config${E}.*clutch\.maintainer" &&
  block "clutch.maintainer is set by the maintainers in their own clone only."

if has "${G}config${E}.*core\.hooksPath"; then
  has "core\.hooksPath[[:space:]]+tools/git-hooks${E}" ||
    block "core.hooksPath must stay tools/git-hooks."
fi

has "${S}gh[[:space:]]+(pr[[:space:]]+merge|release|repo[[:space:]]+(edit|delete|rename|archive))${E}" &&
  block "merging pull requests, releases and repository settings are done by the maintainers. Open the PR and stop there."

has "${S}gh[[:space:]]+api${E}.*(git/refs|/merges|/pulls/[0-9]+/merge|/releases|/branches/)" &&
  has "(-X|--method)[[:space:]]*(POST|PUT|PATCH|DELETE)|(-f|-F|--field|--raw-field|--input)[[:space:]]" &&
  block "changing refs, merges, branches or releases through the API is for maintainers."

exit 0
