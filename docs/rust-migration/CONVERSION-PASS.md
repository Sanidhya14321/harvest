# Conversion-first pass: shared ownership and feature register

Active task: **`application:conversion-first`**, state **in_progress**.
Initial base: `64f3dd7`; current process-byte continuation base: `34e1876`. Current owner: the active migration agent on `ts-to-rust`.
The latest user instruction changes scheduling: convert connected code first,
then resolve errors and test together. `PLAYBOOK.md` records this across sessions.
Do not revert to per-file test/addon cycles on resume.

## What is written versus what is done

“Written and connected” describes code and consumers. It does not mean behavioral
parity, installation verification, completed migration or eligible source deletion.
The machine manifest retains these distinctions. No feature becomes done from a
compiler result. This checkpoint has no behavioral or installed evidence.

## Reserved interfaces and file ownership

One owner per row. Do not introduce another registry, configuration resolver,
plugin process owner or benchmark scorer beside these paths. Coordinate changes
through this table and the machine task before editing a shared interface.

| Feature | Sole Rust destination | Consumers connected | Remaining work | State |
|---|---|---|---|---|
| Tool schemas/registration | `rust-agent/src/chat.rs`, `tools.rs` | OpenAI/Anthropic model requests; sync/async session tool runtime | schema/collision regressions and dynamic registry integration tests | Written/connected; unverified |
| Native plugin v1 host | `rust-agent/src/plugins/{protocol,host}.rs` | `ToolRuntime`, `AgentSession`, `SessionEngineOptions`, run/RPC `--plugin` | cancellation/exit/queue/approval tests; graceful draining and descendant cleanup | Written/connected; unverified |
| Rust plugin SDK | `rust-agent/src/plugins/sdk.rs`, `examples/native-plugin.rs` | protocol authoring, host callbacks, cancellation, typed UI events | copied external consumer; version/malformed/process-exit tests; installed platforms | Written/connected; unverified |
| Plugin UI | same protocol Event enum; SDK callback | direct SDK, sync/async session, CLI --events and RPC v1/v2 event streams | native TUI rendering, durable UI/reconnect state and interactive choice responses | Partial; unfinished |
| Native settings/profiles | `rust-agent/src/configuration.rs`, `configuration/{document,file}.rs`, `config.rs` | sync/async run, RPC, SDK, redacted `harvest config`; TOML/JSONC/YAML overlays | legacy settings schema/discovery/external-source parity, settings persistence/UI, credential ownership | Written/connected foundation; unfinished parity |
| Typed config file owner | `rust-agent/src/configuration/file.rs` | layered native loader, persistent SDK owner, explicit CLI config migrate | full settings/default/discovery and implicit startup migration; cache/validation/migration/installed behavior | Written/connected; unverified |
| Bounded serialized output | `crates/harvest-runtime/src/limits.rs` | config migration, existing process JSONL encoding and fixture archive export | byte-boundary/error/newline/round-trip regressions | Written/connected; unverified |
| Live provider values | `rust-agent/src/configuration/values.rs`, `values/command.rs`; shared provider builder | native config -> OpenAI-compatible/Messages -> sync/async/RPC/SDK/benchmark; existing pi-shell owner | legacy model registry/overrides/live-header mutation, auth retry/OAuth/accounts and installed behavioral contracts | Written/connected foundation; unverified |
| Rust edit scoring | `rust-agent/src/edit_benchmark` | native CLI list/compare/apply and ordinary Rust APIs | normalization/patch/file-set bounds tests; actual model runs and usage integration | Written/connected; unverified |
| Shared process owner | `rust-agent/src/process_transport.rs` | native plugins, MCP stdio, LSP and DAP | platform descendant/console cleanup, cancellation/failure/queue regressions | Written/connected; unverified |
| MCP client and tool bridge | `rust-agent/src/mcp`, shared HTTP/SSE owner | native sync/async sessions, RPC, direct SDK, `harvest mcp`, `--enable-mcp`/`--mcp-config` | OAuth/discovery/legacy SSE/reconnect, idle event delivery, hot reload and installed/platform parity | Written/connected; unverified |
| LSP client and text edits | `rust-agent/src/language_services/{lsp,edits,runtime}.rs` | native tool registry, sync/async sessions, RPC, SDK, CLI | resource operations, full diagnostics/sync/discovery/linters/editorconfig, recovery/performance/platform tests | Written/connected; unverified |
| Debug adapter and session state | `rust-agent/src/language_services/{dap,runtime}.rs` | same host owner/config/tool/session/CLI boundaries | socket/discovery/reverse terminal/advanced breakpoint workflows, cleanup and behavioral/platform tests | Written/connected; unverified |
| Benchmark model orchestration | `rust-agent/src/edit_benchmark/runner.rs` | `run_tasks` SDK and native `edit-benchmark run`; shared model/session/config/approval | behavioral/installed checks, retries/guided/early-stop policies, formatted exports, native views, metaharness cutover | Written/connected; unverified |
| Benchmark fixture preparation | `rust-agent/src/edit_benchmark/{hunks,prepare,tasks}.rs` | native prepare/validate CLI and SDK; shared pi-diff/jail | full mutation eligibility, corpus/difficulty policies and behavioral/installed verification; legacy generator still active | Written/connected foundation; unfinished |
| Edit-shape calibration | `rust-agent/src/edit_benchmark/shapes.rs` | native CLI/SDK and package edit-shapes command; legacy standalone TS script removed | parser/grouping/coverage/installed regressions; native bounded/unchanged metadata cases | Written/connected; removal verification pending |
| Native tool-result metadata | `rust-agent/src/{edit,tools,durable_session,agent_session,session_engine}.rs` | shared native SDK/sync/async/RPC committed records and benchmark analyzer | actual result/error/no-op/metadata bounds and persistence/replay regressions | Written/connected; unverified |
| Fixture archives | `rust-agent/src/edit_benchmark/archive.rs` | SDK FixtureSource/pack_fixtures, CLI pack/unpack/list/validate/run and shared task loader | malformed/CRC/path/alias/budget/cancellation/platform and installed contracts | Written/connected; unverified |
| Syntax-aware mutation generation | `crates/pi-ast/src/mutations.rs`; `rust-agent/src/edit_benchmark/generate.rs` | native generate/mutations CLI and SDK -> shared snapshots/hunks -> validate/run | full binding/grammar eligibility, corpus/difficulty/calibrated shapes, legacy generator cutover, behavioral/installed checks | Written/connected foundation; unverified |

