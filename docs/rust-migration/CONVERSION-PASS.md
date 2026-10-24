# Conversion-first pass: shared ownership and feature register

Active task: **`application:conversion-first`**, state **in_progress**.
Initial base: `64f3dd7`; current metaharness trace-report continuation base: `dd34b68`. Current owner: the active migration agent on `ts-to-rust`.
The latest user instruction changes scheduling: convert connected code first,
then resolve errors and test together. `PLAYBOOK.md` records this across sessions.
Do not revert to per-file test/addon cycles on resume.

## What is written versus what is done

“Written and connected” describes code and consumers. It does not mean behavioral
parity, installation verification, completed migration or eligible source deletion.
The machine manifest retains these distinctions. No feature becomes done from a
compiler result. Most conversion foundations have no behavioral or installed evidence. Feature-specific
local contracts and copied Linux CLI checks for cleanup discovery are recorded in
its reviewed card; they do not establish whole migration or platform parity.

## Reserved interfaces and file ownership

One owner per row. Do not introduce another registry, configuration resolver,
plugin process owner or benchmark scorer beside these paths. Coordinate changes
through this table and the machine task before editing a shared interface.

| Feature | Sole Rust destination | Consumers connected | Remaining work | State |
|---|---|---|---|---|
| Metaharness narrative trace report | trace_report modules; shared provider/configuration/HTTP/session/display/cancel owners | native CLI/SDK, package trace:report and README; HTTP/offline/selected session inputs | shared stored/OAuth credential prerequisite; legacy-auth consumer; consolidation/platform/removal gates | Whole pipeline written/connected; partial and unverified |
| Cleanup discovery | cleanup_scan modules; pi-ast cleanup facts | Rust scan/CLI; both cleanup prompts; package alias | source-bound removal and consolidated/platform gates | Whole feature implemented/cut over; verification pending |
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
| Benchmark model orchestration | `rust-agent/src/edit_benchmark/runner.rs` | `run_tasks` SDK and native `edit-benchmark run`; shared model/session/config/approval | behavioral/installed checks, provider/timeout retries and guided policies, formatted exports, interactive views, metaharness cutover | Written/connected; unverified |
| Benchmark fixture preparation | `rust-agent/src/edit_benchmark/{hunks,prepare,tasks}.rs` | native prepare/validate CLI and SDK; shared pi-diff/jail | full mutation eligibility, corpus/difficulty policies and behavioral/installed verification; legacy generator still active | Written/connected foundation; unfinished |
| Edit-shape calibration | `rust-agent/src/edit_benchmark/shapes.rs` | native CLI/SDK and package edit-shapes command; legacy standalone TS script removed | parser/grouping/coverage/installed regressions; native bounded/unchanged metadata cases | Written/connected; removal verification pending |
| Native tool-result metadata | `rust-agent/src/{edit,tools,durable_session,agent_session,session_engine}.rs` | shared native SDK/sync/async/RPC committed records and benchmark analyzer | actual result/error/no-op/metadata bounds and persistence/replay regressions | Written/connected; unverified |
| Fixture archives | `rust-agent/src/edit_benchmark/archive.rs` | SDK FixtureSource/pack_fixtures, CLI pack/unpack/list/validate/run and shared task loader | malformed/CRC/path/alias/budget/cancellation/platform and installed contracts | Written/connected; unverified |
| Syntax-aware mutation generation/corpus | `crates/pi-ast/src/mutations.rs`; `rust-agent/src/edit_benchmark/generate.rs` | native generate/mutations CLI and SDK -> shared snapshots/hunks -> validate/run | full binding/grammar eligibility, corpus/difficulty/calibrated shapes, legacy generator cutover, behavioral/installed checks | Written/connected foundation; unverified |

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

## Current benchmark attempts, early-stop and native summary continuation

Owner/task: `application:conversion-first`; base `91ee52d`; capability
`surface:benchmarks`. Prerequisites: existing native runner/scorer, durable sync
session, shared configuration/provider/approval and fixture archive owner are
written. Inputs: `packages/metaharness/adapters/edit/runner.ts` retry loop and
buildEarlyStop, its cli.ts import, in-process-client.ts, current Rust runner and
stored tool evidence. Only the ledger's named runner subset is reviewed; remaining
legacy contracts below are not inferred complete.

