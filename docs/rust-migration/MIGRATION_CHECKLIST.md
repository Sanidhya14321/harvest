# Rust migration checklist

Use this file as the single entry point. It does not duplicate authoritative records.

## Rules

- Work on `ts-to-rust`; preserve history and user changes.
- One verified behavior slice per commit; no publish/merge to `main`.
- Use `cargo xtask` commands; never bare `cargo test` for migration verification.
- Temporary adapters are tracked and must be retired.
- No first-party executable JS/TS/Python/Julia/shell artifacts in the final product,
  including generated bindings, templates and embedded payloads. Retain only
  reviewed configuration, documentation, assets and inert fixture data.

## Pointers

- Scope: `docs/rust-migration/SCOPE.md`
- Playbook: `docs/rust-migration/PLAYBOOK.md`
- Handoff: `docs/rust-migration/HANDOFF.md`
- Tasks: `docs/rust-migration/tasks/`
- Capabilities/graph: `docs/rust-migration/capabilities.json`
- Source ledger: `docs/rust-migration/source-ledger.json`
- Source baseline: `docs/rust-migration/source-baseline.json`

## Live status commands

```sh
cargo xtask migration
cargo xtask capabilities
cargo xtask playbook status --json
cargo xtask playbook next --json
```

## Checklist

- [x] Control system repaired (`control:playbook`)
- [ ] Complete protocol migration (`migrate:protocol`) — in progress
- [ ] Review and migrate remaining subsystems per task cards
- [ ] All retained capabilities verified
- [ ] All approved retirements removed
- [ ] Zero tracked or shipped first-party non-Rust executable artifacts
- [ ] Eight installed artifacts pass
- [ ] Release acceptance gates pass

## Current continuation

The imported checkpoint is `6eefffc`; its history is preserved. Read the live
HANDOFF.md for subsequent verified slices. `tasks/CONCURRENT-REMOVALS.md` records
37 inherited deletions still missing replacement/retirement proof. Keep them
pending; do not report file deletion or a Rust type port as verified behavior.
The strict source-removal gate remains blocked until those contracts are resolved.
`tasks/MEDIA-DOCX.md` and `tasks/HOST-DETECT.md` describe the bounded native
consumer connections completed by this continuation. Use the live status commands
for counts and task state; this checklist does not maintain manual percentages.

The former full native TUI failures are resolved in
`tasks/LEGACY-TERMINAL-CONTRACTS.md`. Whole-addon lint failures remain visible
alongside the passing native, DOCX and host checks.

Execution mode: direct implementation, per the user's latest instruction.
Use PLAYBOOK.md's execution-batch rules: connected native Rust behavior, shared
engines, deduplicated focused checks and one final verification per frozen batch.
The OpenCode request failed before edits and that execution approach is cancelled;
its network connection is no longer a migration blocker.

Verified runtime batch: `control:legacy-terminal-contracts` resolves the six prior
native-suite failures together. All four native test groups pass; see its task
card for the clock boundary, retained assertions and remaining independent gates.