## Inputs, connections and deletion rules

Inputs: existing plugin/extension interfaces under
`packages/coding-agent/src/extensibility`, config/config-file/settings sources,
`packages/typescript-edit-benchmark/src/{tasks,verify,in-process-client}.ts`,
metaharness edit adapter, MCP client/types/transports/config/tool bridge under
`packages/coding-agent/src/mcp`, LSP/DAP clients/config/types/edits and shared
`jsonrpc/message-framing.ts`, tools/debug.ts and lsp/tool.ts, current native
tools/provider/session engines.
The source ledger continues owning every retained legacy path through its parent
migration task. This leaf reserves native destination interfaces; it does not
certify the unported parent or move a source row to verified removal.

Implementation order: owned tool schemas; executable protocol/host/SDK; connect
session ownership and host approval; shared configuration/profile selection;
connect native launchers; versioned benchmark loader/apply/scorer/CLI; shared
process owner and MCP transports/client/config/tool bridge/CLI; Content-Length
framing, LSP/DAP clients/state, UTF-16 text edits and native host connections. All are
written before the consolidated stabilization phase. Reuse existing jailed file,
edit, transport, provider and session engines. No new dependency or N-API adapter.

The initial plugin/config/scoring checkpoint (`2c2078c`) deleted no TS: the production application, plugin
loader, settings implementation and metaharness still consume them. Default
launchers remain on the current product until retained behavior is connected and
installed verification passes. Legacy language plugin loading is retired by the
native ABI; the old TS application's retirement diagnostic/cutover is unfinished.

## Language/deletion audit and current benchmark continuation

[LANGUAGE-PROGRESS.md](LANGUAGE-PROGRESS.md) binds counts to `bee9356` and records
GitHub's default branch `main`, live language-endpoint access limitations, 60
TS/TSX deletions since baseline and zero in the previous four commits. Do not
claim Rust additions reduced TS. The model-run checkpoint (`b49ba10`) also removed zero TS
files: the metaharness adapter/package consumers remain active.

Native orchestration now owns sorted task selection, independent input copies,
bounded concurrent repetitions, one session/model per run, externally owned
session/event artifacts, final-argument fixture mutation approval, cancellation,
per-run outcomes and raw/aggregate usage. CLI and SDK call the same owner.
`AgentSession::open_with_storage` reuses the durable writer while keeping the
scored workspace separate; existing `open` callers preserve their storage layout.
The actual legacy inputs/consumers remain `packages/metaharness/adapters/edit/runner.ts`,
its CLI and `packages/typescript-edit-benchmark/src/in-process-client.ts`.
See `RUST-BENCHMARK.md` for every retained gap, limits and exact native command.

Verification during stabilization: run a scripted SDK model through multiple
isolated runs, check deterministic ordering and input/expected immutability,
shared final-argument jail/approval, mixed missing/valid/overflow usage, denied
shell/service calls, overlapping output/writer rejection, failure/cancellation/
timeout/queue cleanup and copied installed CLI model requests. Verify interrupted
artifacts and session cwd/replay. Then cover actual retained metaharness workflows
before moving/deleting their consumers. Recovery: revert this checkpoint; the
production TS runner remains active and historical scores/data are untouched.

## Current fixture tooling batch

Base `b49ba10`. Native shape analysis, fixture validation and paired-tree task
preparation/hunk solving are connected as documented in `RUST-BENCHMARK.md`.
The only owned launcher of `edit-shape-stats.ts` now calls Rust, so that standalone
script is deleted (355 lines, 13,617 bytes). The source ledger records actual
removal and **unverified partial replacement**. Hunk/task TS sources still serve
the legacy generator/metaharness; their rows now have explicit native destinations
and unfinished contracts under this task. Do not mark the analyzer verified or
claim the parent benchmark capability complete from the deletion.

Inputs: original `b49ba10:packages/typescript-edit-benchmark/src/edit-shape-stats.ts`,
retained `hunks.ts`, `tasks.ts`, real historical session records and fixture tree.
Connections: package edit-shapes command -> native CLI/SDK shape owner;
prepare -> shared pi-diff/jailed copy/static Markdown template -> ordinary native
load/validate/run; no runtime loading of TS. Public SDK data and failure contracts
are described beside each Rust API. Read-only discovery now requires explicit
sessions-dir for ambiguous layouts instead of migrating application storage.

Acceptance/recovery: consolidate numbered/unified/malformed/bounded parser,
ID/request/file grouping, calibration distributions and raw session immutability
with exact hunk reconstruction, uniqueness/anchors/overlap/fences/final-newline,
fixture metadata and cancellation/writer/output tests. Exercise copied installed
native CLI and package launcher. Only then advance analyzer replacement/removal
verification. Run `cargo xtask migration` and indexed affected source audit without
weakening their evidence gates. Restore both analyzer and package launcher from
`b49ba10` if its replacement fails; retain historical logs/fixtures unchanged.
Native diff metadata for new session edit results, mutation selection, archive
materialization and full metaharness cutover remain named prerequisites here.

The aggregate compiler connection check passes for this fixture-tooling tree:
`cargo check --locked -p harvest-omp --all-targets --all-features`. Formatting and
playbook metadata checks pass. Logs/receipt use the ignored
`target/migration/conversion-benchmark-preparation-*` prefix. A missing internal
pending-run counter was corrected after its compile diagnostic; the failed log is
retained. No behavioral, installed or platform verification is claimed. The
source-removal gate must remain failed until deferred evidence exists. An affected
source-audit invocation began its registered broad contract build and was stopped
to preserve conversion-first scheduling; it is interrupted/unverified, not passed.
Use `source-audit verify --scope affected --base b49ba10 --plan` for the reproducible
stabilization plan. The migration gate still fails at the first inherited missing
replacement receipt (`crates/pi-natives/tools/bench-natives.ts`).

