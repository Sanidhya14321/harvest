# Current migration handoff

Development branch: **`ts-to-rust`**. Starting remote checkpoint: **`d811cb2`**.
Read `PLAYBOOK.md` and `SCOPE.md`; the archived 2026-10-20 handoff records older
decisions and counters and is not the current task queue.

The normal application still starts in TypeScript. No whole-product parity or
distribution cutover is claimed. Existing browser/WASM proofs do not satisfy the
new zero-JavaScript target.

Completed control slice: **`control:playbook`** implements strict scope, gate
repairs, source ownership/task catalog, generated status and branch CI. Its
verification includes 105 xtask contracts plus doctests, strict Clippy and CLI
state/removal gates. This does not certify the unimplemented release workflows.
Obtain live counts
and task state from:

```sh
cargo xtask migration
cargo xtask capabilities
cargo xtask playbook status --json
cargo xtask playbook next --json
```

Completed **`tooling:spoofed-versions`**: the root maintenance command now calls
Rust; its TS entrypoint is removed. Native CLI parity, failure and jailed update
contracts pass. Live upstream freshness remains separate from offline fixtures.

Next bounded implementation: **`media:sixel`**, with reviewed instructions.
Its card is `tasks/SIXEL.md`. Baseline fixtures must come from Git and a rebuilt
addon; archived machine-local cache paths are optional historical evidence.

Release acceptance registrations deliberately block while their installed suites
and capabilities remain unverified. Run `cargo xtask acceptance GATE` to obtain
the specific missing requirement. Do not substitute smoke tests for full parity.

Preserve the immutable original baseline and all deleted-source ledger rows.
Store final verification receipts under `target/migration/` and preserve them in
CI artifacts. Use Git history for the actual last committed checkpoint.
