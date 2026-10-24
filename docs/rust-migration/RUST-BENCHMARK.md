# rust-v1 edit benchmark conversion checkpoint

Native task loading, patch application, scoring and model-run orchestration are
written and connected; behavioral and installed verification are pending. The legacy metaharness runner
still uses its historical scoring. No existing score is converted or relabeled.

`harvest edit-benchmark list --fixtures DIRECTORY_OR_TAR_GZ` loads tasks with
`prompt.md`, `input/`, `expected/` and optional `metadata.json`. Traversal is
sorted, bounded and rejects symlinks. Owned benchmark inputs remain inert data;
the preserved fixture tarball and historical result file are unchanged.

`harvest edit-benchmark compare --expected DIRECTORY --actual DIRECTORY` checks
file sets and exact content, normalizing CRLF to LF only. Every other whitespace,
blank line and final-newline distinction remains significant.

`harvest edit-benchmark apply --expected DIRECTORY --actual ISOLATED_WORKSPACE
--manifest FILE.json` applies ordered replacements using the existing jailed
atomic edit engine. This command intentionally mutates the supplied actual
workspace. Stop at the first failed edit and report remaining edits as skipped.
The result separates patch application, output matching, elapsed microseconds and
optional usage. Compare-only results do not claim patch-application evidence.

The manifest has `scoring_version: "rust-v1"`, an `edits` array of
`{path, old, new}`, and optional `usage`. Usage is kept losslessly; only a report
with valid `input_tokens` and `output_tokens` makes measured usage available.
Unknown/partial/overflowing usage stays unavailable. The scorer never adds pricing
or combines versions. Result failures return nonzero CLI status.

## Native model runs (conversion checkpoint, unverified)

```text
harvest edit-benchmark run --fixtures /path/to/unpacked/tasks --output /path/to/new-run --repetitions 3 --concurrency 2 --approve-fixture-edits
```

The CLI uses the shared native configuration resolver, profile/overlay selection,
environment credentials, explicit API selection and provider transport. It does
not start configured plugins/MCP/language services/browser clients. `--model`,
`--base-url`, `--max-steps`, `--profile`, repeated `--config` and exact repeated
`--task` selections are supported. Credentials are not serialized into reports.
SDK consumers call `run_tasks` with a per-run model factory, shared cancellation
owner and final-argument approval policy. Custom models must cooperate with
cancellation; the native HTTP implementation uses cancellable I/O.

Tasks are sorted, repetitions indexed, and results sorted by index independent of
worker completion order. Limits: 1..100 repetitions, 1..16 workers, 1..1024 turns,
positive timeout up to an hour, at most 1000 total runs. Each worker owns its own
model/session/workspace. Cancellation preserves partial outcomes; run failures
continue independent runs. Artifact I/O failures fail the affected run; failures to write its outcome or
initialize output ownership are fatal and preserve prior artifacts. Output must be empty and disjoint from fixtures, with one writer lease;
resuming an interrupted output directory is deliberately unsupported.

Only read/grep/write/edit are advertised. Unadvertised mutation requests are
denied. `--approve-fixture-edits` grants host authorization only for jailed final
write/edit arguments; otherwise terminal approval applies and CLI concurrency is
one. SDK hosts provide their own approval owner. This is filesystem isolation
through the shared tool jail, not an OS sandbox for trusted custom SDK models.
Expected fixtures, sessions and run metadata are outside the model workspace.
Fixture symlinks are rejected; input copies use bounded reads and atomic creates.

Artifacts: `run-plan.json`, per-run `workspace/`, `session.jsonl`, bounded
`events.jsonl`, `usage.json`, `score.json` on successful execution,
`outcome.json`, and final `summary.json`. Exact output matching remains independent
of provider usage. Model-run patch evidence now comes from actual persisted edit calls/results as
described below; ordered manifest application retains per-operation outcomes. Sessions
retain actual tool results. No attempt is made to infer patch success from a
model's prose. Unknown/missing/overflowing usage and failed turns remain unavailable;
partial raw response usage is retained. Cache accounting distinctions are retained
in measured token totals. Prices remain unavailable. A completed output mismatch
is unsuccessful, distinct from cancellation, timeout and failed execution. CLI
returns nonzero if any run is unsuccessful. No expected fixture is executed.

