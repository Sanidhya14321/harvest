# Rust migration implementation status

The normal application still launches TypeScript. This change establishes the
shared Rust foundations and native brain maintenance; it does not complete the
application rewrite. No capability is marked as legacy-parity complete simply
because a Rust command or tool has the same name.

## Implemented

- `harvest-tui::mouse` decodes SGR clicks, release, drag and vertical-only
  wheel reports directly from Rust. Installed Rust (without a JS runtime),
  rebuilt addon and frontend each match 2,637 legacy reports, including
  malformed reports, zero-based coordinates, huge numeric fields and modifiers.
  The existing frontend decoder remains for older/unavailable optional addons;
  component hit-test routing is still TypeScript. All 100 TUI tests, four
  doctests, 191 combined Rust tests, 97 focused frontend tests, strict Clippy
  and package gates pass. Warmed 20,000-event probes measured old 4.56/5.04 ms
  and shared 17.64/21.51 ms: host bridge overhead is explicit, with no claimed
  speedup. Full interactive terminal parity and legacy removal remain pending.

- `harvest-tui::highlight` shares the existing syntax engine with ordinary Rust
  consumers. All five bundled grammar assets moved byte-for-byte; N-API only
  converts inputs, owns its inline palette and schedules warmup. Palettes are
  borrowed per push without retaining host handles. Installed Rust and rebuilt
  addon probes match 288 whole-code cases and 720 streamed chunks over 32
  language/alias paths, three palette modes and unknown/empty-input boundaries.
  Async warmup and palette overflow/recovery pass. All 332 shared Rust tests,
  96 Bazel TUI tests, three doctests, strict Clippy and package gates pass;
  native alias/stream and colored Markdown/diff checks pass. The diff test now
  supplies an explicit color mode instead of depending on ambient `NO_COLOR`.
  Warmed 200-call probes measured old 48.64/48.83 ms and shared 49.76/50.48 ms;
  this small bridge overhead is recorded without a whole-app performance claim.
  Native Markdown widgets and complete interactive renderer parity remain pending.

- `harvest-tui::deccara` owns the exported rectangular-background planner;
  the TypeScript module is now a thin snapshot/cutoff adapter. This helper is
  currently exported by the SDK, not wired into the main TUI renderer. Native
  and rebuilt-addon probes match 3,745 corrected legacy frame plans and 8,424
  analyses; 3,313 original well-formed frame fixtures are unchanged. The
  independent surrogate-width fix covers the remaining malformed UTF-16 cases.
  Kitty terminal-core emulation exposed and verifies the fix for lost inverse
  and underlined padding. Unsupported/decorated SGR, oversized frames and
  overflowing coordinates retain complete original rows. All 328 shared Rust
  tests/doctests, 127 focused TS contracts, package gates and strict Clippy pass.
  A warmed 100-frame/40-row probe measured old medians 5.82/9.94 ms and shared
  medians 5.94/5.64 ms; this overlapping microbenchmark is not whole-app proof.
  Platform terminal capability discovery and complete renderer parity remain
  pending; the legacy host capability decision and adapter are retained.

- `harvest-tui::text` now owns ANSI-aware width, wrapping, truncation and
  column/overlay slicing. The Rust SDK calls it directly; N-API retains a thin
  UTF-16 adapter and its existing output convention. Unchanged truncation
  borrows its input; huge column ranges reserve against existing text instead
  of requesting gigabytes. All 317 shared Rust tests/doctests, the 81-test
  Bazel TUI harness, strict Clippy, both package gates and 119 focused TUI/
  fallback contracts pass. Installed core and rebuilt-addon probes each match
  3,074 legacy operations, including malformed UTF-16 and terminal sequences.
  Warmed CPU-affined 18,000-operation probes overlap (old 16.98/18.81 ms,
  shared engine 17.39/17.13 ms); no whole-app speedup is claimed. Widget layout,
  renderer lifecycle, restoration and installed terminal parity remain pending.

