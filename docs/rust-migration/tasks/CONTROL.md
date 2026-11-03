# control:playbook — make migration state executable

Kind: control. Prerequisites: none. Work unit: tooling. Source ownership remains
in the existing ledger; this task does not claim migration of those sources.

## Contract and inputs

Read `PLAYBOOK.md`, `SCOPE.md`, the existing source baseline/ledger, capabilities,
`xtask/src/{migration,capabilities,source_audit,verification}.rs`, the existing
verification tests and Rust CI. Preserve the original baseline byte-for-byte.

Fresh sessions must obtain accurate state and a reviewed next task. Unowned
sources, impossible task transitions, missing prerequisite IDs, cycles, missing
cards and unsupported completion claims must fail with actionable errors.

## Ordered implementation

1. Fix formatting argv; move the capability manifest to v4 with pending stages.
2. Use the shared completion gate and narrowly approved retirement decisions.
3. Count generated executable bindings, templates and all owned script languages.
4. Extend the existing execution graph with task metadata and ledger ownership.
5. Add `cargo xtask playbook check|status|next [--json]` and blocked release-gate
   registration/dispatch. Real installed suites remain pending until implemented.
6. Enable `ts-to-rust` checks and preserve verification receipts as CI artifacts.
7. Archive stale instructions; connect root instructions, README and live handoff.

## Verification and deletion

```sh
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask migration
cargo xtask capabilities
cargo xtask playbook check
cargo xtask source-audit status
cargo xtask source-audit verify --scope affected --base d811cb223cedd065e40112c02f6c0d8b5e7009e4 --plan
```

Require integration cases for candidate/legacy gate transitions, retirement
boundaries, task prerequisites/ownership and generated/template language auditing.
Both `--require-complete` commands must still fail. Gate registrations with no
installed suite must fail; do not claim they certify product behavior.

No application source deletion belongs to this control task. Recovery: revert
only this coherent commit; never reset unrelated work or change the baseline.
Completion evidence is the named tests/checks and CLI results on the finalized
tree. Record environmental blockers separately from successful verification.