Sole destinations: edit_benchmark/{runner,evidence,view,cli}.rs, ordinary
AgentSession's optional tool checkpoint, two static Markdown retry prompts.
Connections: existing run_tasks SDK/native run CLI, existing session generation/
writer/event owner, same jailed workspace and model across attempts; native view
CLI and SummaryView SDK read the existing summary. No parallel runtime or scorer.

Written rules: 1..16 scored attempts (default 1), 0..16 no-edit retries (default 2)
with an independent run-wide budget; maximum 32 submits. Per-turn persisted calls
identify no edit/write attempts; old edits never mask a later read-only turn.
Matching output with required successful tool evidence ends the run immediately.
All submits retain session/history/model/workspace and share the original deadline,
usage and artifact limits. Errors/timeouts are not retried as output mismatches.
Retry prompts contain no expected fixture or fabricated diff. No-edit retries do
not spend a scored-attempt slot until their separate allowance is exhausted.

Early-stop is enabled by default, disabled by CLI --no-early-stop-on-match or SDK
configuration. The ordinary session owner checks only after every call in the
assistant tool batch has a durable result; never abort with unresolved calls or
fabricate an assistant response/usage. Use the existing rust-v1 full file-set and
CRLF-only scorer, and honor read/edit requirements. Normal submit/execute callers
supply no checkpoint and preserve existing behavior. Final scoring still runs.

Numbered attempt/score/usage and optional messages artifacts preserve each turn;
usage snapshots are cumulative and final usage/messages preserve partial failed
turns. Output remains new-directory-only. Outcomes expose attempt/retry counts and
early-stop. Read-only view --output DIR [--json] accepts only complete native
summary files <=2 MiB and <=1000 ordered runs with consistent totals/states;
unknown old attempt fields remain unavailable. Text labels are JSON escaped.
The view never opens paths supplied by a report or contacts providers. It is a
static native report, not the full interactive metaharness dashboard replacement.

Bounded prerequisites to continue independently:
1. Typed provider failure classification and same-session recovery/backoff;
   first-event/attempt-timeout telemetry with retained retry budgets. Do not retry
   arbitrary errors or reset the outer run deadline.
2. Guided prompt/difficulty/mutation-intent policies and formatter retirement
   mapping to rust-v1, without claiming historical score equivalence.
3. Full retained metaharness run/report/queue analytics and interactive native
   views; move cli.ts and actual adapter consumers only after their contracts move.

Deletion: zero. runner.ts, in-process-client.ts and all shared TS inputs remain
live. Recovery: revert this coherent continuation to 91ee52d; default production
TS behavior and immutable baseline are unchanged. Acceptance during consolidated
stabilization: scripted SDK and installed native runs cover the rules above,
repetition/concurrency determinism, deadline/cancel/error/noop/unresolved/denied
calls, mixed usage and artifact failures, tool-batch ordering/no extra provider
request, old submit behavior and malformed/oversize/inconsistent/terminal-control
views. Commands: cargo xtask test -p harvest-omp, installed edit-benchmark run/view,
source-audit affected plan and final removal gates when consumers can be deleted.
Conversion checks: cargo fmt -p harvest-omp -- --check;
cargo check --locked -p harvest-omp --all-targets --all-features;
existing xtask playbook check. Ignored receipt/log prefix:
target/migration/conversion-benchmark-retries-. No behavioral/installed/platform
verification, completed capability or verified source removal is claimed.

## Typed benchmark provider recovery continuation

Owner `application:conversion-first`; base `6dfa7cb`. Prerequisite discovery:
provider.rs already exposes typed ProviderRequestError, so no new transport owner
or string error classifier is needed. Inputs: legacy detectProviderFailure,
pre-mutation guard and exponential retry loop; shared native transport/model,
command credential invalidation, attempt evidence and native view. Same source
subset/capability/consumers as the preceding card remain partial and retained.

Connections: existing benchmark submit loop -> typed error downcast -> same
session/model/workspace and original deadline; Model's default no-op cancellable
invalidate_command_credentials seam -> HttpModel's existing command-source
invalidation -> shared config resolver. MeasuredModel forwards the seam. Other
model implementations remain compatible; this is not general session retry,
OAuth, account rotation or a new permission to execute credential commands.