- `harvest-tokenizer` extracts the existing offline counting engine, embedded
  vocabularies and reference tests from N-API. The Rust SDK exports it directly;
  the legacy addon delegates to the same library. All eight vocabulary blobs
  are unchanged. An installed native probe matches 14,020 UTF-8/UTF-16 legacy
  counts over 701 inputs without Bun/Node on PATH; the rebuilt addon matches
  7,010 counts. All 295 shared Rust tests/doctests, 38 tokenizer tests in the
  Bazel harness, strict Clippy, the native package gate and 24 focused addon/
  agent tests pass. Fifteen stale expectations from five renamed Claude texts
  were corrected against all 1,479 pinned `ctok==1.0.0` references, without a
  runtime algorithm change. Warmed, CPU-affined 10,000-call probes measured
  old medians 100.7/105.3 ms and shared-engine medians 103.4/109.7 ms; this is
  narrow overhead evidence, not an end-to-end performance claim. Catalog
  selection, message/frame accounting and compaction parity remain pending.

- `cargo xtask emit-extensions --directory DIR [--kind types|js] [--check]`
  repairs ESM specifiers in emitted package output. The retained publisher
  invokes Rust for both declaration and runtime output; the old TS helper is
  deleted. All 73 xtask tests, strict Clippy, root tools gate and six publisher
  tests pass. Installed Linux tooling runs without Bun/Node on PATH and matches
  22 fixture rewrites. Real tsgo output matches all 26 omptype files and 52
  rewrites, including paths with spaces; Node ESM loads the repaired runtime
  graph and correctly accepts/rejects schema inputs. Traversal/file/plan bounds,
  lexical escapes, invalid UTF-8, external symlinks and read-only drift checks
  have Rust contracts. The publisher and overall release cutover remain pending.

- `cargo xtask theme-index [--check]` owns bundled theme registry generation,
  including release regeneration and Rust CI drift checks. It fixes the stale
  defaults path and preserves all 98 current exported names and palette values.
  Installed no-runtime generator probes cover identifier collisions, reserved
  names, prototype keys, deleted assets and read-only stale checks; 67 xtask
  tests, strict Clippy, the package gate and 32 theme/startup/export tests pass.
  The old generator is removed; its generated TypeScript registry remains until
  the frontend consumers migrate.

- `cargo xtask fix-elf-verdef --workspace ROOT FILE...` repairs stale GNU
  version-definition pointers through bounded, jailed, atomic streaming writes.
  Invalid metadata, reader failures and unwinds leave the original intact;
  outside hardlink aliases are preserved. All 193 shared Rust regressions and
  19 rebuilt-addon regressions passed. Five installed legacy-oracle cases match;
  controlled x64 and emulated ARM64 glibc 2.17 loader failures recover without
  Bun/Node on the repair command's PATH. The host patchelf chain kept its pointer
  current, so this is deliberate-corruption recovery evidence. The clean Nix
  Linux x64 consumer build passed: the real auto-patchelf chain relocated
  DT_VERDEF, native repair corrected it, and the installed CLI version/help
  checks passed. The old script/test are removed.

- `cargo xtask clippy-config [--check]` owns Bazel lint policy generation.
  Root/release callers and Rust CI use it; the TS generator is removed. Five
  installed legacy-oracle cases preserve generated flags, and missing/stale
  checks leave files untouched without Bun/Node on PATH. All 55 xtask tests,
  strict Clippy and an actual Bazel strict-Clippy build passed. Model catalogs,
  compatibility compilation, browser assets and other generators remain pending;
  the overall codegen capability remains unverified.

- The Rust SDK and legacy TUI now share bounded terminal input framing, paste
  recovery and Escape deadlines through `harvest-tui::input`. The TS adapter
  schedules host timers and delivers events. Shared Rust tests, legacy framing
  and editor suites cover fragmented protocols, same-read paste/Enter delivery,
  watchdogs and discarded OSC tails. Shared output byte chunking and backlog
  progress detection preserve Unicode and keep slow draining frames alive.
  A shared bounded output pump now accepts Rust-owned backends directly, keeps
  in-flight bytes inside its cap and releases detached backend ownership once
  blocked I/O resumes. The legacy Unix adapter uses the same queue.
  Terminal lifecycle, renderer, resize,
  restoration and native interactive installation still require implementation
  and verification; terminal capability parity remains unverified.