## Current benchmark integration batch

Base `1e12aa2`. Continue under the autonomy rule in PLAYBOOK.md: no per-batch
`continue` request is needed. Native result metadata and archive input now connect
the previously named prerequisite gaps. `RUST-BENCHMARK.md` defines exact inputs,
public interfaces, limits, failure behavior and outstanding stabilization.

Connections: pi-edit staged diff -> typed ToolOutput -> shared durable writer ->
sync and async/RPC committed records -> shape analyzer and benchmark ToolEvidence.
Explicit invocation error status prevents successful read text starting `error:`
from becoming a failed result. `AgentSession::stored_messages` exposes immutable
lossless records. Run requirements and optional messages.json export use that same
owner; raw usage is saved before observation/export failure. No separate tool
registry, provider replay codec or persistence writer is introduced.

Archive connections: ordinary FixtureSource/unpack_fixtures -> native Rust tar/gzip
and jailed atomic creates -> shared task loader -> list/validate/run workers.
Automatic temporary ownership spans all consumers. Directory listing paths stay
stable; archive listings expose source/task identifiers without dead temporary
paths. Explicit output extraction retains a partial directory on failure. No
archive source is executed. Native libraries replace external shell/runtime glue.

Deletion: zero TS sources are removed in this integration batch. The analyzer
removed in `1e12aa2` stays pending verification; native mutation selection and
metaharness consumers still own the remaining benchmark TS sources. No feature or
source verification counter advances. Recover by reverting this batch to
`1e12aa2`; retained TS generator/metaharness behavior and original fixture archives
remain available. New public SDK configuration fields are a conversion checkpoint.

Stabilization: actual edits/no-ops/denials/read-prefixes, bounded metadata,
lossless sync/async/RPC replay, failed/unresolved tool observations, requirements,
exports/partial usage; distributed tar.gz fixtures, long/PAX headers, CRC/trailing
members, link/device/sparse rejection, path/case aliases, payload/allocation budgets,
cancellation/cleanup and installed eight-target behavior. Native shape projection
of unknown numeric usage and persistence recovery still need explicit coverage.
Run the consolidated contracts through `cargo xtask test -p harvest-omp`, followed
by installed checks and the unchanged migration/source-removal gates. All remain
deferred. Compiler connection and metadata logs/receipt use ignored
`target/migration/conversion-benchmark-integrations-*`; no addon rebuild is needed.

## Current mutation-producer continuation

Owner: `application:conversion-first`. Base `e29f5cd`. The sole syntax/cache owner
is `pi-ast`; its `mutations/{extended,bindings}.rs` extend the existing public
candidate/apply APIs. Native CLI/SDK generation still uses the same seeded
selection, snapshots, exact hunks and fixture publisher. The registry now has code
paths for 23 named families, with partial supported eligibility and no behavioral
certification. Do not equate family-name coverage with completed migration.

Inputs: legacy `src/mutations.ts` and `generate.ts`; existing pi-ast JavaScript,
TypeScript/TSX and Regex grammars; shared jailed filesystem and task/archive owners.
Regex changes use parsed quantifiers, not substitutions through classes/escapes.
Composites use 2–5 distinct token families and checked non-overlapping ranges in
one original snapshot. Identifier edits use a conservative JavaScript lexical
resolver; unsupported forms fail closed. RUST-BENCHMARK.md specifies the supported
forms, exact limits and intentional native producer policy differences.

Connections: CLI mutations/generate -> ordinary Rust candidates/apply -> existing
hunk/task publisher -> validate/run; CLI pack -> pack_fixtures -> portable native
tar.gz -> existing FixtureSource/unpack/list/validate/run. Generation includes
bounded rejected-source examples and contextual selected-mutation errors. The
archive exporter shares import path rules, stable entry metadata, size bounds,
writer lease and jailed create-only publication. No additional parser or process
runtime is introduced.

Pending prerequisites: expand reviewed binding/grammar eligibility; port original
corpus filters, difficulty/dedup/calibrated-size policies and specialized prompts;
then replace actual generator/metaharness consumers. Source files stay retained;
ledger rows remain partial/unverified. No capability or verified removal advances.

Recovery: revert this continuation to e29f5cd; retained generator/data remain.
Stabilization: run every advertised family through native generate, exact hunk
validation and installed run; add binding/shadow/hoist/unsupported forms, regex
classes/escapes/lazy/flags, composites across families and separated hunks, stable
seed/provenance, stale/overlap ranges, bounded rejection reasons, cancellation and
source immutability. Cover pack/unpack byte identity, archive reproducibility,
long paths/case aliases/compression bounds, existing output and failed publication.
Use `cargo xtask test -p pi-ast -p harvest-omp` in the consolidated stabilization
phase. Current-host compiler/format/ownership receipts are ignored under
`target/migration/conversion-mutation-extensions-*`; behavioral/installed/platform
checks stay deferred. Export expects quiescent source data, not concurrent editing.

## Current configuration-file continuation

Owner: `application:conversion-first`; base `5341753`. Inputs/contracts reviewed:
`config/config-file.ts` constructors, YAML fallback, schema/default/auxiliary
validation, success/error/missing caching, invalidation/mtime, relocation and JSON
migration; native configuration resolver and existing format decoder. Sole owner:
`configuration/file.rs`, using existing `document.rs` and jailed runtime. Do not
build another resolver/cache/serializer beside these owners.

