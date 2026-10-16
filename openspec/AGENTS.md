# OpenSpec Workflow for eStream

eStream uses [OpenSpec](https://github.com/Fission-AI/OpenSpec) for spec-driven development.
Read `openspec/project.md` first for context, stack and conventions.

## Layout

```
openspec/
├── project.md              # Project context, conventions, roadmap
├── specs/<capability>/     # SOURCE OF TRUTH: what is built (spec.md, optional design.md)
└── changes/<change-id>/    # PROPOSALS: what should change
    ├── proposal.md         # Why / What Changes / Impact
    ├── design.md           # Technical decisions (when non-trivial)
    ├── tasks.md            # Implementation checklist
    └── specs/<capability>/spec.md   # Delta: ADDED / MODIFIED / REMOVED / RENAMED Requirements
```

## Three stages

1. **Propose**: create `changes/<verb-led-id>/` with proposal, tasks, and spec deltas.
   Run `openspec validate <id> --strict`. Don't implement until the proposal is approved.
2. **Implement**: work through `tasks.md` in order and tick `- [x]` as each task is done.
3. **Archive**: after merge/deploy, run `openspec archive <id> --yes`. This moves the deltas into `specs/`.

## Spec format rules

- Every requirement uses `### Requirement: <name>` and normative SHALL/MUST wording.
- Every requirement has at least one `#### Scenario: <name>` (exactly 4 hashes) with
  `- **WHEN** …` / `- **THEN** …` bullets.
- MODIFIED requirements must restate the full requirement text, not just the diff.

## Useful commands

```bash
openspec list                 # active changes
openspec list --specs         # capabilities
openspec show <item>          # view change or spec
openspec validate --strict    # validate everything
openspec archive <id> --yes   # archive after deployment
```
