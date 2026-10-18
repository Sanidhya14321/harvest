# Rust migration implementation status

The [2026-10-17 checkpoint](progress-2026-10-17.md) records the current execution
stage counts, fresh verification, partial async SDK, experimental RPC and working read-only WASM/terminal replay.

The normal application still launches TypeScript. This change establishes the
shared Rust foundations and native brain maintenance; it does not complete the
application rewrite. No capability is marked as legacy-parity complete simply
because a Rust command or tool has the same name.

The [integrated execution plan](execution-plan.md) gives the dependency order,
deletion gates, route/middleware scope, CI requirements and public promotion
sequence. The [all-file source ledger](source-audit.json) records byte-inventory
coverage; the linked audits distinguish that from semantic review. Regenerate it
with `cargo xtask migration audit`; report current blockers with
`cargo xtask migration status`. `--require-complete` enforces both retained and
retired capability evidence plus removal of authored JS/TS outside reviewed
generated/vendor/fixture exceptions.

The [approved revised scope](scope.md) replaces the earlier compatibility host.
The Rust SDK and Wasmtime/WIT plugins replace JS interfaces; JS Eval is retired.
Python/shell execution and browser automation remain retained requirements.
Generated browser bindings and npm/Bun startup launchers are allowed. Retirement
is tracked separately from parity and remains pending until removal is verified.

## Implemented

- One Cargo workspace, lockfile, toolchain and build profile configuration for
  the existing native engines and the experimental application.
- `cargo xtask check`, `cargo xtask test` (nextest followed by doctests), and a
  capability inventory with a distribution cutover gate.
- `cargo xtask version` owns Cargo release version updates and first-party
  path dependency requirements, including canary releases and target-specific
  dependencies. Release preparation calls it before lockfile regeneration;
  Rust CI checks for drift without modifying the checkout.
- `cargo xtask npm-versions` replaces the old strict npm dependency synchronizer.
  Release preparation uses `--release VERSION` to update public packages, their
  explicit internal requirements and the Bun catalog while preserving private
  versions and catalog/workspace links. `--check --workspace` verifies agreement
  with Cargo without writes in Rust CI and release validation.
- Shared capability-relative filesystem operations, bounded reads/appends,
  atomic replacement, root writer locks and cancellation in `harvest-runtime`.
  `pi-shell` retains its cancellation API through re-exports.
- `harvest-brain`: Markdown records, five record kinds, scoped graph/BM25
  retrieval, evidence-bound deterministic admission, exact duplicate merging,
  protected authored/manual/pinned/safety/leased/dependency records, recoverable
  archive/restore/purge transactions, quotas and idle-attempt scheduling.
- Native brain CLI with both organizer spellings, JSON reports and dry runs.
  Explicit user lessons can be admitted with `brain learn`.
- The existing application now uses `harvest-brain` through a small N-API
  adapter. Its duplicate TypeScript scanner, BM25 setup and graph expansion have
  been removed. The legacy SDK message transform remains during migration;
  scope filtering now precedes ranking and applies through graph traversal.
- Bazel native-addon dependency targets include the shared Rust crates and
  their static prompt assets. Cargo native builds are verified locally; Bazel
  cross-builds still need CI verification because Bazel is unavailable here.
- Rust `xtask package` generates Homebrew and WinGet release metadata with
  validated versions, calendar dates and asset checksums. It replaces the two
  standalone Python generators. Secret scanning remains in the existing
  repository-wide Gitleaks security job and the prototype credential scanner.
  The blanket comment ban and arbitrary source-length gate have been removed.
  CI preserves the coverage floor, release tests, cross-target checks, strict
  Rustdoc, feature powerset, packaging and installer verification.
- Rust credential scanning preserves the prototype detection rules, redacts
  matched values and caps manifests, source sizes and findings. Read failures,
  escaping paths and oversized input fail the check. Both scanners run during
  migration; the existing Python scanner has not been removed.
- Both Rust binary aliases share a library. The prototype uses bounded I/O,
  jailed session writes, embedded Markdown prompts, ephemeral retrieval from
  project/user brains and host-owned approvals. Shell execution in `full`
  reuses `pi-shell` with a bounded channel, capped capture and a timeout.
- A bounded session codec reads title slots and v3 trees, preserves original
  bytes and raw unknown/provider values, and migrates v1/v2 entries in memory.
  `harvest session --workspace <root> --leaf <id> <file>` reports a selected
  branch without rewriting its source. The experimental Rust `AgentSession` now
  replays text/function branches, streams OpenAI-compatible SSE, executes approved
  tools and durably appends v3 messages. Ownership locks, content comparisons,
  generation-specific handles and committed events are tested. Signed/multimodal
  replay, compaction, imports, crash repair and large-log streaming remain pending.
- `harvest-terminal` shares legacy/Kitty keyboard decoding, selector ranking, SGR coalescing, ConPTY UTF-16
  chunking, bounded resumable escape scanning, printable Kitty/xterm text decoding
  and safe carried-width padding with the native addon. The current application
  routes fresh protocol strings of at least 4,096 UTF-16 units through Rust.
  Small packets, raw text and resumed scans stay local because paired benchmarks
  found N-API regressions there. Printable decoding is available in the shared
  Rust engine and binding API; its TypeScript consumer cutover is pending.
  The 441-line authored TypeScript selector implementation and its transitional
  fallback are deleted. Generated bindings use the same Rust matcher through
  N-API or an embedded WASM payload. The TypeScript ConPTY fallback is also
  deleted; missing-addon installations use Rust/WASM output chunking.
  `cargo xtask terminal-wasm` owns payload generation and declaration files;
  `--check` rejects stale generated assets. Scanner fallbacks remain until their
  replacement satisfies input performance gates.