Written/connections: shared public `ConfigFile<T>` API -> native layered reads ->
config inspection/run/RPC/benchmark/SDK. Explicit `config migrate` -> same owner ->
validated JSON snapshot -> bounded YAML serialization and round trip -> source
fingerprint recheck -> jailed atomic create. Native reads remain read-only; retain
legacy implicit startup migration until its real discovery/schema callers move.
`BoundedWriter` in shared runtime replaces duplicate process-frame/archive buffers
and also bounds migration output. See NATIVE-CONFIGURATION.md for exact cache,
default/fallback, relocation, error, locking/cancellation and publication contracts.

No TS sources are deleted. ConfigFile's source row stays partial/unverified and
retained. Remaining prerequisites: full settings schema/defaults and discovery,
legacy implicit/async startup, persistent mutation conflict detection, symlink/
quarantine handling, failed-mutation retry, UI and actual legacy consumer cutover.
The current Settings source records file generations, pending paths/base values,
quarantine targets and retry sets; do not replace that with a blind full-file save.
Rust migration's source recheck is not a transaction against uncooperative writers.

Recovery: revert this continuation to 5341753. Existing native TOML and full TS
settings remain. Consolidated acceptance: cached success/missing/errors and
explicit invalidation/relocation/validation/defaults/mtime; primary/fallback and
invalid-primary behavior; same-stem migration with either YAML spelling, existing
outputs, missing/invalid/schema-failed inputs, unknown fields/byte preservation,
source races/native writer conflict, bounds/cancellation/interrupted publication;
SDK/native CLI/run/RPC consumers; existing frame/newline/archive contracts. Use
`cargo xtask test -p harvest-runtime -p harvest-omp` during stabilization. Current
compiler/format/ownership evidence uses ignored
`target/migration/conversion-config-owner-*`; no behavioral/installed/platform or
complete-source/capability claim is made.

## Current native completion retirement

Owner: `application:conversion-first`; base `c4aef9a`. Approved contract:
`SCOPE.md`/`compat:shell-completion-scripts` retire owned executable shell completion
and bootstrap artifacts; native in-application completion remains retained. Inputs:
`rust-agent/src/{cli,shell}.rs`, CLI tests, Cargo dependency, both Rust-candidate
installer scripts and their test harnesses. Connections reviewed: both installers
invoked native completion installation after moving binaries and would abort on
its retirement; those direct calls are removed while checksum/alias/PATH handling
is retained. All four retained script rows transfer to this task as pending
retirements, with their remaining executable runtime/test ownership explicit.

Written: hidden native `completions` rejection accepts legacy arguments and emits
no script/installation; removes generator/helpers and `clap_complete`; deletes
shell.rs and its retired implementation-specific tests. CLI and installer tests
now expect rejection/no generation. Native archives retain both real command
aliases. Manual/help remain. Existing installed user files are untouched. Retained
command completion is honestly absent, not partially implemented by a deprecated
shell generator. See distribution.md for exact boundaries and historical evidence.

The JavaScript-eval dependency review confirms its worker/runtime is still shared
with browser/computer and the eval tool/code-mode bridge; it cannot yet be removed.
Preserve that existing blocker and port its named native consumers first. This
review does not retire or certify those unrelated workflows.

Recovery: revert this continuation to c4aef9a. Stabilization: native legacy argument
variants return nonzero/empty stdout/no filesystem writes; native help/version/man
and packaged aliases work; both installers succeed without running downloaded
completion code and retain checksum/architecture/PATH behavior. Updated tests are
not executed in conversion-first phase. Use `cargo xtask test -p harvest-omp`, then
owned installer/platform workflows during consolidation. Retire remaining TS shell
generator and shell/PowerShell installer/test sources only after native replacements
are connected and their required evidence exists. Logs/receipt:
`target/migration/conversion-shell-retirement-*`. No verified source-removal,
capability or completed retirement claim is advanced.

## Current provider-value continuation

Owner: `application:conversion-first`; base `cb4216d`. Reviewed inputs/contracts:
`config/model-config-values.ts`, exact env helper, model-registry resolver/header
consumers, native provider/Messages request code, shared configuration/CLI/benchmark
builders and pi-shell raw streams/cancellation. Sole destination:
`configuration/values.rs` and `values/command.rs`, not another process/provider
registry. NATIVE-PROVIDER-VALUES.md specifies the full bounded supported contract.

Written/connections: retain file API-key/header sources in resolved configuration;
shared `HttpModel::from_configuration` constructs all native model hosts/SDK;
request-time env/literal/explicit command resolution and one-pass header folding;
OpenAI-compatible and Messages auth headers; scoped, bounded command success/
negative cache, refresh/all-source invalidation; explicit host command opt-in;
existing shell raw stdout/cancellation owner. The async launcher now receives the
same configured model rather than reconstructing it and losing value sources.
Inspection/migration never runs commands. Both native providers use the same owner.

Remaining prerequisites: full models schema/discovery/override/KDL policy, TS live
proxy mutation consumers, stored accounts/OAuth/session stickiness/usage backoff/
sibling rotation and actual automatic auth-retry integration. Source ledger row
stays partial/unverified/retained until those consumers move. No provider family
or OAuth capability is certified. Native command sources require the full build;
command timeout/process/fd behavior needs real platform evidence. Stale (>6-hour)
application incremental cache directories were removed to regain build space;
sources, evidence, native-engine and dependency caches were preserved.

Recovery: revert this continuation to cb4216d. The production TS registry and
original settings/account data remain. Consolidated checks: native SDK and CLI
local HTTP requests for each retained wire API through sync/async/RPC/benchmark;
key precedence/exact env/casing/live lookup, header folding/bounds/redaction,
disabled/failed/successful/oversized/non-UTF8 commands, stdout/stderr/stdin/fd/cwd
isolation, caller cancellation/timeout/descendant cleanup, concurrent cache TTL/
refresh/invalidation and inspect-without-execution. Regress normal requests,
streaming usage, tool approval/session lifecycle and core-build diagnostics. Use
`cargo xtask test -p harvest-omp` in consolidated stabilization. Compiler/format/
ownership receipts use ignored `target/migration/conversion-provider-values-*`;
behavioral/installed/platform/performance checks remain deferred.

