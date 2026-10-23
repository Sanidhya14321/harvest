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

Pending: language-specific mutation selection, retries/guided mutations,
connection/early-stop policies, formatted conversation exports, interactive
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
