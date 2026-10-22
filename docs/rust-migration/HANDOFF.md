# Current migration handoff

Development branch: **`ts-to-rust`**. Starting remote checkpoint: **`d811cb2`**.
Read `PLAYBOOK.md` and `SCOPE.md`; the archived 2026-10-20 handoff records older
decisions and counters and is not the current task queue.

The normal application still starts in TypeScript. No whole-product parity or
distribution cutover is claimed. Existing browser/WASM proofs do not satisfy the
new zero-JavaScript target.

Completed **`browser:cdp-core`**: `harvest-browser` provides bounded native CDP
control of a marked, locked persistent Chrome profile. Native CLI, public SDK,
synchronous sessions and asynchronous sessions use the same engine and host
approval boundary. Real Chrome checks navigation, DOM, typing/form clicks, PNG,
exclusive ownership and persistent cookies; a copied native binary runs JSONL
without language runtimes on PATH. Browser/runtime contracts, 111 core app tests,
public SDK checks and strict Clippy pass. The initial screenshot probe raced a
form navigation and failed; the strengthened fixture observes the submitted DOM
before capture and passes. Logs retain both results. The larger native browser
capability remains partial; legacy browser consumers have not moved yet.

Completed **`control:embedded-sources`**: audit/ledger ownership now includes
source-named text wrappers and known injected/prelude/bundle containers. All 118
xtask contracts pass. Nineteen omitted executable payloads are recorded as pending;
this expands the inventory without claiming any additional source removal.

Next bounded browser slice: `tasks/BROWSER-RELAY-RETIREMENT.md`. Remove the approved
extension/relay interface after preserving actionable migration errors and all
remaining non-relay consumers. Do not delete shared browser/eval/computer code
until its own last consumer moves.

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

Completed **`tooling:homebrew-tap`**: the tap-staging shell script is replaced
by `cargo xtask tap-update`; CI/release consumers call it directly. The compiled
helper runs with Git-only PATH and preserves the baseline new/no-op/update
behavior, unrelated staged work, local config and linked worktree isolation.
Five integration contracts and all 117 xtask tests plus doctests/strict Clippy
pass. This does not publish or certify the complete release capability.

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

`control:portability` awaits updated Windows/macOS CI. Five local provider-usage
contracts pass. The registry mirror now also explicitly restores accepted socket
blocking mode: a forced-nonblocking/delayed-header regression fails before the fix
and passes afterward, along with all five existing offline/cache/checksum registry
contracts. Timeouts and limits remain unchanged. The newer macOS release failure
is captured in `tasks/PORTABILITY.md`; keep this task blocked until platform CI
passes. Windows full-feature and macOS full-feature CI passed at `747421f`
(run `39782981204`); the core variants still need the follow-up evidence.

Completed **`control:terminal-fixtures`**: all 38 terminal/SIXEL host contracts
pass headlessly with an explicit fixture color mode, without a global FORCE_COLOR
override. Coding-agent typecheck and targeted formatting/lint pass; production
color detection is unchanged.

The source-bound transition at `747421f` passes 452 Rust contracts/doctests,
the native rebuild and all 49 workspace chunks, then fails 22 singleton contracts.
Its receipt remains failed. **`control:legacy-session-contracts`** records the
writer-lease, discovery/path and error/label families. Compare the baseline and
review their contracts before changing behavior or splitting implementation cards.

Completed **`control:coverage`**: the additive selection includes all nine
existing pi-* engine suites and every previous package. On Linux x86_64 with the
pinned nightly, 2,925 tests pass (five existing skips), and the unfiltered report
measures 107,698/144,597 lines (74.48%). The unchanged 65% enforcement succeeds.
Logs/LCOV are under `target/migration/coverage-current*`; CI uses the same command.
This measures selected Rust product/engine code, not whole-application parity.

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