Rules written: CLI/SDK provider_retries defaults 3, range 0..16. Only typed
HTTP 401/403, 408/425/429, 5xx and transport IO qualify. No read-only turn with
an edit/write/unknown mutation call qualifies, even if that call failed or was
denied. All non-read/grep calls conservatively count as mutation attempts.
Auth retries invalidate all command-backed keys/headers through existing owner;
next request resolves normally and still honors explicit command opt-in.
Backoff 1/2/4/8/10 seconds, capped 10, checks cancellation/deadline every 20 ms.
No retry consumes a scored or no-edit slot; total submits now at most 48.
Timeout/session/malformed/body/unknown errors, other statuses, post-mutation
failures and exhausted budgets still fail. No automatic auth refresh in ordinary
session clients is claimed.

Persist redacted provider-retry-NN.json with kind/status/attempt/count/delay,
without server text/URL/headers/secrets. Outcomes and summary view expose provider
retry count; older summaries retain unavailable. Failed request count is distinct
from received responses; any failed request makes usage unavailable even after
successful recovery, while known partial totals/raw usage remain preserved.
The static report header now uses explicit fragments so tab escapes cannot be
rewritten into a broken header by format wrapping.

Guidance discovery: native tools currently expose exact replacement, while the
retained guided path specifically requires hashline read provenance and session
snapshot ownership. Do not emit hashline prompts for an incompatible native tool.
Bounded prerequisite `application:hashline-host-bridge` is planned/review pending
in tasks/HASHLINE-HOST-BRIDGE.md; same shared files remain reserved to this owner.
First-event/attempt-timeout telemetry/recovery remains a separate prerequisite.

Acceptance during consolidated stabilization: fake/local HTTP SDK and installed
runs for each selected/nonselected status, transport error versus malformed data,
mutating/denied/unknown calls preventing retry, budgets/deadline/backoff cancellation,
command cache/header refresh with opt-in disabled/enabled, shared model clones,
partial stream/failed request usage availability, redacted artifacts and summary
compatibility. Commands as preceding card, with cargo xtask test -p harvest-omp
and installed run/view later. Conversion: compiler/fmt/playbook only; ignored
receipt prefix target/migration/conversion-benchmark-provider-retries-.
Recovery: revert this continuation to 6dfa7cb. Zero TS deletion or verified count
advance; full metaharness and credential consumers still need to move.

Shared transport refinement in this continuation: request-builder/configuration
and response-decoding errors retain distinct redacted InvalidRequest/InvalidResponse
types and cannot enter the selected IO retry path. MCP and other transport
consumers preserve their existing owner; regression evidence is pending.

## Cooperative attempt timeout and recovery continuation

Task/owner: `application:conversion-first`; base `b5ccadf`; capability
`surface:benchmarks`; same retained runner.ts ledger subset. Prerequisites: existing
native shared CancelToken child deadlines, sync session lifecycle, benchmark
attempt/evidence/usage/artifact loop and typed provider recovery. Inputs: legacy
PromptTimeoutError/PromptAttemptTelemetry and maxTimeoutRetries loop, native
cancel.rs, runner/evidence/view/CLI. Destinations remain these existing owners;
no new timer thread, runtime, transport or session writer.

Written/connections: CLI --attempt-timeout-seconds N or SDK attempt_timeout
explicitly enables a cooperative child deadline per submit. Default None preserves
the previous overall-only timeout. N is positive and <= overall timeout; every
child still clamps to the parent's deadline. timeout_retries defaults 3, range
0..16, separate run-wide allowance, no scored/no-edit/provider slot consumption;
at most 64 submits across all budgets. Same model/session/workspace persists.
Only typed Timeout AbortError qualifies; runtime cancellation preserves the exact
existing Aborted diagnostics and now exposes its cause without text matching.
User/signal/unknown aborts and textual lookalikes do not qualify. A parent timeout
or cancellation always wins; retries never extend/reset that outer budget.

Every timeout/provider recovery also requires zero mutation calls and zero
unresolved calls. Evidence tracks pending IDs across all tool families, including
grep/unknown names, rather than only read/edit. Never send a follow-up after an
interrupted multi-call batch with missing results. Failed/denied mutation attempts
block retry. Exhausted or unsafe attempt timeouts produce timed_out outcomes with
attempt_timeout reason; other errors retain existing classification. Custom SDK
models must cooperate; this is not forced interruption of arbitrary model code.
No first-event/connection timer is implemented by this submit deadline.

