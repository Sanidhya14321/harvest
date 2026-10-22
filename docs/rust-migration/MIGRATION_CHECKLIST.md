# Rust migration checklist

Use this file as the single entry point. It does not duplicate authoritative records.

## Rules

- Work on `ts-to-rust`; preserve history and user changes.
- One verified behavior slice per commit; no publish/merge to `main`.
- Use `cargo xtask` commands; never bare `cargo test` for migration verification.
- Temporary adapters are tracked and must be retired.
- No authored first-party JS/TS/Python/Julia/shell artifacts in final product.

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
- [ ] Zero authored non-Rust executable artifacts
- [ ] Eight installed artifacts pass
- [ ] Release acceptance gates pass
