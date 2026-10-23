# rust-v1 edit benchmark conversion checkpoint

Native task loading, patch application, scoring and model-run orchestration are
written and connected; behavioral and installed verification are pending. The legacy metaharness runner
still uses its historical scoring. No existing score is converted or relabeled.

`harvest edit-benchmark list --fixtures DIRECTORY` loads unpacked tasks with
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
of provider usage. Model-run scores retain `patch_applied: null`; ordered manifest
application remains the source of explicit patch-application outcomes. Sessions
retain actual tool results. No attempt is made to infer patch success from a
model's prose. Unknown/missing/overflowing usage and failed turns remain unavailable;
partial raw response usage is retained. Cache accounting distinctions are retained
in measured token totals. Prices remain unavailable. A completed output mismatch
is unsuccessful, distinct from cancellation, timeout and failed execution. CLI
returns nonzero if any run is unsuccessful. No expected fixture is executed.

Pending: Rust mutation/fixture generation, archive materialization, edit-shape
analysis, retries/guided mutations, connection/early-stop policies, tool-use
requirements, conversation exports, interactive benchmark views and metaharness
consumer cutover. Global artifact/disk quotas, crash resume and custom-model
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
return nonzero. No NaN percentages are emitted for empty buckets. Successful
native edits currently lack persisted diff details; they count as missing diffs,
not measured edits. Adding structured native edit-result metadata is still pending.

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
mutation selector. Native mutation selection, archive handling and guided/retry/
metaharness workflows remain pending. Stabilization must exercise real legacy
fixtures, parser failure/coverage, grouping, empty cases, source immutability,
ambiguous/overlapping hunks, exact final-newline behavior, cancellation and copied
installed commands before source removal is marked verified.