Each attempt records elapsed submit microseconds, delivered event/text-delta
counts, last delivered event type, committed message count and typed timeout flag.
Counts describe existing session events (including lifecycle/failure), not proof
of provider activity. Immutable timeout-retry-NN records recovery counts/telemetry
without raw provider diagnostics. Outcomes and native views add timeout_retries;
old summaries show unavailable. Failed request usage remains unavailable after
recovery. Artifact failures are not recovered as timeouts.

Next bounded prerequisites: first-provider-event deadline and idle/stall telemetry
need a defined shared transport/session notification contract; do not use local
Started as remote activity or add a detached watchdog. Hashline guidance stays in
application:hashline-host-bridge; general metaharness consumers remain retained.
No TS deletion in this continuation. Recovery: revert to b5ccadf. Stabilization:
scripted/cooperative models and local HTTP through SDK/installed run/view cover
parent/child deadline precedence, cancellation, typed errors through IO/context,
textual errors rejected, retry exhaustion/counts, complete read/grep batches,
unresolved tool batches, denied/failed mutations, same-session continuity,
failed usage and artifact writes, exact old cancellation diagnostics, all native
cancel users and old summary parsing. Run cargo xtask test -p harvest-runtime and
-p harvest-omp in consolidation, with installed/platform checks later.
Conversion checks: cargo fmt -p harvest-omp -p harvest-runtime -- --check;
cargo check --locked -p harvest-omp -p harvest-runtime --all-targets --all-features;
existing xtask playbook check. Ignored receipt/log prefix:
target/migration/conversion-attempt-timeouts-. Compilation/ownership evidence
only; retained capability/source removal and installation states stay unverified.

## Native corpus inspection and source selection continuation

Owner `application:conversion-first`; base `2d80e17`; retained capability
surface:benchmarks; source generate.ts remains partial/retained. Inputs: legacy
collectFiles/isExcluded/filterFiles/hasStructure, current native generator,
shared bounded task traversal and pi-ast mutation selector. Prerequisites are the
existing generator/hunk/task/archive owners. No new parser, filesystem walker,
random generator, scorer or fixture publisher is introduced.

Destinations: edit_benchmark/corpus.rs holds policy/report; existing tasks.rs
traversal accepts an internal pruning predicate and cancellation token, with
ordinary list_files callers retaining the all-files behavior. CLI corpus and
inspect_corpus SDK use the shared source policy. generate --source-policy and
new generate_task_with_policy SDK use the same policy before existing seeded AST
selection -> unchanged snapshot mutation/hunks/prepare -> pack/validate/run.
generate_task SDK keeps reviewed-tree as its default. Generation reads selected
source paths once, retaining bytes in the existing temporary immutable expected
snapshot; corpus inspection is a separate read-only invocation, not a duplicate
scan on every generation.

Policies written: reviewed-tree preserves the curated source-tree snapshot and
copies unsupported files as context. Explicit legacy-packages starts at packages/
and prunes __tests__, __mocks__, fixtures, fixture, dist, build, node_modules,
scripts. Only .js/.jsx/.ts/.tsx survive path selection; filenames containing
.test./.spec./.fixture. are excluded. Its copied corpus admits only nonempty UTF-8
JS/TS/TSX <=256 KiB, 30..800 LF-split lines and one literal structure token
function-space/class-space/export-space/=>. These are coarse lexical filters, not
syntax/mutation-candidate validation. Other admitted files are omitted from this
explicit policy's fixture snapshot. Missing packages/no viable candidate fails;
no silent fallback to reviewed-tree or language runtime. Nothing clones or
executes source data.

Limits: existing depth 64, 10,000 visited entries/files, individual file bound and
64 MiB admitted path-tree bytes; inspect checks total read bytes too. Corpus paths
must be UTF-8, <=4096 bytes each and <=1 MiB combined. Cancellation is checked
during shared traversal and reads. New generation also checks prospective output
against the original source tree through the existing path resolver before
creating output; snapshots/output must not overlap the original source.

Corpus report contains sorted path/hash/bytes/LF line count/coarse eligibility and
reason, rust-corpus-v1 and policy name. Rejected content is not emitted. Pruned
paths are outside the inspected set. CLI corpus returns 1 if none are coarsely
eligible; this does not prove a requested mutation can be generated. Producer
metadata/report record source policy/version and scanned/retained file counts;
rust-ast-v1 mutation selection and rust-v1 score meanings remain separate. Old
Babel source discovery/difficulty/seed/formatter behavior is not certified.

