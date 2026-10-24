# Metaharness narrative trace reporting

ID `application:metaharness-trace-report`; reviewed, in progress. Prerequisites:
`control:playbook`, `review:application`, `review:browser`,
`review:catalog-providers`. Base dd34b68. Sole writer is the active migration
agent; destinations below are reserved to this feature. Retained capability
`surface:benchmarks`; source packages/metaharness/scripts/trace-report.ts.

## Whole feature contract and ownership

Inputs: the complete legacy standalone reporter; metaharness server #trace
normalization/GET routes and store RunRow/TraceRow; shared native provider,
configuration/value, cancellation/HTTP transport, protocol session/display and
jailed output owners. Output remains narrative Markdown: numbered assistant
turns and harness notices, deterministic tool counts/error labels, per-turn model
sentences, then Story Arc and failed-run analysis. Prose never proves execution.
Do not run transcript tools, code, plugins or hooks. No application session writer
or mutation approval path is involved except explicit report-file publication.

Destinations: rust-agent/src/trace_report/{mod,render,cli}.rs; static prompts under
src/prompts/trace-report (included by the existing Bazel compile_data glob).
CLI harvest/omp trace-report and ordinary Rust decode/group/generate/fetch/
from_session/Trace/Run/Options/Report APIs share this single implementation.
Packages/metaharness trace:report and primary README instructions now call Rust.
The explicit trace:report:legacy-auth alias points at the retained standalone TS
script for existing stored/OAuth users. No native command silently invokes it.

The pipeline groups results onto the most recent assistant, even across a notice;
an orphan result is an error. Up to 32 scoped workers (default 8) summarize turns,
then join before a single synthesis request. Preserve source order regardless of
completion order and tool occurrence order for counts. Stop scheduling new work
on worker failure; join active requests under the same caller deadline. Reject
model tool calls. Four text attempts with 1/2/3-second cooperative waits; failed
request usage is unavailable. Shared runtime cancel::wait replaces the identical
benchmark-local helper; do not fork another timer loop. No deadline reset, detached
watchdog, engine replacement or cross-session state is introduced. SDK models
must cooperate with cancellation; arbitrary model code is not forcibly interrupted.

Shared HttpModel gains an explicit output-limit option, absent for existing
callers; map requests use 300 tokens, synthesis 3000. Both retained native wire
APIs use that existing provider owner. Credential/header commands stay explicit
host opt-in. Select native profiles, not a new hardcoded provider/model catalog.
Unknown usage/pricing remains unavailable; cache reads/writes remain separate.
Native output is versioned rust-trace-report-v1 and separates map/reduce usage
(including when the same model ID is selected), not historical cost equivalence.
Temperature/catalog generation policy and stored/OAuth parity remain pending;
never infer matching model output from compilation or a copied prompt.

## Input, output and limits

HTTP uses the existing bounded/cancellable transport, encodes run/trace as path
segments and rejects credentials/query/fragment in the base URL and dot/slash
identities. Request the server's maximum tail=2000. The server may already truncate
trace/result bodies; always disclose normalized entry/event counts and excerpt
limits. Unsupported question/answer/reference/conversation variants fail explicitly,
not a silent empty report. Optional unavailable/invalid remote run metadata yields
an explicit trace-only diagnostic; offline supplied malformed metadata is an error.
Metadata identity, duplicate trace rows and finite nonnegative cost/duration checks
happen before model work. Failure metadata must reach the failure-analysis prompt.

Offline --input reads normalized JSON, optional --metadata supplies run JSON.
--session/--leaf instead uses the shared SessionDocument/validate_tree/project_branch
owners. Reject malformed records pending explicit recovery. Only assistant prose,
tool names/results and compaction/branch boundary notices enter the narrative view;
thinking, signatures, provider metadata and tool arguments remain out. Original
bytes/tree are unchanged. This is a lossy analysis projection, not provider replay.

Bounds: 8 MiB normalized input/aggregate text, 1 MiB per entry, 4096 entries,
1024 tool labels per assistant, 4096 bytes/control-free label, 16 KiB focus,
2 MiB synthesis prompt, 4 MiB rendered report, 4096-byte map model response,
64 KiB synthesis response. Inputs/prompts/output use bounded shared readers/writers.
SDK session byte limit stays the shared 64 MiB limit; selected branch <=4096.
CLI whole deadline default 600 seconds/max 3600, lanes 1..32. Map excerpts are
2000 prose/1400 result UTF-16 units; native Unicode-safe truncation never emits a
split surrogate. Final prose cap 4000 units. Large/policy-invalid work fails.

--workspace jails input/output/overlays; --out prevalidates target and holds shared
resource ownership before network/model work, rejects overlap with explicit source/
metadata/session/overlay files, and atomically publishes only a full successful
report. On errors/deadlines, previous output stays intact. stdout is one completed
Markdown/JSON document, no provisional deltas or raw provider diagnostics. Markdown
escapes tool/model/metadata labels and strips terminal controls; generated narrative
remains untrusted Markdown data. No graphical dashboard or interactive TUI parity.

## Conversion, deletion and stabilization

Sequence written: normalized types/validation/grouping; shared prompt assets;
bounded model/usage pipeline; HTTP/offline/session inputs; native CLI/SDK/package/
README connections; precise compatibility prerequisite and ledger update.
Tests written: rust-agent/tests/trace_report.rs (six contract scenarios). The active
conversion-first phase runs aggregate compiler/format/ownership checks only.
Do not claim these tests have executed from cargo check --all-targets.

Conversion commands:
- cargo fmt -p harvest-omp -p harvest-runtime -- --check
- cargo check --locked -p harvest-omp -p harvest-runtime --all-targets --all-features
- cargo xtask playbook check
- cargo xtask source-audit verify --scope affected --base dd34b68 --plan

Consolidated stabilization: cargo xtask test -p harvest-omp --test trace_report;
cargo xtask test -p harvest-runtime; shared provider/benchmark callers plus strict
Clippy; scripted map/reduce out-of-order/failure/empty/oversized/tool-call/usage
scenarios, same-model split usage, typed cancellation and retry deadline, real local
HTTP source/model fixtures including encoded IDs/metadata fallback/tail/unsupported
formats, profile/env/command sources, actual copied installed CLI stdout/JSON/
atomic-file/locking/path-alias/symlink/overlap cases and native session selected
branch/private-data/error/recovery. Test output-limit requests for both wire APIs
and existing caller request compatibility. Credential-dependent live checks remain
blocked until available, fixture checks cannot stand in for them. Finally exercise
promised platform artifacts and source/lifecycle validators.

No TS source removed: application:legacy-provider-credentials must supply the
shared stored/OAuth/discovery owner, complete exact credential/catalog/provider
checks, move the last compatibility consumer, then delete trace-report.ts and
its explicit legacy alias. Do not delete shared auth/provider/server sources.
Update the ledger/Markdown and required source-bound evidence before doing so.
Recovery: revert this coherent checkpoint to dd34b68; legacy standalone reporter
and unchanged stored credentials remain available. Completion requires retained
behavior/installed/removal gates; no done or verified claim in this conversion.