Pending: full language-specific mutation eligibility, provider/timeout retries and guided mutations,
connection/first-event timeout policies, formatted conversation exports, interactive
benchmark views and metaharness consumer cutover. Global artifact/disk quotas, crash resume and custom-model
cancellation enforcement also need explicit contracts and stabilization evidence. Babel/Prettier historical scores retain their old meanings.
No TS benchmark source is removed before those remaining consumers move.

## Native fixture tooling and edit-shape continuation

```text
harvest edit-benchmark edit-shapes --sessions-dir /path/to/sessions --max-sessions 2000 --json
harvest edit-benchmark validate --fixtures /path/to/unpacked/tasks
harvest edit-benchmark prepare --input /path/to/input --expected /path/to/expected --output /path/to/new-fixtures --task-id reviewed-case
```

These are conversion interfaces, with behavioral/installed verification pending.
They share the existing task loader, filesystem jail and pi-diff line engine.
No new parser/diff implementation or external language runtime is introduced.
The `edit-shapes` package command now invokes the Rust CLI; the standalone
355-line `src/edit-shape-stats.ts` is deleted. Its ledger row remains pending
verification, with the actual removal recorded. Other benchmark TS sources stay
because the generator and metaharness still use them.

Edit-shape analysis reads bounded JSONL records, correlates edit/apply_patch IDs
with paths, and parses numbered pipe/older space and unified diffs. Context and
elisions close runs. Changed lines sum max(old,new) per run. Per-call distributions
are calibration references; per-request/file totals include retries and reversals
and remain activity upper bounds. Size/hunk/operation buckets and the historical
upper median are preserved. Text and JSON expose parse coverage, missing diffs,
malformed records and failed files; zero observations and incomplete file scans
return nonzero. No NaN percentages are emitted for empty buckets. Native edit results now persist bounded staged diff details through both native
session hosts. Missing/oversized diffs remain counted separately; unchanged
operations have their own count and never become measured changed runs.

Session selection is newest filename first. `--max-sessions 0` scans all within
the explicit 10,000-file budget. Per-record limit is 1 MiB, per-file 64 MiB, total
scan 1 GiB, and samples/pending request runs are bounded. The CLI retains `--repo`
for read-only default-directory discovery using canonical home/temp/absolute and
historical hashed names, profile/agent-dir and existing XDG data roots. Ambiguous
or missing layouts require explicit `--sessions-dir`. Unlike the old utility's
application storage opener, analysis never creates, merges or migrates stored
session roots. Session storage migrations remain application-owned.

Fixture validation collects structured per-task issues for prompts, input and
expected trees, nonempty files, JSON metadata and metadata target presence.
Missing metadata remains a validation issue even though the general loader allows
it. Reads and traversal are bounded; symlinks and special inputs are rejected.
It never repairs or executes fixture data.

Paired-tree preparation accepts identical nonempty file sets with at least one
changed UTF-8 file. Unchanged binary files remain inert copies. It generates
placements using pi-diff, merges nearby changes, anchors insertion/deletion
blocks, extends repeated blocks or supplies an explicit starting line, and
re-solves every hunk to require exact expected bytes. Fences exceed embedded
backtick runs. Prompt instructions live in the static Handlebars Markdown asset.
Source fingerprints and producer `rust-paired-tree-v1` are recorded in metadata;
no old mutation IDs/scores are relabeled. Prompt publication is last, output has
one writer and must be empty/disjoint, and every copied file is created atomically.
A cancelled/failed partial directory is retained for inspection and must be
replaced with a fresh destination before retrying.

Limits: 256 KiB per changed source, 4,000 combined input/expected lines and at most
1,000,000 line-pair work budget; 128 MiB copied data, 256 KiB prompt and metadata.
Unrepresentable changes, including some final-newline-only changes, fail explicitly.
This is a deterministic paired-tree producer, not the unported language-specific
mutation selector. Native mutation selection and guided/retry/metaharness workflows remain pending;
archive materialization is now connected as described below. Stabilization must exercise real legacy
fixtures, parser failure/coverage, grouping, empty cases, source immutability,
ambiguous/overlapping hunks, exact final-newline behavior, cancellation and copied
installed commands before source removal is marked verified.


## Connected native result evidence and fixture archives

This continuation connects the previously named metadata/packaging prerequisites.
It remains **unverified conversion work**, with consolidated behavioral, installed
and platform checks pending.