Remaining: difficulty/repeated-block/range scoring, dedup/calibration/case quotas,
source repository acquisition, specialized prompts/guidance/full grammar and
legacy package/metaharness consumers. Keep generate.ts and shared TS hunk/parser
consumers. Deletions zero. Recovery: revert this continuation to 2d80e17.
Consolidated acceptance: native corpus/generate/validate/pack/run SDK and installed
CLI against curated and packages fixtures, line boundaries/structure/name rules,
unsupported/binary/empty/large sources, pruning/symlink/path/entry/depth/byte caps,
source/output overlap/aliases, cancellation and original-byte preservation,
deterministic same-policy seeds, expected policy provenance and coarse filter
versus actual AST candidate distinction. Use cargo xtask test -p harvest-omp and
installed native commands later. Conversion checks: cargo fmt -p harvest-omp --
--check; cargo check --locked -p harvest-omp --all-targets --all-features; existing
xtask playbook check. Receipt prefix target/migration/conversion-corpus-selection-.
No behavioral/installed/source-removal or retained parity evidence yet.

## Native difficulty and region selection continuation

Owner/task `application:conversion-first`; base `518458c`; source generate.ts and
capability surface:benchmarks remain partial, unverified and retained. Inputs:
legacy repeated-line, block/density/indent/function-range analysis,
scoreDifficulty/getCandidatesForDifficulty, regionAvailable/recordRegion and
current corpus/AST generator. Prerequisites: existing native source policy,
seeded AST candidates, shared prepare/hunk/archive/run owners. Destinations:
edit_benchmark/{difficulty,regions}.rs and existing corpus/generate/CLI. No second
parser, scorer, filesystem owner, random engine or mutable history store.

Connections: corpus --difficulty and inspect_corpus_with_difficulty expose lexical
metrics/file preference; generate --difficulty/--min-score and
GenerationOptions/generate_task_with_options filter actual shared AST candidates;
existing generator -> expected snapshots/mutations/hunks/publication -> archive/
validation/run stays the sole production path. Old generate_task/with_policy
callers default to no difficulty/history and preserve their seed draw sequence;
that compatibility still requires runtime evidence. Corpus eligibility is still
coarse source eligibility, not named mutation/minimum-score success.

Written difficulty policy rust-difficulty-v1: easy prefers <150 lines and <3
repeated-line groups; medium 100..300 lines; hard >200 and >=3 regex block matches;
nightmare >200 with a repeated-line group. Default minimums 0/2/5/8, optionally
overridden only with an explicit level. Scores retain lexical file-size/middle/
repetition/block/density/indent/trivial-context/similar-name contributions.
Whitespace and lengths use ECMAScript whitespace and UTF-16 units; regex names/
boundaries remain ASCII. Function ranges intentionally retain the legacy raw
brace-count and first-same-named-range heuristics, not semantic binding claims.
Shared pi-ast still owns syntax and mutations. Metrics <=256 KiB/4000 lines;
identifier sets <=4096 and similarity comparisons <=1,000,000 per file, cached by
function/line and cancellable. Budget failures are explicit, not fake low scores.

Native sampling exhaustively considers bounded AST candidates, counts candidates
passing the minimum, and keeps separate preferred/fallback reservoirs through the
existing seeded PRNG. Use fallback only when no preferred candidate passes; report
that choice and counts. This differs from legacy random attempt limits, fallback
scheduling and difficulty distributions; no historical seed/result equivalence.
No policy silently lowers the requested minimum or mutates source data.

Written region policy rust-regions-v1: output all selected edit spans as
path/original_sha256/start_line/end_line (deduplicated), and accept an explicit
read-only JSON array via --exclude-regions or SDK excluded_regions. All edits,
including distant composite/binding edits, participate. Matching relative path
and exact source hash are required; changed-source history does not exclude the
new content. Exclude intersections with prior spans plus three lines, including
insertions. Index/merge intervals per source before candidate sampling. History
<=4096 records, normalized relative path components, <=4096 UTF-8 bytes/path and
1 MiB combined paths, lowercase SHA-256 and positive bounded line spans; CLI input
<=1 MiB. A source-hash/path pair never opens a path from history. No automatic
history mutation, cross-process persistence, resumable batch owner or complete
content deduplication is implied. Producer metadata/report carry policy versions,
selected score, fallback/counts, excluded candidate count and selected regions.
Existing bounded CLI document reads now serve scoring and region inputs together.