## Current plugin-event continuation

Owner: `application:conversion-first`; base `165827d`. Reviewed input subset:
extension UI notify/progress/selection data in `extensibility/extensions/types.ts`,
existing native plugin host/SDK protocol event callback, ToolRuntime invocation,
sync/async session event/writer ownership and RPC adapter. Sole owner remains the
same plugin protocol and session event infrastructure. Other extension types and
interaction callbacks are explicitly not reviewed/ported by this subset.

Written/connections: shared author/host event admission -> ToolRuntime event-aware
methods -> sync AgentSession / bounded async worker packets -> existing owner
sequences / SDK / CLI --events / RPC v1/v2. Native output carries session/run/
sequence/tool-call correlation; no plugin data becomes a model message or another
durable writer. Limits: 16 KiB serialized event, 1 MiB total event data, 1,024
per-call events, 64 choices, nonempty parent call ID <=256 bytes. Old SDK methods
retain explicit ignored-event behavior; sessions use the event-aware methods.

Remaining prerequisites: native TUI rendering, widget/status persistence and
reconnect state, interactive choice request/response protocol, full extension hooks
and actual TS loader/UI consumer replacement. The raw-wire accounting follow-up
below now retains consumed frame sizes instead of re-encoding frames. That code
still needs behavioral boundary evidence; event-data limits and wire limits are
separate contracts.
Keep types.ts retained/partial, with full remaining API review owned here.

Recovery: revert this continuation to 165827d. Consolidated acceptance: external
Rust author/host SDK progress/notification/choice events through sync/async/CLI/RPC;
parent/session/run/sequence and commit ordering; unchanged stored/model histories;
author/host size/count/ID/choices bounds; lag/backpressure/reconnect, cancellation,
callback failure, unexpected exit and shutdown; unchanged final-argument approval.
Use `cargo xtask test -p harvest-omp` during stabilization. See
`docs/rust-plugin-protocol.md`; receipts/logs use ignored
`target/migration/conversion-plugin-events-*`. No source deletion, verified
capability or completed interactive workflow is claimed.

## Current process-byte accounting continuation

Owner: `application:conversion-first`; base `34e1876`. Observed gap: plugin host
exchange quota used serialized decoded frames, losing whitespace and unknown
fields. Inputs are the shared Process<T> JSONL/Content-Length readers, receive/
try_receive APIs, plugin host exchange and other MCP/LSP/DAP/SDK consumers. The
same transport remains the sole owner; do not add another framing reader.

Written/connections: internal Received<T> carries typed value plus exact consumed
wire bytes through the existing bounded channel. Value-only receive/try_receive
and SDK framing APIs preserve their interfaces. Plugin exchange uses the new
receive_with_size path and checked addition before dispatch/result. Counts include
JSONL newline/whitespace/ignored fields or Content-Length headers plus body. The
16 MiB limit admits consumed frames; bounded queue prefetch may already hold more
bytes. It is not an OS/process total-output limit. No decoded frame re-encoding is
used to charge incoming plugin traffic.

Recovery: revert to 34e1876. Consolidated acceptance: large ignored fields and
whitespace frames retain decoded values but charge raw bytes; aggregate exact/
over-limit boundaries, repeated calls/cancellation/failure and no event/result
admission after exhaustion; Content-Length optional/extra headers, embedded
newlines, malformed/truncated data; unchanged MCP/LSP/DAP typed receive and SDK
stdio behavior. Use `cargo xtask test -p harvest-omp` during stabilization.
Compiler/format/ownership evidence only under ignored
`target/migration/conversion-process-byte-count-*`. No TS deletion or verified
capability/retirement/removal advances.

## Stabilization queue (deferred, not passed)

- [ ] Plugin frame/handshake/version/schema/name/identity bounds and corruption.
- [ ] External Rust SDK example against copied native host; stdout/stderr isolation.
- [ ] Final-argument approval, denied calls, nested jailed mutations, denied escapes.
- [ ] Cancellation before initialization, during reads/writes/host calls; timeout,
      early exit, response truncation, EOF and bounded stderr/queue flooding.
- [ ] Startup interruption, graceful shutdown/draining and descendant cleanup;
      aborting the owned process must not leave threads or children behind.
- [ ] Deliver declarative events to session/TUI clients and define choice responses.
- [ ] Full plugin discovery/install/update/marketplace/reload and TS callback
      migration guidance; do not equate protocol support with the plugin product.
- [ ] Configuration precedence and profile inheritance/cycles across global,
      project and explicit overlays; missing/invalid/secret-containing inputs.
- [ ] Complete legacy schema/config discovery and YAML/JSONC compatibility plus settings
      UI/edit persistence; explicit profile and credential/KDL migrations.
- [ ] Regress existing provider, browser, native tool and session/RPC contracts
      against the shared owned-schema/configuration changes.
- [ ] Benchmark CRLF-only normalization, exact file sets, binary/text bounds,
      interrupted/failed/skipped edits, isolated workspace writes, usage unknown
      versus zero, score version isolation, inert historical fixture retention.
- [ ] Complete benchmark generation/archive/orchestration and metaharness cutover.
- [ ] MCP initialization/version/ID envelopes, cursor bounds, tools/resources/
      prompts, server roots/ping callbacks and unsupported methods.
- [ ] MCP stdio and HTTP JSON/SSE, owned headers/session identity, cancellation,
      DELETE cleanup, origin-locked credentials, process exit/queues and approval.
- [ ] MCP OAuth, external discovery/JSONC/config precedence and expansion,
      legacy SSE GET/endpoint transport, reconnect/idempotency and pooling.
- [ ] Idle resource/progress/log subscriptions, HTTP resumption, session/TUI
      delivery and atomic dynamic-tool refresh across the shared host registry.
- [ ] Platform process/console/session ownership and descendant cleanup for both
      plugin and MCP users of the shared process owner.
- [ ] Content-Length framing fragmentation, duplicate/oversized/truncated headers
      and bodies, queue/EOF/cancellation and shared JSONL compatibility.