The edit engine already computes the staged diff. `ToolOutput` now carries that
host-produced metadata separately from model text, plus an explicit error status.
Both synchronous and asynchronous session drivers commit details and error status
in the same durable tool-result record. Read results beginning with `error:` are
still successful reads when the actual invocation succeeded. Raw message export
and native RPC committed records retain these fields; provider replay text keeps
its existing shape. Existing string-returning tool SDK entrypoints remain usable.
No-op edits return unchanged success without writing. Diffs over 32 KiB remain
unavailable with an explicit reason; they are never silently truncated into valid
calibration input. Metadata is bounded to 256 KiB before edit mutation and again
at persistence. Denied/failed edits have no successful staged metadata.

`--require-read-tool` and `--require-edit-tool` require at least one successful
read and changed edit respectively. Defaults remain off. Every run outcome
separates requirements, read/edit calls, successful/failed/unresolved results and
unchanged edits. Model-run `patch_applied` is unavailable with no edit calls,
false with unresolved/failed edits or only no-ops, and true when at least one
changed edit succeeded and no edit result failed/remains unresolved. Exact output
matching remains a separate property. An earlier failed attempt can coexist with
matching final output; the report retains both facts. `--export-messages` writes
a bounded, lossless `messages.json` alongside the durable session; it preserves
raw provider fields. Partial usage is saved before observation/export errors.

```text
harvest edit-benchmark unpack --archive fixtures.tar.gz --output /path/to/new-extraction
harvest edit-benchmark validate --fixtures fixtures.tar.gz
harvest edit-benchmark run --fixtures fixtures.tar.gz --output /path/to/new-run --approve-fixture-edits --require-read-tool --require-edit-tool --export-messages
```

The ordinary `FixtureSource` SDK owner accepts an unpacked tree or tar.gz file.
Temporary extraction stays alive through loading and all run workers, then is
removed by its owner. For durable extracted paths use `unpack`. Archive listings
identify the archive and task IDs and omit temporary filesystem paths. Directory
listings retain input/expected paths. Validation issues and archived fixture source
are inert data; no archived source is executed.

Extraction uses native Rust tar/gzip libraries and the existing jailed filesystem.
Only directories and regular files are accepted. Paths reject traversal, absolute
prefixes, backslashes/ADS/device aliases, case aliases (including directory
prefixes), duplicates, excessive nesting and oversized names. The optional top
`fixtures/` directory is stripped. Every file is atomically created; no tar
ambient-unpack or shell helper is used. Output must be empty and owned by one
writer. Explicit extraction preserves partial files on failure; retry into a new
directory. gzip CRC/footer is read through EOF, and non-padding data after the
tar terminator is rejected.

Bounds: 32 MiB compressed input, 256 MiB decompressed stream, 128 MiB payload,
10,000 entries, 64 path components, 4,096-byte paths, 255-byte components and the
shared 4 MiB per-file limit. Cancellation reaches compressed/decompressed reads
and entry writes. Fixture loading validates the resulting task structure. The
new tar dependency and cached gzip/temp-directory dependencies are Rust-only.

Stabilization must cover actual distributed fixtures, PAX/long-name metadata,
truncation/CRC/trailing members, links/devices/sparse entries, duplicate and
case-alias paths, file/directory collisions, cancellation/cleanup and bounded
allocation on all promised platforms. Also cover real edits/no-ops/denials,
read-content error prefixes, metadata bounds, both session hosts and RPC replay,
missing/failed/unresolved results, requirement flags and exported raw numeric
usage. Session persistence failure after a successful filesystem mutation retains
the existing recovery limitation; journaling is still pending. The analyzer's
full-record JSON projection also needs unknown numeric-usage compatibility tests.

## Native AST mutation producer (conversion checkpoint)

```text
harvest edit-benchmark mutations
harvest edit-benchmark generate --source /path/to/reviewed-source-data --output /path/to/new-fixtures --task-id comparison-case --mutation swap-comparison --seed 42
```

`pi_ast::mutations` owns syntax-aware candidates and byte-edit application, using
the existing shared parser/language/cache. The application `generate_task` API
owns selection, snapshots, fixture publication and cancellation. There is one
native AST parser/cache, not a benchmark-specific parser. `generate` writes a
normal unpacked task that the existing native validate/list/run commands consume.
The source tree is explicit reviewed data; no repository/corpus discovery is
silently performed and no source is executed. Original input trees remain intact.