Implementation/deletions: Rust connections written, zero TS removal. Legacy
package generate.ts, its specialized prompts/formatting/mutation helpers and
metaharness remain active. Next: multi-case/shape quota/calibration orchestration,
content deduplication, full retained producer prompts and hashline host bridge,
then actual consumer moves. Recovery: revert this coherent batch to 518458c.
Stabilization: scripted/installed corpus/generate/validate/pack/run cases for
metrics and source boundaries, JS whitespace/astral units, first duplicate-named
function behavior, preferred/no-preferred/minimum rejection, old seed paths,
complete multi-edit exclusion/insertion/EOF/newline/changed-hash boundaries,
malformed/oversized history, unsafe paths, cancellation/work bounds, unchanged
source bytes, deterministic metadata, immutable original scores and all existing
source-policy consumers. Run cargo xtask test -p harvest-omp and copied installed
CLI later. Conversion checks: cargo fmt -p harvest-omp -- --check;
cargo check --locked -p harvest-omp --all-targets --all-features; existing xtask
playbook check. Receipts/logs: target/migration/conversion-difficulty-*.
Compiler/format/ownership only: no verified parity, installed or source-removal
count advances. Keep this task in progress and the legacy ledger rows retained.

## Native multi-case generation continuation

Owner/task application:conversion-first; base 8c19c66; same generate.ts source and
surface:benchmarks remain partial/retained/unverified. Inputs: legacy main case
loop/usedLines and current corpus/difficulty/regions/generator/prepare/archive.
Prerequisites: these existing engines and publisher. Destinations: batch.rs,
existing generate/prepare/CLI/mod; existing archive path normalization is reused
for portable case IDs. No parallel writer, parser, scorer or mutable history DB.

Connections: generate-batch --source DIR --output NEW_DIR --plan FILE and Rust
generate_batch -> shared source policy/inventory -> immutable temporary snapshot
-> existing native generate_task_owned/difficulty/region engine -> borrowed shared
TaskCollection owner -> same task directories, hunks and prompt-last publisher ->
existing validate/list/pack/archive/run. Normal single-task generation/preparation
still acquires its original fixture lock and requires an empty output root.
Collection generation holds that same .fixture-owner resource lease throughout;
inner case publication borrows it rather than reopening a lock or weakening the
standalone empty-output rule. Root plan/journals/results use the same jailed owner.

Plan: ordered JSON array, 1..64 records with exact task_id/mutation/seed; IDs use
existing validation plus archive portability and reject case aliases/duplicates.
Global GenerationOptions supplies source policy/difficulty/minimum/initial region
exclusions. Unknown kinds and invalid plans/history fail before output creation.
CLI plan/history documents <=1 MiB. No automatic random attempts, missing-case
fallback, shape quotas or historical distribution equivalence is claimed.

One selected source snapshot is captured before output ownership/publication;
read hashes must match inspected inventory, and output must be disjoint from
original source. Later cases never reread live original data. Repeated cases use
the existing generator over that snapshot (not a new mutation implementation).
Successful selected regions are carried into subsequent cases; the 4096-region
budget remains explicit. Future quota/calibration/content-dedup/resume work needs
its own contract. Cases run serially, in plan order, with stable supplied seeds.

Bounds: preflight estimates 2*source bytes +512 KiB/case <=48 MiB, conservative
entry count <=10,000 and paths fit existing archive depth. Exact create-only
writes (task inputs/expected/prompt/metadata and root JSON) are charged through
TaskCollection against an actual 64 MiB limit; each JSON artifact <=4 MiB.
Snapshot/source byte limits and generation work/candidate/history limits remain.
Charge is conservative after failed writes and never refunded to fake capacity.
Cancellation propagates to the existing snapshot/generator/hunk/copy owners.

Artifacts: generation-plan.json includes version rust-batch-v1, explicit cases,
options, initial exclusions, source inventory hashes and planned bytes;
generation-case-NN.json is published for every attempted case; generation.json
reports completed/failed/cancelled/not_started cases and successful/unsuccessful
counts. A generation failure stops further cases, preserves completed prefix and
records bounded diagnostics; CLI returns 1 for incomplete plans. Setup/artifact
I/O or output quota errors can fail the whole invocation, preserving prior files.
Partial task directories are not certified or silently erased; validation/archive
may refuse them. Retry into a new directory; there is no crash resume. Prompt is
still the last per-task publication. No repository/source code is executed.

