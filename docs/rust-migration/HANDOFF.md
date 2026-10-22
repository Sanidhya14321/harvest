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

Completed **`media:sixel`**: ordinary Rust/SDK and temporary N-API consumers now
share one bounded engine. The Git-reconstructed baseline, native Rust consumer,
rebuilt addon and host contracts pass. Its larger media capability stays partial.

Completed **`control:terminal-reproducibility`**: generated payloads match between
two checkouts and different Cargo homes. They remain transitional JS artifacts
owned by the native UI migration, not approved final-product glue.

Current foundation repair: **`control:portability`**. Linux runtime/memory/tooling
contracts, strict Clippy and tokenizer rustdoc pass. Push the repair and inspect
Windows/macOS CI results before marking it done. `control:coverage` remains
blocked by a measured first-party coverage gap; the 65% floor remains intact.
See `tasks/COVERAGE.md` before changing coverage scope. Independent reviewed
subsystem-review procedures remain available; split implementation work into
bounded cards before making it ready.

A live read-only version-checker probe failed because this environment's HTTPS
proxy denied the GitHub API tunnel. Native/offline and loopback HTTP contracts
pass; no live freshness claim or provider-source update was made.

Release acceptance registrations deliberately block while their installed suites
and capabilities remain unverified. Run `cargo xtask acceptance GATE` to obtain
the specific missing requirement. Do not substitute smoke tests for full parity.

Preserve the immutable original baseline and all deleted-source ledger rows.
Store final verification receipts under `target/migration/` and preserve them in
CI artifacts. Use Git history for the actual last committed checkpoint.