Code paths for 23 named families are written; eligibility and parity remain partial: comparison/equality/logical/arithmetic
operators, negation, increment/decrement, booleans, optional chains, argument
swaps, nullish operators, Unicode en dashes, off-by-one boundaries, adjacent and
sibling statements, if/else branches, redundant wrappers, duplicate blocks,
fall-through case labels, repeated-statement literal/operator flips and distant
block moves, regex quantifier changes, composite edits and conservative identifier
reference edits. `mutations` advertises the exact native names. JS/JSX, TypeScript and
TSX use their registered grammars. Flow recovery and Babel binding analysis are
not fully ported. Tree-sitter syntax validation does not certify name resolution or
runtime semantics, which intentionally change in mutation tasks.

The native producer is separately identified as `rust-ast-v1`. Its seed algorithm
is SplitMix64, with unbiased reservoir selection across sorted source files and
ordered candidates. It is not the legacy JavaScript LCG/Babel candidate stream.
Metadata records producer, exact seed, family, target, candidate ranges/old/new
bytes, line, candidate/rejected-source counts and SHA-256 original/mutated content
identities. Historical fixtures, mutation results and scores retain their original
identities; no historical seed/candidate equivalence is claimed.

Source snapshots are bounded to 64 MiB and shared file/traversal limits. Unsupported
file types are copied inertly; non-UTF8, oversized or parser-rejected mutation
targets remain unchanged and are counted as rejected sources. Candidate discovery
is bounded to 256 KiB source, 100,000 named nodes, depth 256, 4,096 candidates and
8 MiB candidate text per file. Global selection caps at 100,000 candidates. Parse
callbacks check cancellation before/after parsing and during traversal; a parse
itself remains synchronous inside the existing owner. Application validates old
bytes, UTF-8 ranges and non-overlap, bounds output and re-parses it. A rejected
selected mutation fails explicitly; there is no unbounded retry or fallback.

Original snapshots become expected files, mutated snapshots become input files.
Publication reuses the exact hunk producer and static Markdown prompt, including
its byte-exact re-solve. Output paths are checked for source separation before
creation; expected/input fingerprints and mutation provenance are bounded before
prompt publication. Temporary snapshots are cleaned by their owner. Partial
explicit output remains inspectable on failure, as with paired preparation.

Regex quantifiers use the existing shared Regex grammar over each literal's
pattern. Only actual `one_or_more`/`zero_or_more` nodes are changed; character
classes, escaped signs, flags and lazy suffixes remain unchanged. Patterns rejected
by that grammar and Unicode-set (`v`) literals are ineligible, rather than guessed.

Composite candidates combine 2–5 different token families against the same original
snapshot with non-overlapping ranges. They enumerate a bounded neighborhood of 64
later candidates per anchor, not the historical sequential re-mutation algorithm.
The pool/output each cap at 4,096 candidates and 8 MiB edit text; selected output
still passes shared range checks and syntax re-parse. Seed/provenance remain native.

Identifier multi-edit resolves declarations and references before selection, with
lexical block/function scopes, var hoisting, simple parameters and function names.
It prefers bindings with references on at least three lines, falling back to two;
selects 2–4 distinct reference lines; and includes the declaration when its line is
selected. Only selected references change. ASCII misspellings follow the legacy
permutation, excluding unchanged/keyword/invalid-first-character/colliding names.
The current subset supports JavaScript only: imports/exports, JSX, classes/methods,
catch/with/eval, destructuring/default/rest parameters, duplicate declarations,
for-in/of and Annex B block functions are rejected. These restrictions preserve
honest partial eligibility; full Babel binding behavior is still unfinished.
Generation reports retain up to 32 rejected-source examples with bounded reasons;
no-eligible-candidate errors name the first rejection. Selected-application failures
include the family and relative source path. Sources remain unchanged.

`pack_fixtures` and `edit-benchmark pack --fixtures /path/to/tasks --output /existing/parent/tasks.tar.gz`
connect validated native tasks to the existing archive loader/list/validate/run.
No source is executed. Sorted regular-file entries use stable timestamps/mode/IDs
and gzip headers, and the same portable path/case-alias policy as extraction.
All shared source traversal/file bounds apply, compressed output caps at 32 MiB,
and publication is jailed, writer-owned, atomic and create-only. Output must be
outside the fixture tree; its parent must already exist. Cancellation/failure
before publication leaves no partial archive. Sources must be quiescent while
exporting; this does not claim a transaction over a concurrently modified tree.
Historical archive import remains supported independently of native production.