- Native distribution uses one eight-target registry for release/CI matrices,
  package names, Homebrew bindings and both WinGet architectures. ZIPs include
  both real aliases, deterministic metadata and legal notices; validation checks
  all targets, alias bytes/modes, headers and legal files. WinGet generation and
  submitted-archive verification run through Rust and reject unknown manifest
  fields or changed downloads before extraction. Canonical bootstrap consumers
  use `SHA256SUMS.txt`. Local fixtures passed 176 combined Rust tests, 90 core
  app tests and shell installer routing/upgrade/refusal checks. A full release
  build completed, and real cargo-binstall 1.26.0 installed both aliases from a
  loopback archive; execution and completion setup worked without Bun/Node.
  PowerShell files passed the real parser and the non-Windows refusal contract;
  native Windows/macOS runs, GNU portability-floor builds and Cargo publication
  dependency closure remain part of distribution parity. See
  [distribution contracts](distribution.md).

- Native settings/override/environment diagnostics omit API keys and endpoint
  URLs. Invalid TOML errors retain only the config filename and byte offset,
  dropping parser source-line/error chains; malformed environment step counts
  no longer echo their input. SDK and real CLI failure contracts cover both
  aliases, no session creation and unchanged config bytes. Eight installed
  credential-bearing parse failures passed without Bun/Node. This does not
  implement credential storage, OAuth or compatible profile/settings migration.

- `cargo xtask release-notes VERSION [OUTPUT]` replaces the old TS release-notes
  script and supplies curated bodies to both release workflows. It merges
  unpublished-tag changelog sections through the target, deduplicates entries,
  and includes native crate/application changelogs as well as legacy packages.
  Automatic floor lookup uses authenticated `gh release list` and fails closed;
  `--floor VERSION`, `--single-version` and `--published-releases JSON_FILE`
  support explicit/offline runs. Seven legacy-oracle cases also passed through
  installed xtask without Bun/Node. All 41 xtask tests, strict Clippy and workflow
  YAML validation passed; no GitHub release was published during verification.

- Native session replay projects text compactions, handoffs, branch summaries
  and durable context resets onto the selected branch without rewriting history.
  Eight legacy-oracle cases match, including reset/compaction precedence and
  sibling isolation. Missing/forward/sibling kept references, active opaque
  compaction payloads and broken tool pairs fail before writing. All 16 installed
  alias/case continuations passed without Bun/Node; app/runtime/brain/xtask passed
  155 tests and the explicit core app passed 85. Custom messages, inferred
  settings, general prompt formatting, provider replay, bitmap archives and
  automatic compaction generation remain pending; session parity is unverified.

- Native grep delegates traversal and ignore matching to `pi-walker`; file bytes
  still pass through the workspace capability. Explicit `case`/`gitignore`
  options, cancellation, file/encoding limits, and search-limit notices have
  host contracts and an installed model/tool/result workflow. Shared Unix read,
  optional-read and hashing operations reject FIFOs before they can block.
  Tool prompts are static Markdown assets. The combined Rust suite passed 149
  tests, core app 79, and rebuilt-addon legacy brain/edit suites 22. Grep's
  remote/archive/selector, advanced matching, pagination and renderer contracts
  remain pending; no legacy grep source or parity status was removed.

- Shared native HTTP request construction returns typed status/transport errors
  without attaching raw URL/status/body diagnostics; `HttpModel` debug output
  omits configured credentials. Real loopback 401/429 failures, invalid schemes
  and rejected JSON/SSE finish reasons exercise both SDK request paths. This
  does not implement credential storage, OAuth refresh or provider retry policy.
  The 72-test app suite, strict Clippy and installed HTTP-failure checks through
  both aliases passed; installed failures preserve the user turn and commit no
  assistant output. The unchanged legacy usage-attribution suite passed 22 tests.

