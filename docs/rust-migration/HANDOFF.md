# Current migration handoff

Development branch: **`ts-to-rust`**. Starting remote checkpoint: **`d811cb2`**.
Read `PLAYBOOK.md` and `SCOPE.md`; the archived 2026-10-20 handoff records older
decisions and counters and is not the current task queue.

The normal application still starts in TypeScript. No whole-product parity or
distribution cutover is claimed. Existing browser/WASM proofs do not satisfy the
new zero-JavaScript target.

Completed **`control:legacy-terminal-contracts`** as one connected runtime batch:
the native input bridge and host timers now share an explicitly selected monotonic
clock, with validated timestamps and native-clock defaults for direct callers.
The OSC 66 capture checks the settled full paint instead of the heading-only
resize probe. Existing arrow, keyboard-push/pop and glyph-preservation assertions
remain. One optimized addon rebuild, 141 focused tests, all four full native
chunks, 127 Rust terminal contracts/four doctests and both affected typechecks
pass. The former six full-native failures are resolved; the separate 14 existing
whole-addon Clippy diagnostics remain unchanged and failed. No whole-product
parity or source deletion is inferred from this control fix.

Direct implementation resumed at the user's request; the OpenCode approach is
cancelled. Use the execution-batch rules in PLAYBOOK.md to avoid per-file binding
rebuilds and repeated whole-workspace tests. Scope and acceptance remain strict.

Completed **`tooling:gen-nix`**: the inherited Rust command now preserves the
legacy line-anchored version stamp contract and ignores non-executable PATH
entries. Rust normalization tests and compiled-CLI tests exercise native generator,
Nix development-shell and portable fallback argument/working-directory behavior,
failure propagation, existing-output preservation and future-version rejection.
All 125 xtask tests and strict Clippy pass. The external-generator fixture is Rust;
its temporary PATH has no installed language runtime. This proves orchestration,
not the third-party generator's Nix semantics. Bun fallback remains transitional.
Two inherited source rows are verified; the other inherited removals remain
pending. No native-addon rebuild is needed for this xtask-only batch.

Imported concurrent checkpoint **`6eefffc`**, preserving all 28 intervening commits
and the new `MIGRATION_CHECKLIST.md` entry point. Rust wire types and tooling ports
remain available. Protocol is still in progress until `packages/wire` consumers
move and installed behavior passes; storage/provider tasks wait behind that work.

**Source-removal blocker:** `cargo xtask migration` rejects 37 inherited deletions
without complete contracts and verified replacement/retirement evidence. Invalid
`implementation: verified` values and unsupported verified claims were reset to
pending review; removed files stay removed and their history is preserved.
`tasks/CONCURRENT-REMOVALS.md` lists each source and the first concrete review.
Do not make a false green gate or count these removals as completed migration.

Completed bounded slices: **`media:docx`** moves ZIP/XML/styles/lists/tables/footnotes
and image rendering into `harvest_media::docx`, shared by the native CLI, jailed
file reader, SDK and a temporary N-API callback bridge for Markit. Captured legacy
fixtures match exactly; malformed/bounded/cancelled operations fail explicitly.
The two superseded private TS engines are removed; the public TS adapter remains
pending with its final application/Markit consumers. Media remains partial.

**`tooling:host-detect`** repairs the concurrent port's Rust/Node architecture and
JSON naming mismatch, adds platform/arch fields needed by the Bazel wrapper, and
rejects unsupported hosts. Runtime CPU detection requires OS instruction support;
no sysctl/PowerShell helper is needed. Serialization tests cover host branches;
installed checks here cover Linux only, not all promised platforms.

The merged checkpoint passes 272 Rust contracts plus the media doctest, eight
copied-binary CLI contracts, 11 DOCX/Markit host checks, both affected TS
typechecks, strict media/application/xtask Clippy, and all 49 legacy workspace
groups. The addon is rebuilt through the corrected detector (139 exports).

Verification logs/receipts are under `target/migration/docx-*`,
`host-detect-*` and `merged-control-tests.log`. The optional strict whole-addon
Clippy check still fails in unchanged PTY/portable-terminal/Wayland/OAuth code;
`docx-napi-clippy.log` retains all 14 diagnostics. No lint was disabled. The media,
application and xtask strict checks are separate. The historical full native run failed six TUI contracts; that batch is now
resolved as recorded above and in `tasks/LEGACY-TERMINAL-CONTRACTS.md`. The
older full transition and its 22 singleton failures remain failed; passing
native/targeted checks do not replace that transition.

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

Completed **`browser:relay-retirement`**: the owned Chrome extension, relay
bridge/server/daemon, installer/build/smoke code, embedded payloads and retired
relay-only tests are removed. The old command and enabled relay requests return
native-browser migration guidance. Settings UI, registry/worker connections and
Bun/Nix workspace ownership are updated; license documents remain. Eighteen
source rows are verified approved retirements, not retained behavior parity.
The native candidate passes JSONL/profile reopening with an empty PATH and all
seven installed CLI contracts; 112 core app tests and strict Clippy pass.
The addon rebuild and 86 native host contracts also pass. Focused legacy browser,
argv/settings contracts and typecheck pass. Viewport fixtures now use loopback
HTTP because host Chrome policy blocks `data:`; failed receipts stay. The attach
retry fixture counts its target navigation rather than unrelated background
requests, retaining the exactly-once assertion.

The browser search/PDF/computer/eval and other native-product consumers still
need bounded implementation cards and migration. Their shared sources stay
pending until the last consumer moves; do not retire them based on relay removal.

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
