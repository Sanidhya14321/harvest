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

The full native TUI suite also has unresolved contracts; see
`tasks/LEGACY-TERMINAL-CONTRACTS.md`. Keep that failure and whole-addon lint
failures visible alongside passing DOCX/host checks.

Execution split: the supervising agent selects/contracts/reviews bounded slices;
OpenCode performs their code edits and scoped tests in an isolated worktree.
The first assignment is `tasks/GEN-NIX.md`; it owns the two inherited Nix source
rows. Review the resulting diff before final commits/pushes. No model output or
claimed test result alone advances the source-removal gate.

OpenCode connection blocker: the first `tooling:gen-nix` run failed with an HTTP
proxy 403 before editing. Its reviewed task remains blocked pending access to
OpenCode's API or an accessible configured server/provider. See GEN-NIX.md for
the exact execution checkpoint; do not claim that delegation completed the port.