- Native OpenAI-compatible streaming and JSON responses preserve raw usage
  metadata, normalize complete integer token/cache reports and persist them
  before committed events. Unknown numeric metadata remains lossless and stays
  out of replayed model requests. `--request-usage` explicitly enables the wire
  request option; the default preserves compatibility with unsupported hosts.
  Seven accounting cases captured from the legacy helper match both installed
  aliases without runtimes on PATH. Partial/invalid reports retain raw bytes and
  remain unavailable; catalog pricing and other provider families remain pending.
  Explicitly unsuccessful JSON responses cannot commit assistant output.

- `cargo xtask dev <check|fmt|lint|fix|test>` replaces the Bun Rust-task launcher.
  Root aliases, the pooled test runner and CI invoke it. It preserves the
  vendored-fork exclusion, local change skips, CI forcing and Rustup Cargo
  selection; bounded Git failures run gates conservatively. New untracked
  directories are inspected recursively. Real Cargo fixture contracts verify
  repair/recheck and nextest/doctest failure ordering. Installed commands run
  without Bun or Node; remote macOS/Windows execution remains pending.

- Native `session --title TEXT FILE` updates or adds a fixed-width title slot
  under the Rust writer lease, preserving every remaining session byte,
  including unknown signed/provider records. Unicode truncation, clearing,
  malformed/legacy/tree rejection, active-writer exclusion and symlink safety
  are covered. Resume now validates all branches before opening a writer.
  The combined app/runtime/xtask suite passed 99 tests; both installed aliases
  ran without runtimes, and the legacy title reader accepted their output.
  Cached prefix overlays and legacy writer coordination remain pending.

- Both release checksum consumers and the package alias use `cargo xtask
  checksums OUTPUT ASSET...`. The TS generator and its tests are removed.
  The Rust command streams authorized regular files with bounded memory, emits
  deterministic sha256sum-compatible basenames, and rejects duplicate, unsafe,
  escaping or missing inputs before atomic manifest replacement. The installed
  Linux command works without runtimes on PATH; `sha256sum --check` verified its
  output. The 36-test runtime/xtask suite and strict Clippy passed. Remote
  release and cross-platform execution remain unverified.

- `cargo xtask version` reads the root inherited version; `--set VERSION
  --expect CURRENT` changes it atomically under a writer lock while preserving
  TOML comments and unrelated fields. Rust release jobs use root manifest,
  lockfile and target paths, explicit application package selectors, and the
  nextest/doctest gate. Release remains blocked while the legacy release workflow
  owns tags or source/capability parity is incomplete. The crate publication
  dependency graph, installer naming, complete platform matrix and actual remote
  release validation remain pending. Local validation covers 21 xtask contracts,
  strict Clippy, and a two-member fixture whose inherited versions were checked
  through Cargo metadata after running the updater without runtimes on PATH.
  Workflow YAML parses locally; hosted release jobs have not been executed.

- One Cargo workspace, lockfile, toolchain and build profile configuration for
  the existing native engines and the experimental application.
- `cargo xtask check`, `cargo xtask test` (nextest followed by doctests), and a
  capability inventory with a distribution cutover gate. Focused test target
  selectors apply to nextest while doctests retain package/build options;
  `--test session_recovery` no longer fails by mixing `--doc` and target flags.
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
  escaping paths and oversized input fail the check. The redundant Python scanner
  and its dedicated lint configuration/job have been removed after Rust contract
  and installed-binary verification. Hygiene now uses the Rust scanner alone;
  repository-wide Gitleaks remains unchanged.
- Both Rust binary aliases share a library. The prototype uses bounded I/O,
  jailed session writes, embedded Markdown prompts, ephemeral retrieval from
  project/user brains and host-owned approvals. Shell execution in `full`
  reuses `pi-shell` with a bounded channel, capped capture and a timeout. Native
  builds now enable `full` by default; `--no-default-features --features core`
  keeps the explicit lightweight build. Default/core application suites passed
  57/56 tests. A default-installed binary completed an approved shell round trip
  through a PTY, evaluated arithmetic in the workspace and durably recorded the
  tool result without runtimes on `PATH`. Other platform shipping checks remain
  pending.