Still pending: full binding/numeric/string escape/Flow and grammar-specific
eligibility parity, legacy difficulty/corpus/dedup/calibrated-size policies,
mutation-specific prompt guidance and full metaharness consumer cutover. Both TS `mutations.ts` and `generate.ts` remain required by the
legacy package generator. Their ledger rows are partial/unverified, not removed.

Stabilization must run representative cases of every family through generate,
validate and installed native run; check the repaired expected bytes and unchanged
surrounding source, deterministic seeds, UTF-8/comments/strings, JSX/TS ambiguity,
optional calls/computed access, grouping/shadowing, no candidates, stale/overlap
ranges, syntax rejection, statement movement and actual two-hunk shapes. Exercise
source/output aliases before creation, symlinks, budgets, cancellation and failure
cleanup. Include export/import byte identity, stable archive bytes, long names,
case aliases, existing output, compression limits and concurrent-source behavior. No behavioral or installed verification is claimed by this checkpoint.

## Bounded attempts and exact-match stopping (written, unverified)

`run --max-attempts 3 --no-edit-retries 2` uses the same model/session/workspace
for follow-ups. Defaults are one scored attempt and two no-edit retries; each
setting is capped at 16 (attempts minimum one). A no-edit retry means this turn
made no persisted edit/write call; it consumes its independent run-wide retry
budget before spending a scored attempt. Tool success is still taken from actual
results, not prose or attempted writes. All attempts share one run timeout and
existing per-run usage/events limits. Unknown/session/timeout failures end the run. Selected typed provider recovery
is described below; first-event/attempt-timeout recovery remains pending. These retries
are not the full historical adapter policy.

After all calls in a tool batch have durable results, the same session owner may
stop before another provider request if rust-v1 exact full-tree matching and
required read/edit evidence pass. Default enabled; disable with
`--no-early-stop-on-match`. No formatted comparison, synthetic assistant message,
extra usage, new writer or unfinished tool call is introduced. Final scoring is
repeated. General submit/execute clients retain their original behavior.

Artifacts add `attempt-NN.json`, `score-NN.json`, cumulative `usage-NN.json` and,
with --export-messages, `messages-NN.json`. Final usage/messages also preserve
partial failed turns; outcome.json includes attempts/no_edit_retries/early_stopped.
Final score.json remains the last completed scoring result, absent on turn failure.

`harvest edit-benchmark view --output RUN_DIRECTORY [--json]` and the SummaryView
SDK inspect native summary.json without model/network access. Read limit 2 MiB,
1..1000 ordered runs, validated version/state/totals and attempt limits; text task
identities are JSON escaped and no artifact paths are followed. Older rust-v1
summaries without attempt data show unavailable, never zero. This static native
report does not establish full interactive dashboard or metaharness parity.

Pending acceptance: scripted and installed retry/no-edit/multi-tool/early-stop
ordering/cancellation/usage/artifact contracts, old session API regression checks,
and malformed/version/index/totals/control-label summary handling. No runtime or
installed verification exists for this continuation. Legacy cli.ts still imports
runner.ts; in-process-client.ts and metaharness orchestration remain active.

## Typed pre-mutation provider retries (written, unverified)

`run --provider-retries 3` (default 3; 0..16) retries only typed HTTP 401/403,
408/425/429, 5xx and transport IO before this attempt makes any non-read/grep
call. Unknown tool calls and denied/failed mutations conservatively block retry.
Other errors/statuses are terminal. Backoff 1/2/4/8/10 seconds capped at 10 stays
inside the original run deadline and checks cancellation every 20 ms. Budgets are
separate from scored/no-edit attempts; maximum combined submits is now 48.

Auth retries invalidate existing command-backed key/header cache entries through
the shared model/value resolver; host command opt-in remains required. SDK models
may implement the invalidation seam, whose default only checks cancellation.
No OAuth/account rotation or general session retry is established. Static or
environment credentials follow their existing resolution rules.

provider-retry-NN.json records only kind/status/index/count/delay. Failure text,
URLs, bodies and secrets never become retry prompts or artifacts. Failed request
count stays separate from received model response accounting; any failed request
keeps usage unavailable after recovery. Known raw/partial totals are retained.
View/outcome data includes provider retry counts; old summaries show unavailable.