- [ ] LSP initialization, UTF-16 positions/edits/surrogates/overlap/order, server
      callbacks, final-edit approval, stale versions, partial writes and sync.
- [ ] Full language-server discovery/pooling/linters/warmup, diagnostics, resource
      operations/code-action commands/rename_file, editorconfig and recovery.
- [ ] DAP initialization/configurationDone, out-of-order requests/events, final
      target argument approval, launch/attach/stepping/inspection and cleanup.
- [ ] Socket adapters, reverse terminal/child sessions and remaining breakpoint
      wrappers; startup/exit/cancellation, stale references, TUI and platform parity.
- [ ] Native coordinate CPU budgets, large outputs and URI/path/encoding limits;
      do not infer performance or crash recovery from compilation.
- [ ] Run consolidated behavioral, installed-platform and full transition checks;
      update receipts and only then update done/verified/removal claims.

## Benchmark checkpoint compiler evidence

Aggregate `cargo check --locked -p harvest-omp --all-targets --all-features`
passes on the current host; log `target/migration/conversion-benchmark-connections-final.log`.
Formatting and playbook ownership metadata validation pass. This is compiler and
metadata evidence only. Behavioral, installed, performance and platform checks
remain deferred. Receipt: `target/migration/conversion-benchmark-receipt.json`.
That model-run checkpoint made no source-removal or completed-capability claims.

## Checkpoint evidence and recovery

One aggregate `cargo check --locked -p harvest-omp --all-targets --all-features`
passes after connecting the complete checkpoint. Its initial missing BufRead
import was corrected. Logs: `target/migration/conversion-pass-connections*.log`.
Formatting ran once for the final tree. No tests, Clippy, native addon rebuild,
legacy suites or installed product verification were run for this checkpoint.
Machine ownership/card validation is metadata validation only.

MCP continuation writes and connects the stdio/streamable HTTP boundary and
refactors plugin process ownership into the same ordinary Rust implementation.
Its aggregate all-target/all-feature compiler connection check passes, recorded under
`target/migration/conversion-mcp-connections*.log`. No tests, Clippy, addon rebuild
or installed verification is claimed for this continuation. See `NATIVE-MCP.md`
for implemented contracts and the remaining retained behavior.

Language-services continuation writes and connects native LSP/DAP and workspace
text-edit handling. Shared framing supports Content-Length as well as JSONL.
Aggregate compiler connection results are recorded in
`target/migration/conversion-lsp-dap-connections*.log`; behavioral/installed/
performance/platform checks remain deferred. See `NATIVE-LSP-DAP.md` for every
unfinished boundary and the exact current API behavior.

Future stabilization commands: `cargo xtask test -p harvest-omp`, affected strict
Clippy/format, actual native plugin/CLI installation workflows, and the existing
source/capability verification plan. Receipts remain under ignored
`target/migration/`. Existing inherited-removal/platform/runtime blockers remain
open; no old failed result is converted into a pass.

Recovery: scoped revert of the conversion checkpoint, preserving user changes,
all session/settings files, inert inputs and historical benchmark scores. New
configuration loading never rewrites existing config files. Native plugin launch
is host opt-in, and configuration inspection does not start processes.

Completion: implement every retained contract above, connect the final consumers,
run consolidated checks, record exact evidence, then delete eligible superseded
sources. Until then this task stays in_progress and capabilities stay unverified.

## Next conversion frontier

Native language/debug foundations now exist in the reserved paths above. Continue
remaining service/application conversions against these owners; do not duplicate
LSP/DAP/process/registry implementations. Preserve the explicit unconverted
contracts in NATIVE-LSP-DAP.md. Prioritize remaining native session lifecycle and
provider/config/catalog consumers, then native interactive UI; use the generated
feature matrix and source ledger to maintain complete ownership.

## Full feature snapshot from capabilities.json

Generated from the authoritative manifest for this conversion checkpoint. Update
this section from that manifest, never by estimating percentages. “Partial” means
Rust code exists; it does not certify completeness. Parent-task owners remain
responsible for all outstanding contracts and legacy source ownership. Approved
retirements are listed separately from implemented features.

### storage

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:session-v3` | partial | unverified | `migrate:storage` |
| `surface:session-v1-v2-import` | partial | unverified | `migrate:storage` |
| `surface:prototype-session-import` | absent | unverified | `migrate:storage` |

### catalog-providers

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:streaming-providers` | partial | unverified | `migrate:catalog-providers` |
| `surface:oauth-credentials` | absent | unverified | `migrate:catalog-providers` |
| `surface:kdl-catalog` | absent | unverified | `migrate:catalog-providers` |

### memory

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `tool:memory_edit` | absent | unverified | `migrate:memory` |
| `slash:memory` | absent | unverified | `migrate:memory` |
| `surface:memory-backends` | absent | unverified | `migrate:memory` |
| `surface:automatic-session-learning` | partial | unverified | `migrate:memory` |

### plugins

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `compat:single-engine-owner` | partial | unverified | `migrate:plugins` |