- Native replacement edits now call `pi-edit` directly through capability-relative,
  bounded host reads and atomic writes. BOM/CRLF and notebook preservation,
  generated-file protection, ambiguous matches, cancellation and symlink escapes
  have native application contract tests. Other edit modes, streaming previews,
  LSP/ACP write-through and freshness integration still require host ports.
  Local core tests passed (324); full-profile application/edit/AST tests passed
  (408, one skipped), with doctest passes. Strict linting and formatting passed
  on the changed Rust packages; installed cross-platform parity remains pending.
- A bounded session codec reads title slots and v3 trees, preserves original
  bytes and raw unknown/provider values, and migrates v1/v2 entries in memory.
  `harvest session --workspace <root> --leaf <id> <file>` reports a selected
  branch without rewriting its source. The experimental Rust `AgentSession` now
  replays text/function branches, streams OpenAI-compatible SSE, executes approved
  tools and durably appends v3 messages. Ownership locks, content comparisons,
  generation-specific handles and committed events are tested. Explicit
  `session --import-v3` exports all branches to a new file, rejects malformed records and invalid
  parent chains, preserves source bytes and unknown values, and never overwrites
  a destination. Shared atomic creation publishes fully synced content using
  no-clobber hard links. Explicit `session --recover-to` preserves the raw v3
  prefix and original source, reports the source hash and excluded tail size,
  and rejects interior corruption, broken trees and complete final records.
  Both snapshot commands share the cooperating Rust writer lease; unresolved
  tool calls still block resume. Recovery has seven CLI contracts; the full
  application suite passed 57 tests and strict linting. Installed full-profile
  recovery/continuation passed using a local provider fixture with no runtimes
  on `PATH`. The rebuilt native addon also passed eight legacy edit tests. Signed/multimodal replay, compaction, automatic
  repair, legacy ownership coordination and large-log streaming remain pending. Local installed full-profile `harvest` and
  `omp` passed v1 import, local-provider replay/continuation, durable writes,
  source preservation and overwrite rejection with no runtimes on `PATH`.
  This does not certify live providers or other operating systems.
- Help/version have no setup side effects. Experimental release automation now
  requires capability parity and is explicitly dispatched instead of publishing
  an incomplete application on each source push.

- Protocol generation now uses explicit message/enum roots in
  `packages/catalog/scripts/protocol-roots.json`, replacing Cursor TS-import
  scanning. Cursor/Devin output remains byte-identical when generated in an
  isolated tree containing only the generator, parser, roots and `.proto` files.
  Unknown roots fail before outputs are written. The compiler itself and its
  generated TypeScript runtime still require Rust replacements.

## Remaining before cutover

The manifest includes 41 legacy commands, 29 canonical tools, three hidden tools,
84 slash entries and integration surfaces. Primitive Rust tools are partial:
legacy read/edit/grep/bash behavior and settings still have richer contracts.

Complete provider streaming/OAuth/catalog KDL resolution; remaining session
import/replay adapters, fork/compaction, recovery and legacy ownership integration; RPC/ACP;
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
cargo xtask test -p harvest-omp --test session_recovery
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
The initial inventory contains 5,220 paths. Reviewed entries retain their evidence
while other paths require individual contract review; this enumeration does not
establish that step 1 of the migration is complete.

Run `cargo xtask migration` to validate coverage and deletion evidence, or
`cargo xtask migration --require-complete` to enforce the source cutover gate.
Rust CI runs the coverage gate independently of capability parity. The command
uses the Git index, so stage intended deletions before checking removal evidence.
New tracked source files require ledger entries. Baseline paths must never be
removed just because their source was deleted; retain their completed records.
The gate checks the baseline against the original Git tree, so shrinking both
ledger and baseline cannot hide deletions. Shallow checkouts must fetch the
recorded baseline commit; CI checks and release parity fetch history for this.
Rust release requires both source completion and capability parity before it
can create a release commit or tag.

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