This resolves only the typed HTTP/transport part of the earlier prerequisite.
First-event/attempt timeout recovery, broad provider policy and installed failure
contracts are pending. Legacy guided patches require hashline; native tools
currently expose replacement mode. See tasks/HASHLINE-HOST-BRIDGE.md for the
bounded host/snapshot/read-provenance prerequisite rather than sending unsupported
hashline calls. All runtime/installed/platform evidence remains pending.

The shared transport now keeps request-builder/configuration and response-decoding
errors as separate redacted nonretryable categories, rather than treating them as
IO. Cancellation and malformed message errors keep their existing failure paths.

## Cooperative per-attempt deadlines (written, unverified)

`run --attempt-timeout-seconds 30 --timeout-retries 3` enables a child deadline
for each submit. Default has no child deadline; the original overall run timeout
still applies. Child timeout must be positive and no greater than the overall
limit. A fresh child never extends the original parent deadline. Timeout retry
budget is 0..16 (default 3), independent of all other budgets; combined maximum
submits is now 64. This is cooperative cancellation, not forced SDK interruption
or the retained first-provider-event/connection timeout policy.

Only a typed runtime AbortError with Timeout cause qualifies, with parent still
live, no mutation calls and no unresolved tool call results. The shared runtime
keeps its original Aborted diagnostics, now typed for safe cause inspection.
Signal/user/unknown aborts and textual timeout lookalikes are not retried. Both
provider and timeout recovery now require complete results for every tool family,
including grep; no follow-up enters a session with an unfinished call batch.
Unsafe/exhausted attempt timeouts end as timed_out with attempt_timeout reason.

Numbered attempt artifacts include elapsed submit time, delivered session event/
delta counts, last event, committed message count and typed timeout flag. Lifecycle
Started/Failed events do not imply remote provider activity. timeout-retry-NN.json
preserves redacted recovery telemetry. Outcome/view timeout_retries is unavailable
for older summaries. Usage remains unavailable after failed provider requests.
Runtime/installed/platform and cancellation compatibility evidence remain pending.

## Native corpus policies (written, unverified)

```text
harvest edit-benchmark corpus --source /path/to/local-data --policy legacy-packages
harvest edit-benchmark generate --source /path/to/local-data --source-policy legacy-packages --output /path/to/new-fixture --task-id case-001 --mutation <NAME_FROM_MUTATIONS> --seed 42
```

Use `mutations` to list actual supported mutation names. `inspect_corpus` and
`generate_task_with_policy` expose the same Rust policy. Existing generate_task
and CLI generate default to reviewed-tree, preserving curated-tree snapshots.
Native source policy is independently versioned rust-corpus-v1; hashes/counts/
policy enter the report/provenance, never legacy results.

legacy-packages inspects packages/ beneath the supplied local root; it prunes
__tests__/__mocks__/fixtures/fixture/dist/build/node_modules/scripts. It admits
.js/.jsx/.ts/.tsx names without .test./.spec./.fixture., then nonempty UTF-8 JS/TS
sources <=256 KiB with 30..800 LF-split lines and a literal function/class/export
space or => structure token. Generation snapshots only admitted source files in
that mode, preserving packages-relative paths. reviewed-tree keeps all source
files as context and existing native candidate rejection behavior. Missing root,
no selected source/candidate, ambiguous hunks or unsupported syntax fails explicitly.

The corpus report lists coarse per-file eligibility/reason, SHA-256, byte and
line counts, not source content. Pruned paths are outside the report. No AST
parse or named-mutation success is certified by eligibility. CLI returns 1 if
none are coarsely eligible. Selection and inspection share bounded/cancellable
jailed traversal (64 depth/10,000 entries/64 MiB); corpus path limits are 4096
UTF-8 bytes each and 1 MiB combined. The generator reads sources once per snapshot,
uses the existing engine, and rejects output overlapping the original data tree.
No repository is cloned and no fixture source executed by these commands.

Difficulty/range/repetition scoring, dedup/calibration/multi-case quotas, repository
acquisition and specialized guidance remain pending, as do all runtime/installed
checks and actual legacy producer/metaharness consumer moves. No TS deletion.

## Difficulty preferences and source-bound regions (written, unverified)

`corpus --source DIR --policy legacy-packages --difficulty hard` adds bounded
lexical metrics and preferred-file flags. `generate --difficulty hard` uses these
preferences and actual native AST candidates; optional --min-score overrides the
level's 0/2/5/8 minimum (easy/medium/hard/nightmare). Without --difficulty, no
metrics/score filter runs and the existing native seed draws stay unchanged.
Corpus coarse eligibility/exit status does not certify preference or a mutation.