- TypeScript and experimental Rust sessions share the same extracted kernel
  ownership engine, canonical identity and `.lock` namespace. Missing leaves
  beneath symlinked ancestors are resolved inside the filesystem capability.
  The actual TypeScript-to-Rust writer handoff is covered by an integration test.
- Experimental provider transport performs network I/O on a shared asynchronous
  runtime with bounded chunks/queues. Cancelling drops pending header/body reads;
  text callbacks remain on the calling host thread and support reentrancy.
- `cargo xtask checksums` replaces the TypeScript release checksum generator,
  validates all eight nonempty assets and atomically writes `SHA256SUMS.txt`.
  Rust archive preparation streams both executable aliases into deterministic
  ZIPs for cargo-binstall and WinGet, retaining standalone binary downloads.
- Rust PR checks run for all pull requests. Windows x64 Bun artifacts are smoked
  on a Windows host. Experimental Rust releases use root workspace paths, all
  features, the eight production asset names and an existing authoritative tag.
  They no longer create separate version bumps, commits or tags. Installer and
  Homebrew/WinGet consumers now agree with those names and `SHA256SUMS.txt`.
- Help/version have no setup side effects. Tag and manual public release requests
  now share one parity-gated Rust distribution workflow, publishing lock and
  immutable candidate commit. Publication rechecks capability/source readiness
  and rejects replacement of an existing release. Full browser/plugin/npm/signing
  and installed-product promotion stages remain implementation work.

## Remaining before cutover

The manifest includes 41 legacy commands, 29 canonical tools, three hidden tools,
84 slash entries and integration surfaces. The expanded 475-entry source-surface
inventory adds public SDK import paths/functions, browser routes and views,
compatibility behavior, concrete worker hosts and generators. Schema 3 now tracks
284 retained capabilities and 210 intentionally retired interfaces across 494
entries. Rust SDK/plugin replacements and the remaining operations stay retained;
no retirement is marked as parity or verified removal. An inventoried
source entry is not evidence of behavioral parity. Primitive Rust tools are partial:
legacy read/edit/grep/bash behavior and settings still have richer contracts.

Complete provider streaming/OAuth/catalog KDL resolution; compatible session
imports, fork/compaction, recovery and ownership handoff/draining under a complete production runtime; RPC/ACP;
the inline terminal; delegation,
goals/advisor; MCP/LSP/DAP; browser/WASM/eval/SSH; collaboration; media/inference;
stats/benchmarks; Python automation; generators and consolidated distribution
still require ports and verification. The Rust library is experimental and does
not yet expose the approved complete `AgentSession` interface or Rust/WASM plugin contract.

Automatic session extraction is **not connected to a production event loop**.
The admission/ledger APIs and one-attempt idle scheduler exist, but the bounded
provider extraction call, committed-outcome adapters, safe inferred global
promotion and legacy skill-management wiring remain implementation work.
Repository identities conservatively unify origin URLs across transport; missing
origins, includes and inaccessible worktree metadata cannot support promotion.
Existing Markdown is indexed non-destructively; legacy revision histories remain
untouched and have not been migrated into the new transaction ledger.

Automatic retirement is limited to generated records with trustworthy receipts.
It never purges. Existing records without receipts are protected, and indexing
incomplete or malformed dependency inventories prevents organization.
Production retrieval does not yet register active-record leases or application
outcomes. Cross-scope dependency protection covers the selected workspace and
user roots; a registry covering other projects' reverse dependencies remains
required before global automatic maintenance can satisfy the complete plan.

## Verification

```sh
cargo xtask test -p harvest-runtime -p harvest-brain -p harvest-omp
cargo xtask test -p harvest-omp --features full
cargo clippy --locked -p harvest-runtime -p harvest-brain -p harvest-omp \
  -p xtask --all-targets --all-features -- -D warnings
cargo xtask capabilities
# Intentionally fails until all parity gates are met:
cargo xtask capabilities --require-complete
```

`capabilities.json` is a conservative inventory, not a test-results database.
Schema v3 reports retained implementation progress independently of intentional
interface retirement and requires verified removal as a separate cutover gate.
Partial entries name Rust paths, contract suites, experimental installation paths
and legacy counterparts. `cargo xtask capabilities` prints both counts. A verified
entry requires complete implementation, contract evidence, default Rust installation
and a removable legacy counterpart; missing evidence fails the gate. These fields
record reviewed evidence, rather than automatically certifying every test run.
The installed full-profile workflows must also pass before distribution
cutover. CI retains cross-platform checks; local measurements alone cannot prove
macOS/Windows parity.

Local migration verification is recorded in [verification.md](verification.md).
The complete browser/WASM, installed-artifact and cross-platform gates have not
run. First-party path libraries now have synchronized registry requirements.
A real full-feature `cargo package` still cannot resolve the unpublished
dependencies; the patched shell fork also requires an owned registry dependency
closure. Multi-package staging encountered a pinned-Cargo checksum error.
The release preflight checks a real package,
so listing archive paths cannot certify release readiness.

The proposed binary/RSS/paint/retrieval performance targets are not yet certified.
No legacy application entrypoints were removed. Distribution remains blocked
until capability parity is verified; no release has been published.