### application

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `command:launch` | absent | unverified | `migrate:application` |
| `command:acp` | absent | unverified | `migrate:application` |
| `command:auth-broker` | absent | unverified | `migrate:application` |
| `command:auth-gateway` | absent | unverified | `migrate:application` |
| `command:agents` | absent | unverified | `migrate:application` |
| `command:bench` | absent | unverified | `migrate:application` |
| `command:cleanse` | absent | unverified | `migrate:application` |
| `command:commit` | absent | unverified | `migrate:application` |
| `command:completions` | absent | unverified | `migrate:application` |
| `command:__complete` | absent | unverified | `migrate:application` |
| `command:compress` | absent | unverified | `migrate:application` |
| `command:config` | partial | unverified | `application:conversion-first` |
| `command:dry-balance` | absent | unverified | `migrate:application` |
| `command:gc` | absent | unverified | `migrate:application` |
| `command:grep` | absent | unverified | `migrate:application` |
| `command:gallery` | absent | unverified | `migrate:application` |
| `command:git` | absent | unverified | `migrate:application` |
| `command:grievances` | absent | unverified | `migrate:application` |
| `command:images` | absent | unverified | `migrate:application` |
| `command:if-bench` | absent | unverified | `migrate:application` |
| `command:install` | absent | unverified | `migrate:application` |
| `command:join` | absent | unverified | `migrate:application` |
| `command:models` | absent | unverified | `migrate:application` |
| `command:plugin` | absent | unverified | `migrate:application` |
| `command:ps` | absent | unverified | `migrate:application` |
| `command:say` | absent | unverified | `migrate:application` |
| `command:share` | absent | unverified | `migrate:application` |
| `command:setup` | absent | unverified | `migrate:application` |
| `command:shell` | absent | unverified | `migrate:application` |
| `command:read` | absent | unverified | `migrate:application` |
| `command:render` | absent | unverified | `migrate:application` |
| `command:ssh` | absent | unverified | `migrate:application` |
| `command:update` | absent | unverified | `migrate:application` |
| `command:usage` | absent | unverified | `migrate:application` |
| `command:tiny-models` | absent | unverified | `migrate:application` |
| `command:token` | absent | unverified | `migrate:application` |
| `command:ttsr` | absent | unverified | `migrate:application` |
| `command:worktree` | absent | unverified | `migrate:application` |
| `command:search` | absent | unverified | `migrate:application` |
| `tool:read` | partial | unverified | `migrate:application` |
| `tool:security_scan` | absent | unverified | `migrate:application` |
| `tool:bash` | partial | unverified | `migrate:application` |
| `tool:edit` | partial | unverified | `migrate:application` |
| `tool:ast_grep` | absent | unverified | `migrate:application` |
| `tool:ast_edit` | absent | unverified | `migrate:application` |
| `tool:ask` | absent | unverified | `migrate:application` |
| `tool:debug` | partial | unverified | `application:conversion-first` |
| `tool:github` | absent | unverified | `migrate:application` |
| `tool:glob` | absent | unverified | `migrate:application` |
| `tool:grep` | partial | unverified | `migrate:application` |
| `tool:lsp` | partial | unverified | `application:conversion-first` |
| `tool:checkpoint` | absent | unverified | `migrate:application` |
| `tool:rewind` | absent | unverified | `migrate:application` |
| `tool:task` | absent | unverified | `migrate:application` |
| `tool:hub` | absent | unverified | `migrate:application` |
| `tool:todo` | absent | unverified | `migrate:application` |
| `tool:web_search` | absent | unverified | `migrate:application` |
| `tool:write` | partial | unverified | `migrate:application` |
| `tool:retain` | absent | unverified | `migrate:application` |
| `tool:recall` | absent | unverified | `migrate:application` |
| `tool:reflect` | absent | unverified | `migrate:application` |
| `tool:learn` | absent | unverified | `migrate:application` |
| `tool:manage_skill` | absent | unverified | `migrate:application` |
| `tool:search_code` | absent | unverified | `migrate:application` |
| `tool:sessions` | absent | unverified | `migrate:application` |
| `tool:presets` | absent | unverified | `migrate:application` |
| `hidden-tool:think` | absent | unverified | `migrate:application` |
| `hidden-tool:yield` | absent | unverified | `migrate:application` |
| `hidden-tool:goal` | absent | unverified | `migrate:application` |
| `slash:security` | absent | unverified | `migrate:application` |
| `slash:settings` | absent | unverified | `migrate:application` |
| `slash:setup` | absent | unverified | `migrate:application` |
| `slash:plan` | absent | unverified | `migrate:application` |
| `slash:plan-review` | absent | unverified | `migrate:application` |
| `slash:vibe` | absent | unverified | `migrate:application` |
| `slash:goal` | absent | unverified | `migrate:application` |
| `slash:guided-goal` | absent | unverified | `migrate:application` |
| `slash:loop` | absent | unverified | `migrate:application` |
| `slash:queue` | absent | unverified | `migrate:application` |
| `slash:model` | absent | unverified | `migrate:application` |
| `slash:switch` | absent | unverified | `migrate:application` |
| `slash:fast` | absent | unverified | `migrate:application` |
| `slash:skillful` | absent | unverified | `migrate:application` |
| `slash:extended-context` | absent | unverified | `migrate:application` |
| `slash:computer` | absent | unverified | `migrate:application` |
| `slash:prewalk` | absent | unverified | `migrate:application` |
| `slash:advisor` | absent | unverified | `migrate:application` |
| `slash:export` | absent | unverified | `migrate:application` |
| `slash:trace` | absent | unverified | `migrate:application` |
| `slash:dump` | absent | unverified | `migrate:application` |
| `slash:share` | absent | unverified | `migrate:application` |
| `slash:collab` | absent | unverified | `migrate:application` |
| `slash:join` | absent | unverified | `migrate:application` |
| `slash:leave` | absent | unverified | `migrate:application` |
| `slash:browser` | absent | unverified | `migrate:application` |
| `slash:copy` | absent | unverified | `migrate:application` |
| `slash:open` | absent | unverified | `migrate:application` |
| `slash:todo` | absent | unverified | `migrate:application` |
| `slash:session` | absent | unverified | `migrate:application` |
| `slash:jobs` | absent | unverified | `migrate:application` |
| `slash:usage` | absent | unverified | `migrate:application` |
| `slash:changelog` | absent | unverified | `migrate:application` |
| `slash:hotkeys` | absent | unverified | `migrate:application` |
| `slash:tools` | absent | unverified | `migrate:application` |
| `slash:context` | absent | unverified | `migrate:application` |
| `slash:extensions` | absent | unverified | `migrate:application` |
| `slash:agents` | absent | unverified | `migrate:application` |
| `slash:skills` | absent | unverified | `migrate:application` |
| `slash:git` | absent | unverified | `migrate:application` |
| `slash:hub` | absent | unverified | `migrate:application` |
| `slash:branch` | absent | unverified | `migrate:application` |
| `slash:fork` | absent | unverified | `migrate:application` |
| `slash:tree` | absent | unverified | `migrate:application` |
| `slash:login` | absent | unverified | `migrate:application` |
| `slash:logout` | absent | unverified | `migrate:application` |
| `slash:ssh` | absent | unverified | `migrate:application` |
| `slash:new` | absent | unverified | `migrate:application` |
| `slash:fresh` | absent | unverified | `migrate:application` |
| `slash:clear` | absent | unverified | `migrate:application` |
| `slash:drop` | absent | unverified | `migrate:application` |
| `slash:compact` | absent | unverified | `migrate:application` |
| `slash:shake` | absent | unverified | `migrate:application` |
| `slash:handoff` | absent | unverified | `migrate:application` |
| `slash:resume` | absent | unverified | `migrate:application` |
| `slash:tab` | absent | unverified | `migrate:application` |
| `slash:diagnostics` | absent | unverified | `migrate:application` |
| `slash:pin` | absent | unverified | `migrate:application` |
| `slash:btw` | absent | unverified | `migrate:application` |
| `slash:tan` | absent | unverified | `migrate:application` |
| `slash:omfg` | absent | unverified | `migrate:application` |
| `slash:cleanse` | absent | unverified | `migrate:application` |
| `slash:retry` | absent | unverified | `migrate:application` |
| `slash:debug` | absent | unverified | `migrate:application` |
| `slash:rename` | absent | unverified | `migrate:application` |
| `slash:move` | absent | unverified | `migrate:application` |
| `slash:wt` | absent | unverified | `migrate:application` |
| `slash:add-dir` | absent | unverified | `migrate:application` |
| `slash:remove-dir` | absent | unverified | `migrate:application` |
| `slash:dirs` | absent | unverified | `migrate:application` |
| `slash:exit` | absent | unverified | `migrate:application` |
| `slash:restart` | absent | unverified | `migrate:application` |
| `slash:marketplace` | absent | unverified | `migrate:application` |
| `slash:plugins` | absent | unverified | `migrate:application` |
| `slash:reload-plugins` | absent | unverified | `migrate:application` |
| `slash:force` | absent | unverified | `migrate:application` |
| `slash:live` | absent | unverified | `migrate:application` |
| `slash:pause` | absent | unverified | `migrate:application` |
| `slash:quit` | absent | unverified | `migrate:application` |
| `slash:commands` | absent | unverified | `migrate:application` |
| `slash:sidebar` | absent | unverified | `migrate:application` |
| `surface:settings-profiles` | partial | unverified | `application:conversion-first` |
| `surface:rust-sdk` | partial | unverified | `migrate:application` |
| `surface:extension-process-abi-v1` | partial | unverified | `application:conversion-first` |
| `surface:dynamic-tools` | absent | unverified | `migrate:application` |
| `surface:delegation-goals-advisor` | absent | unverified | `migrate:application` |
| `surface:codegen` | partial | unverified | `migrate:application` |
| `surface:release-installers` | partial | unverified | `migrate:application` |
| `surface:installed-parity-without-legacy-runtimes` | absent | unverified | `migrate:application` |
| `surface:session-tail-recovery` | partial | unverified | `migrate:application` |
| `surface:native-command-completion` | absent | unverified | `migrate:application` |