Deletions: zero. Legacy generate.ts/package scripts/metaharness retain their
calibration/prompts/grammar/orchestration consumers. Recovery: revert to 8c19c66.
Consolidated acceptance: scripted/installed generate-batch -> list/validate/pack ->
archive run against multiple mutations/seeds; immutable original/snapshot source
and changed-source detection; ID/alias/path/kind/history/memory/disk/entry/depth
limits, exact quota boundary, mutation-region carry, lock contention with other
batches/single prepare, partial/cancel/interruption/report failure/no resume and
unchanged single-case behavior. Run cargo xtask test -p harvest-omp and copied
installed workflows later. Conversion checks: cargo fmt -p harvest-omp -- --check;
cargo check --locked -p harvest-omp --all-targets --all-features; existing xtask
playbook check. Receipt/log prefix target/migration/conversion-generation-batch-.
Behavioral/installed/platform/source removal remains unverified; no done claims.

Batch report fixture.output is relative to its collection root. Original source
roots/temporary snapshot paths are omitted from successful journals; standalone
prepare/generate report paths retain their existing behavior. Cross-machine
reproducibility and portability still require the recorded installed checks.

## Whole cleanup discovery feature cutover

Owner `tooling:cleanup-discovery` (same active migration agent); base 8b30730.
Read tasks/CLEANUP-DISCOVERY.md for the reviewed whole-feature contract and exact
inputs/consumers/evidence/recovery. All six reports, corpus/reference indexing,
AST facts/barrel closure, clone normalization/grouping, rankings and JSON/text
CLI move together. Two cleanup prompts and root developer alias now reach Rust;
the stale old index entry and scripts/cleanup-scan.ts are removed. This is a real
legacy entrypoint/consumer deletion, not another parallel primitive.

Native module ownership is reserved to this leaf; do not introduce another
cleanup scanner or TypeScript runtime. It reuses shared cached AST, filesystem,
text and bounded-output owners. Shared ECMAScript whitespace patterns now have
one host utility; benchmark difficulty uses the existing pi-edit whitespace
predicate instead of a duplicate implementation.

The inventory was missing this retained developer feature; surface:cleanup-discovery
is registered rather than hidden under an unrelated capability. Implementation
is complete but behavioral/lifecycle stages stay unverified/pending and the task
in progress until source-bound/consolidated/platform gates accept evidence.
Five focused contracts, copied Linux CLI runtime-free PATH checks, and historical
oracle row comparisons are recorded; broad stabilization remains deferred. Actual
TS deletion: 1 file, 492 lines/17,304 bytes; verified source-removal count does not
advance from the deletion alone. Original baseline and inherited missing evidence
remain intact. Receipt prefix conversion-cleanup-scan-* and cleanup-oracle/ under
ignored target/migration. Continue other connected ready features after this
checkpoint; do not rewrite or abandon this completed code/consumer move.

## Metaharness narrative trace feature continuation

Base dd34b68; sole leaf owner application:metaharness-trace-report. Read its card
for full inputs/destinations/connections/limits/recovery/stabilization queue. The
whole pipeline is written together: native normalized trace/metadata fetch,
offline data, shared persisted-session branch projection, result grouping across
notices, bounded ordered map workers, static map/reduce prompts, four-attempt
cancellable requests, Story Arc/failure analysis and separate map/reduce usage.
CLI/SDK/package trace:report/primary README use existing provider/config/session/
HTTP/jail owners. Shared provider output limits are opt-in; ordinary callers
retain absent limits. Benchmark and report retries share runtime cancel::wait.
No new parser, provider/credential store, process owner or runtime is introduced.

Six scripted contracts are written for consolidation; this conversion checkpoint
uses compiler/format/ownership checks, not their execution or installed evidence.
Metadata/trace identity, bounded input/output, tool-call rejection, native branch
private-data omission, output locking/atomic publication and known native versus
legacy differences are documented. Model prose is untrusted analysis, not tests.

Legacy trace-report.ts and explicit trace:report:legacy-auth alias remain for
stored/OAuth consumers. Bounded prerequisite application:legacy-provider-credentials
is planned/review pending; its card identifies exact shared sources and review
needed before ready. Do not implement per-leaf auth or delete that last consumer.
Actual TS deletions zero here. Capability/source states stay partial/unverified/
retained; compiling a new native report does not certify all metaharness behavior.
Receipt/log prefix target/migration/conversion-trace-*. Continue other connected
ready consumers while credential review and consolidation remain pending.