rust-difficulty-v1 ports repeated lines, regex blocks/function ranges, density,
indentation, middle-of-file/trivial context and similar names, with JS whitespace
and UTF-16 length semantics. They remain lexical heuristics: raw braces/comments
and first duplicate function name can affect ranges. They do not replace pi-ast
binding or parsing. Limits: 256 KiB/4000 lines, 4096 unique identifiers/function,
one million name comparisons/file; cached and cancellable, explicit failure on
budget overflow. Native reservoirs prefer passing candidates in matching files,
falling back to other passing files only if none preferred pass. Counts, selected
score and fallback are recorded. This is independently versioned native sampling;
legacy random attempts/distributions and historical benchmark scores are distinct.

`generate --exclude-regions FILE.json` reads a JSON array of previous native
regions: path, original_sha256, start_line, end_line. SDK GenerationOptions exposes
the same exclusions. Output regions cover every edit in the chosen candidate,
including composites, with source SHA-256. Matching source/path spans plus three
lines are excluded before sampling; changed hashes never suppress new source.
Insertion and end-exclusive byte ranges map to LF-based original line spans.
Policy rust-regions-v1, counts and regions enter metadata/report. Input <=1 MiB,
4096 records; bounded relative UTF-8 paths/hashes/line ranges. The input is read
only and never used to open named files. It is not a mutable/resumable history
store, whole-content deduplication or multi-case quota scheduler.

Ordinary Rust APIs include difficulty_metrics, inspect_corpus_with_difficulty,
GenerationOptions and generate_task_with_options; prior APIs retain defaults.
Full calibration/shape quotas/multi-case orchestration, specialized guidance,
full syntax eligibility and legacy producer/metaharness cutover remain pending.
All runtime/installed/platform behavior and old-consumer regression evidence
remain deferred. No TS source is deleted by this continuation.

## Explicit multi-case collection generation (written, unverified)

`generate-batch --source DIR --output NEW_DIR --plan cases.json` accepts an ordered
JSON array, for example:

```json
[
  {"task_id":"case-001","mutation":"swap-comparison","seed":42},
  {"task_id":"case-002","mutation":"flip-boolean","seed":43}
]
```

Use --source-policy, --difficulty, --min-score and optional --exclude-regions for
the whole plan, or Rust generate_batch with GenerationOptions. Plans have 1..64
unique portable task IDs, reject case aliases/unknown kinds, and execute serially
against one immutable selected corpus snapshot. Hash rechecks reject changed data
while snapshotting. Later changes to original source do not affect that batch.
Successful regions are carried into later cases through the existing exclusion
policy; a case with no eligible candidate fails rather than changing its seed or
level silently. This is explicit case scheduling, not calibrated shape quotas.

TaskCollection retains the existing fixture resource lock for the full invocation.
Each case borrows the same publisher, with expected/input copy and prompt-last
publication; ordinary single-case empty-directory rules remain. Preflight copy/
metadata estimate <=48 MiB, conservative <=10,000 entries and archive nesting
limits; actual output bytes across tasks/artifacts <=64 MiB, root JSON <=4 MiB.
CLI plan/history inputs <=1 MiB. All existing source/AST/history bounds apply.

Task directories stay compatible with list/validate/pack and archive/model runs.
Batch reports use fixture.output paths relative to the collection root; standalone
prepare/generate reports keep their existing path behavior. Original source roots
and temporary snapshot paths are not serialized into successful batch journals.
Root artifacts generation-plan.json, generation-case-NN.json and generation.json
use rust-batch-v1 and retain inventory hashes, options, per-case reports and
complete/partial counts. Stop at a failed/cancelled case; remaining cases are
not_started. CLI exits 1 for an incomplete plan. Setup or artifact/quota failure
may return an invocation error; preserve prior artifacts and use a new directory.
Interrupted task directories are not verified or automatically removed, and pack
may reject them. No resume, concurrent writer, automatic source acquisition,
source execution, archive publication or historical generator equivalence.

Runtime/installed/platform and single-case regression evidence remains pending.
Shape calibration/quotas, whole-content deduplication, full retained grammar and
specialized prompt/guidance/metaharness consumer cutover remain unfinished.
