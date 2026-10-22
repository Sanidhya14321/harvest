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

Completed **`tooling:version-wrapper`**: removed the unused Bun launcher for
version synchronization. Release consumers already invoke `cargo xtask npm-versions`;
native release/check contracts run without Bun/Node on PATH.

Completed **`control:inert-data`**: source/task final gates now consistently
accept explicitly reviewed, fully verified retained data on the existing exact
allowlist. `verified_inert` is separate from removals; no actual source row was
advanced by this accounting repair.

Completed **`media:sixel`**: ordinary Rust/SDK and temporary N-API consumers now
share one bounded engine. The Git-reconstructed baseline, native Rust consumer,
rebuilt addon and host contracts pass. Its larger media capability stays partial.

Completed **`control:terminal-reproducibility`**: generated payloads match between
two checkouts and different Cargo homes. They remain transitional JS artifacts
owned by the native UI migration, not approved final-product glue.

Completed **`control:oauth-fixtures`** and **`control:legacy-workspace`**: 19
refresh-race assertions, seven planner/watchdog contracts and all 49 workspace
chunks pass. AI fixtures now inject at the current refresh boundary; the legacy
runner bounds VM/process lifetimes without dropping test files. The earlier
full-transition failure is retained as a failed receipt, not reclassified as a
passing transition. Other legacy buckets/release evidence remain separate.

`control:portability` now awaits Windows release-profile CI for the accepted
socket blocking-mode follow-up. Five local provider-usage contracts pass;
Linux/ARM/macOS contracts and the earlier Windows debug contracts passed CI.
Keep the task blocked until the new Windows variants pass.

Current active work: **`control:coverage`**. An additive instrumented build of all
nine existing pi-* Rust engine suites plus the previous product suites is running
in the isolated coverage worktree (`git worktree list`). It uses
`target/coverage-native` under the main workspace as its instrumented target.
Run its full tests and unchanged 65% report before committing the workflow
selection. The nine-package recipe is in its card; the current pushed CI recipe
still has the old package selection and is expected to fail that floor.
See `tasks/COVERAGE.md` before changing coverage scope. Installation now passes
its help/version/brain checks, then hits the independent binary-size budget
(121,519,040 > 20,971,520 bytes). `control:artifact-size` records the measured
blocker and next resource/link-map investigation; do not raise the budget. Independent reviewed
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