### terminal

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:terminal-inline-transcript` | partial | unverified | `migrate:terminal` |
| `surface:terminal-input-resize-restoration` | partial | unverified | `migrate:terminal` |
| `surface:native-replay` | absent | unverified | `migrate:terminal` |
| `surface:native-stats` | absent | unverified | `migrate:terminal` |
| `surface:native-collaboration` | absent | unverified | `migrate:terminal` |
| `surface:native-run-dashboard` | absent | unverified | `migrate:terminal` |
| `surface:native-automation-dashboard` | absent | unverified | `migrate:terminal` |

### services

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `slash:mcp` | absent | unverified | `migrate:services` |
| `surface:rpc-v1-v2` | partial | unverified | `migrate:services` |
| `surface:acp-v1` | absent | unverified | `migrate:services` |
| `surface:mcp` | partial | unverified | `application:conversion-first` |
| `surface:lsp-dap` | partial | unverified | `application:conversion-first` |
| `surface:eval-ssh` | absent | unverified | `migrate:services` |
| `surface:native-browser-control` | partial | unverified | `migrate:services` |

### browser

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `command:stats` | absent | unverified | `migrate:browser` |
| `slash:stats` | absent | unverified | `migrate:browser` |
| `surface:stats` | absent | unverified | `migrate:browser` |
| `surface:benchmarks` | partial | unverified | `application:conversion-first` |
| `surface:collaboration` | absent | unverified | `migrate:browser` |
| `surface:python-automation` | absent | unverified | `migrate:browser` |

### media

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:media-local-inference` | partial | unverified | `migrate:media` |

### tooling

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:native-installation` | absent | unverified | `migrate:tooling` |
| `surface:performance-baselines` | absent | unverified | `migrate:tooling` |
| `surface:registry-installation` | absent | unverified | `migrate:tooling` |
| `surface:artifact-signatures` | absent | unverified | `migrate:tooling` |

### Approved retirements

| Interface | Removal state | Replacement or removal contract |
|---|---|---|
| `command:browser-relay` | removed | surface:native-browser-control |
| `tool:eval` | pending | remove-language-eval |
| `surface:browser-wasm` | pending | surface:native-replay, surface:native-stats, surface:native-collaboration, surface:native-run-dashboard, surface:native-automation-dashboard |
| `sdk-import:typescript` | pending | surface:rust-sdk, surface:rpc-v1-v2 |
| `sdk-import:python-rpc` | pending | surface:rust-sdk, surface:rpc-v1-v2 |
| `compat:shell-completion-scripts` | pending | surface:native-command-completion |
| `compat:installer-scripts` | pending | surface:native-installation |
