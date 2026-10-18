# Rust migration implementation status

The normal application still launches TypeScript. This change establishes the
shared Rust foundations and native brain maintenance; it does not complete the
application rewrite. No capability is marked as legacy-parity complete simply
because a Rust command or tool has the same name.

## Implemented

- One Cargo workspace, lockfile, toolchain and build profile configuration for
  the existing native engines and the experimental application.
- `cargo xtask check`, `cargo xtask test` (nextest followed by doctests), and a
  capability inventory with a distribution cutover gate.
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
- Bazel native-addon dependency targets include both shared Rust crates and
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
- Help/version have no setup side effects. Experimental release automation now
  requires capability parity and is explicitly dispatched instead of publishing
  an incomplete application on each source push.

## Remaining before cutover

The manifest includes 41 legacy commands, 29 canonical tools, three hidden tools,
84 slash entries and integration surfaces. Primitive Rust tools are partial:
legacy read/edit/grep/bash behavior and settings still have richer contracts.

Complete provider streaming/OAuth/catalog KDL resolution; compatible session
imports, fork/compaction, recovery and legacy ownership integration; RPC/ACP;
the inline terminal; delegation,
goals/advisor; MCP/LSP/DAP; browser/WASM/eval/SSH; collaboration; media/inference;
stats/benchmarks; Python automation; generators and consolidated distribution
still require ports and verification. The Rust library is experimental and does
not yet expose the approved complete `AgentSession` interface or process ABI v1.

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
Schema v2 reports implementation progress independently of compatibility.
Partial entries name Rust paths, contract suites, experimental installation paths
and legacy counterparts. `cargo xtask capabilities` prints both counts. A verified
entry requires complete implementation, contract evidence, default Rust installation
and a removable legacy counterpart; missing evidence fails the gate. These fields
record reviewed evidence, rather than automatically certifying every test run.
The installed full-profile workflows must also pass before distribution
cutover. CI retains cross-platform checks; local measurements alone cannot prove
macOS/Windows parity.

The proposed binary/RSS/paint/retrieval performance targets are not yet certified.
No legacy application entrypoints were removed. Distribution remains blocked
until capability parity is verified; no release has been published.

## Source migration ledger

`source-baseline.json` freezes the tracked TS/TSX/MTS/CTS, JavaScript,
Python and Julia paths at its recorded commit, including declarations, fixtures,
examples and generated sources. `source-ledger.json` retains one entry per path.
The initial inventory contains 5,220 paths. All require individual contract review;
this enumeration does not establish that step 1 of the migration is complete.

Run `cargo xtask migration` to validate coverage and deletion evidence, or
`cargo xtask migration --require-complete` to enforce the source cutover gate.
Rust CI runs the coverage gate independently of capability parity. The command
uses the Git index, so stage intended deletions before checking removal evidence.
New tracked source files require ledger entries. Baseline paths must never be
removed just because their source was deleted; retain their completed records.
Changes to the baseline itself require review against the recorded Git commit.

For each path, review its behavior, consumers, persisted formats, dependencies,
Rust destination and verification requirements. Use explicit `none` where a
review finds no persisted format or dependency; null means unreviewed. Record
replacement, intentional retirement, fixture materialization or browser glue as
separate dispositions. Retirement needs a rationale; all other reviewed entries
need a destination. Record concrete contract suites and installed-workflow evidence
in verification requirements before changing their independent status fields.
Evidence strings are reviewed assertions, not an automatic certification of tests.

Implementation, behavioral verification, installed verification and legacy removal
are independent. Deletion requires completed implementation (or retirement work),
both verification stages and reviewed contract analysis. Fixture sources must move
to data assets and be materialized by Rust tests. Only documented browser bootstrap
JavaScript may remain as browser glue; application logic and TypeScript cannot use
that exception. Capability parity, installed workflows, platform validation and
release consolidation remain separate mandatory gates.

Initial ledger validation: pinned `nightly-2026-08-08` formatting, strict
Clippy and `cargo xtask test -p xtask` passed (11 tests plus the doctest pass).
`cargo xtask migration` reports 0/5,220 completed entries, and the completion
gate rejects that unfinished inventory as intended. CI includes xtask tests in
its cross-platform core/full matrix; those remote runs remain pending.
