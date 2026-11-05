# Current Rust migration handoff

## User stop/export instruction (2026-11-05)

The user stopped local migration execution and requested pushing every pending
project change to GitHub, followed by complete local Harvest uninstall/data/cache/
checkout deletion. This source checkpoint remains uncompiled and untested; preserve
all partial/unverified states and the original baseline. Do not automatically resume
migration or recreate the local checkout. A later explicit user request is required
to resume from the saved remote main. Earlier continuation instructions below are
historical context. Local runtime credentials/data/caches are not source backups
and are not included in this commit.

## Native skill invocation/resource continuation (2026-11-05)

Current work stays directly on `main`, uncommitted. No new branch/worktree,
compiler/test/Clippy/addon run, commit or push. Rustfmt and diff hygiene only;
all code below remains **unverified**, with no parity or source-removal advance.

Connected workflow: enabled catalog -> explicit `/skill:<name>` or mid-prompt token
-> both synchronous AgentSession and asynchronous SessionEngine -> persisted user
message -> provider/replay. The existing selected skill snapshot supplies the body,
source path and directory. Static `skill-invocation-native.md` supplies instructions;
no skill becomes a system message or expands approval/workspace permissions.
A bounded JSON user-content envelope (`kind=harvest_skill_invocation_v1`, camel-case
originalPrompt, skill, instructions) retains the selected body and original request
across replay without a new session record kind or protocol field. Memory recall/
retention uses originalPrompt only for user messages; cursor digests continue to
hash actual durable content. Assistant messages/tool results are not reinterpreted
as invocation envelopes. Unknown/malformed skill names remain ordinary user text;
source/index errors still fail closed before the user message is committed.
Cancellation is checked again after expansion and before commit.

Native TUI `/skills` opens the existing completion menu with `/skill:`. Completion
uses the same enabled catalog on the existing bounded discovery worker, filters
before its 100-item cap, and retains stale-draft/cursor publication checks. Hidden/
model-disabled listings remain available to explicit user commands. Transcript
rendering and editor history show originalPrompt rather than the JSON/body;
recovery uses the editor's existing entry and aggregate byte caps.

`skill://name/<relative-path>` now reads bounded UTF-8 companion files or sorted
bounded directory metadata. It resolves the selected source capability and skill
base through JailedFs, rejects absolute/traversal/encoded/drive-like paths and
symlink escapes from the skill directory, reuses resolved paths for I/O and grants
no numbered-line edit provenance. The selected skill body's existing snapshot read
is unchanged. No executable script is run by resource access.

Pending: Agent Plugin skill-source manifests and package containment grants,
percent-encoded resource compatibility, non-text/image resource previews, complete
legacy custom-message/attribution interchange and user-facing catalog diagnostics.
The native JSON envelope is an explicit native persisted representation, not a
claim of legacy custom-message byte parity. Full behavioral/security/cancellation/
replay verification, installed/Windows evidence and TypeScript consumer retirement
remain deferred. Keep legacy skills.ts, skill-command.ts and skill-protocol.ts until
all remaining consumers move. Other editor/LSP/memory/plugin gaps below remain.

Exact next connections: continue plugin skill-source ownership/containment through
the existing plugin/index owners, special-write mutation receipts through existing
writers, and the remaining editor/history/memory surfaces. Preserve this batch and
keep the original source baseline and partial/unverified gates.

## Main conversion continuation: skills, plugin choices and LSP (2026-11-05)

Written directly on existing `main`; no new branches/worktrees, commits or pushes.
This batch is **uncompiled and untested**. Rustfmt and `git diff --check` are the
only executed checks. All original baselines, retained legacy consumers and final
acceptance gates remain. Earlier test results below do not verify this batch.

Connected native paths:
- `managed/catalog.rs` uses the existing brain snapshot engine for enabled
  project/user/custom skill sources, authored precedence, managed revision
  consistency, hidden listings and exact `skill://name` reads. Generic brain
  context excludes skill documents/companions to avoid shadowed/hidden leakage.
  Sync and async model loops refresh only their known transient context message
  before subsequent requests. They preserve stored history and protocol blocks.
- `manage_skill`, `presets` and skill-producing `learn` now use the same selected
  agent directory as discovery/settings. Child sessions inherit that profile,
  settings and memory owner without reopening the parent's database. Explicit
  external memory owners and evaluation opt-outs remain host-controlled.
- Plugin SDK `Context::choose` -> correlated `UiRequest` -> host Approval choice
  seam -> existing bounded native TUI approval channel -> `UiResponse`. Cancellation,
  dismissal, index validation, message limits and complete paged review are written.
  Choice data never authorizes a tool. Plain Choices events stay display-only.
  Additive v1 frames require updated host/SDK for this optional interaction; older
  peers fail explicitly. RPC/custom hosts default to UI-unavailable until wired.
- Opt-in native LSP discovery loads the existing declarative defaults.json plus
  bounded workspace configuration; explicit server names override discovered ones.
  No duplicate default registry or process owner. Bazel includes the defaults asset.
  Startup approval binds the actual command, args, cwd and environment fingerprint.
- Existing LSP client owns version-aware push/pull diagnostics and the bounded
  12-second deferred queue with disk fingerprint/stale-version checks. Native tool
  dispatch polls existing clients without starting servers. Static diagnostics
  prompt appends data to the same correlated result only when the original output
  fits; large file results stay intact and diagnostics remain queued.
- NativeWriter preflights canonical resolved-path metadata before publication.
  Edit success and structured partial-commit reports, plus ordinary filesystem
  writes, synchronize overlays/watchers/diagnostics in existing clients. Create,
  update, delete and rename identities use the existing WorkspaceChange model;
  partial moves report only their landed destination/source operations. Plain-write
  create/update classification uses the pre-write observed state, without a CAS
  guarantee against external writers. Native
  reconciliation preserves the engine's fresh edit snapshots; LSP-origin changes
  still invalidate them. Notification failures preserve committed tool results.

Remaining connections/limits (do not mark complete): the compatibility path-only
notification API still reports live targets as Changed; normal native tool results
now use the operation-aware bridge. Archive/SQLite/conflict-resolution writes lack equivalent
committed-path receipts. Native-tool Err paths without structured progress cannot
claim synchronization. Oversized outputs may omit a synchronization notice; explicit
LSP diagnostics remain the recovery surface. Workspace/global/plugin LSP source
parity, full checkers/mux warmup, directory/binary/recursive/overwrite writer ops,
ACP/cache/auto-repair consumers remain. Basic skill invocation and jailed companion reads are connected above; full
plugin-source semantics and legacy custom-message interchange remain. Plugin RPC reply broker,
durable UI reconnect and broader overlays remain. Managed eval/architect runners,
revision crash recovery, legacy SQLite history import/FTS, native local/smol/vector/
Hindsight memory and complete legacy bank recovery remain unfinished.

Exact resume: continue these existing owners and missing real callers, especially
special-write receipts and plugin skill-source/resource grants;
then move other dependency-ready inventory domains. Do not add a second writer,
registry, backend or UI broker. Preserve partial/unverified ledger states. Tests,
compiler/Clippy/addon/installed loops stay deferred until integration scheduling.

## Current shared-checkout continuation (resumed 2026-11-04)

Latest user instruction: work directly on MAIN and shift the entire owned
codebase to Rust; NO TESTS right now. Compiler/Clippy/addon/test loops are deferred
for the conversion sweep. No new branches or worktrees. The existing five
migration commits were fast-forwarded onto local main without a merge commit;
the entire tracked uncommitted diff was hash-checked before/after and preserved.
The historical codex branch below is a recovery reference, not the active branch.
All current workers edit disjoint files in this one main checkout. No worker
commits, pushes or changes Git refs. Keep unverified/partial states honest.

Active domain ownership: native editor undo/completion/history + UI consumer;
LSP ordered workspace resource/text edits through the existing NativeWriter;
memory configured-provider/subagent connections using existing host/backend;
coordinator owns bounded native Markdown renderer, shared interface exposure,
CLI wiring and shared records. No tests are written or run for this sweep.
Prior test results below are historical and do not verify these new edits.

## Main conversion checkpoint: native editor, LSP and memory (2026-11-04)

All code below is written directly in the existing main checkout, uncommitted.
NO tests, compiler, Clippy or addon execution was performed for this conversion
batch. Rustfmt syntax/formatting and diff hygiene only. Earlier checks further
below apply to earlier source, not this tree. No verified/done/removal advances.
The workers reached their usage limit; their partial edits were preserved and
the coordinator finished missing memory host calls and formatting connections.
No background writer or automatic continuation is implied by this checkpoint.

Written real connections:
- Native editor: bounded undo/redo, grapheme-safe edits, stale-draft-checked async
  path/command completion with visible selection, history search/recall, native
  help/clear commands. Persistent normalized unique prompt history/provenance
  uses bounded locked atomic history-native-v1.json, preserving legacy history.db.
- Markdown: new harvest-tui::markdown::render is called for both committed and
  streaming assistant output. CommonMark headings/lists/links/tasks/tables/code
  use shared Unicode/ANSI width and syntax engines; raw and entity-decoded
  controls are sanitized. Input/output/row/code/table/nesting limits are explicit.
  Math remains literal source and images are labels; rich math/image/theme/overlay
  parity is pending. pulldown-cmark 0.13.4 is in the generated Cargo lock. Offline
  metadata could not fetch an unrelated uncached any_spawner package; lockfile
  resolution completed without builds and preserved existing dependency pins.
- LSP: ordered file create/text/rename/delete uses whole-plan jailed/version/
  alias/preimage preflight and complete final-argument approval with parent tool/
  server identity. Existing NativeWriter publishes sequentially with real prefix/
  partial-move reports. Runtime reconciles watchers/overlays/diagnostics and shares
  session EditStore invalidation across configured clients, reconnecting broken
  clients on later invocations. Direct and server-initiated edits use this owner.
- Formatting: language_services/formatting.rs resolves bounded workspace-local
  EditorConfig sections/root inheritance with the shared pi-walker glob normalizer,
  then content indent detection and the existing fallback. Client formatting calls
  it directly. Parent configurations above the workspace capability require host
  options; complete cross-boundary/EditorConfig compatibility remains pending.
- Memory: configured remote mode uses the existing Arc<Model>/credentials/client
  through a central per-request generation-limit seam; HttpModel clones existing
  configuration and caps to its host output limit. No new model registry/client,
  fabricated embeddings or paid smol fallback. Scoped cancellable callback guards
  are thread-bound. Static provider/task prompts replace inline task construction.
- Child tasks inherit the selected backend/settings instead of reopening SQLite,
  disabling duplicate automatic child retention. File-backed native memory config
  refresh now runs at actual sync/async idle turn boundaries, resets recalled
  context/owners on change, and respects explicit off. Externally injected owners,
  inherited child owners and evaluation opt-outs retain their host contract.
  Selected agent directories propagate to initial settings, backend paths and
  redaction, including sync CLI. Failed managed refresh disables stale memory
  and remains retryable on a later turn; fixed warnings contain no secret errors.

Exact remaining conversion work: LSP defaults/discovery/deferred diagnostics and
broader editor-service consumers; directory/recursive/binary resource operations
and replacing existing rename targets need shared writer contracts first (current
preflight rejects them). Native UI still needs broader slash/plugin choices,
advanced overlays/themes/images/math and legacy SQLite history import/FTS.
Memory still needs native local/smol/vector providers, Hindsight, legacy bank/
cursor recovery and full transition/extraction/export compatibility. Do not add
alternate parsers/backends/writers. Continue these connected workflows from the
current main tree; no new branches and no tests during the conversion sweep.

The interrupted native input/memory lifecycle batch is now connected in this
checkout. No branches, worktrees, commits or pushes were created during this
shared-checkout continuation. Historical source commits below remain unchanged.

Written connections:
- Native Unix reader -> fixed 4 KiB reads -> existing InputBuffer strict policy
  -> shared key parser -> bounded editor/history. Strict paste checks occur before
  payload extension (up to five bytes of closing-marker overhead); overflow or
  torn-paste timeout closes input without delivering a prefix or trailing Enter.
  Input closure uses existing denial/cancellation/terminal restoration cleanup.
  Windows retains its fixed-record backend; legacy/N-API defaults remain intact.
- Both session hosts -> transient recalled context -> settings-controlled durable
  user/assistant retention. Recalled blocks/tool output/provisional deltas are not
  retained; acknowledged prefix digests prevent duplicate reopen retention.
  Existing redactor handles retained text; failures emit fixed-label warnings and
  preserve completed answers. Native v1/v2 RPC and TUI consume warning events.
- Sync CLI calls AgentSession::close even after submit failure; async shutdown
  flushes below-cadence retained text. Rust SDK hosts must call close explicitly.
  Benchmark sessions explicitly opt out of automatic lifecycle, preserving tools
  and avoiding synthetic evaluation retention; external storage alone does not
  disable normal session behavior.
- Source inventory recognizes only exact live registered nested-worktree directory
  tokens with matching Git common-dir/top-level identity, reports exclusions and
  keeps ordinary/sibling/tracked descendant sources. Unrelated/stale replacement
  repositories fail closed. Playbook status/next now operate on this checkout.
- Focused test runner strips nextest -E/--filter-expr selectors before its Cargo
  doctest pass. This fixes a real post-nextest failure found during this batch.

Executed on the current checkout:
- cargo check --locked -p harvest-omp -p harvest-tui -p pi-natives --all-targets:
  PASS after adding the warning event to benchmark event recording. Inherited
  runtime/addon warnings remain; no product cutover follows from compilation.
- cargo xtask test -p xtask --test playbook: 12 tests PASS plus doctest pass.
- cargo xtask test -p xtask --lib testing::tests: 5 tests PASS plus doctest pass.
- cargo xtask test -p harvest-tui -p harvest-omp --lib -E
  'package(harvest-tui) | test(memory::lifecycle_tests) | test(interactive::input::tests)':
  114 selected tests PASS, 99 unselected; 4 TUI doctests PASS. The first invocation
  passed nextest then failed its doctest flag parsing; the runner fix above and
  final invocation pass both stages. These are bounded workflow regressions,
  not complete application/provider/installed/platform parity.

Logs are under ignored target/migration/shared-continuation-2026-11-04-*;
compiler/test output is not a final source/capability parity receipt. All legacy
production consumers remain. No verified source/capability/removal states advance.
Native-addon rebuild/TS consumer checks, Clippy, WASM/Windows/installed/platform
and broader behavioral consolidation are pending under conversion-first scheduling.

Exact next connections: continue existing native editing owners through rich read
rendering/LSP/ACP/cache/repair and advanced editor/plugin UI consumers. Memory still
needs cancellable provider/vector hooks, local/Hindsight and full extraction/graph/
export parity, legacy bank/cursor rescue, backend transitions and subagent lifecycle
inheritance. Do not create a second backend/registry/input parser. SDK close is
explicit; dropping a sync session intentionally performs no retention I/O. Keep
partial manifest states and the original baseline while conversion continues.

## Previous integrated checkpoint (2026-11-03)

The user requested continuation in Claude’s parallel-worktree style, with shared
documentation maintained for a later Claude handoff. Integration branch:
`codex/continue-rust-migration`, based on main `485145a`. Do not restart from the
older paused `ts-to-rust` tip; main already contains that branch. No publication
or default-launcher cutover is part of this batch.

The three inherited Claude domains are now committed and integrated:
- Native memory engine: source `a937c92`, integrated `bbfef35`.
- Native TUI/editor/interactive CLI: source `035d30d`, integrated `d00774f`.
- Managed/session/memory tools: source `b40abc1`, integrated `f9d9ab3`.
Their original worktrees/branches remain available and clean. Resume from this
integration branch rather than repeating those inherited changes. The coordinator
resolved the common command registration conflict while retaining main’s latest
bench/complete/tiny-models commands. Source baseline is unchanged; no TS deletions.

Written connections: native chat/default Rust CLI -> existing SessionEngine,
streamed/queued turns, cancellation/resume, bounded editor/transcript and paged
full final-call approval; native memory engine -> settings-gated memory tools and
memory:// reads through sync/async hosts; jailed managed artifacts -> real task
preset discovery; persisted session list/inspect/rename -> host storage/workspace.
Custom preset policies not yet enforceable are rejected, never silently ignored.
New engine added to Cargo lockfile, Bazel graph and Rust CI package lists.

Verification: aggregate `cargo check -p harvest-mnemopi -p harvest-tui -p
harvest-omp --all-targets` PASSED after fixing existing core-tool test assertions
for the structured ToolOutput return. Only inherited harvest-runtime dead-code
warning remains. Compiler log: ignored
`target/migration/codex-continuation-2026-11-03-check.log` (compiler output only,
not a parity receipt). Engine/TUI cargo fmt, changed registration/test-helper rustfmt
with skip_children=true, and diff hygiene pass. Recursive formatting of command
modules exposes preexisting formatting debt; unrelated files were not rewritten.
New storage/TUI/persistence tests are written, NOT EXECUTED. No Clippy, addon,
Bazel, WASM, behavior, installed or platform verification is claimed. Conversion-
first scheduling remains active; partial capability/source states are explicit.

Source-task ownership assigns the 14 previously unowned sources to existing
catalog/tooling tasks without verification advancement. Existing migration and
capability inventory commands pass. At that checkpoint, playbook status/next passed ownership
validation but failed with `expected a regular file`: Git listed the nested
.claude/worktrees repositories as directory entries. This source-inventory issue is resolved by the newer shared-checkout batch above.

Exact next actions: continue the pending native UI/editor connections below and
memory host lifecycle/provider wiring using these owners. Consolidation should run
`cargo xtask test -p harvest-mnemopi -p harvest-tui -p harvest-omp`, strict Clippy,
addon-consumer and native/WASM checks, then installed/platform workflows. Keep
known baseline failures listed below distinct from new failures. Memory still
needs full extraction/category/export/vector/graph parity, automatic learning,
provider cancellation and legacy bank recovery; local/Hindsight adapters remain
pending. Managed evaluation/architect execution, skill refresh and live-session
create/send/stop registry remain pending. TUI still needs advanced editing,
completion/history, Markdown/images/themes/overlays, plugin choices and full slash
commands. Crossterm buffers paste internally before the editor cap; parser-level
paste preallocation bounds must use the existing bounded input owner. All legacy
production consumers/default launchers remain until final acceptance evidence.

## Merged into main (2026-11-03)

The user re-authorized conversion and approved merging ts-to-rust into main.
Working mode: convert straight from the TS code in large batches; run
integration tests and fix failures in one pass at the end.

Integration shape: the branch's `ToolRuntime`, session engine, edit modes,
providers (OpenAI + streaming Anthropic), MCP, plugins, LSP/DAP and RPC are the
core. Main's richer tools live in `core_tools.rs`/`core_specs.rs` and are
routed from `ToolRuntime`: write, grep, bash, todo, ast_grep, ast_edit,
web_search, think, checkpoint, rewind, plus `read` in the default (replace)
edit mode (archives, SQLite, directories, summaries). Catalog routing (`catalog.rs`) runs inside
`configuration::load`: an unpinned `provider/id` selector sets base URL, wire
API (OpenAI or Anthropic Messages), provider key env var and output cap.

Known failing tests after the merge (fix in the final pass): provider_diagnostics
credential redaction and rpc_cli real-provider stream (both failing on the branch
before the merge), chat tool-name list, core_tools hashline round trip (main's
hashline edit was replaced by the branch's edit modes), session_engine queued
turns, agent_session rewind (the rewind loop handling in main's agent_session
was not carried over).

## Rules and ownership

Read AGENTS.md, PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative source/
capability manifests. Coordinator application:conversion-first; reviewed leaf
ownership is in the common card. One writer; inspect actual run/process state.
Existing hourly continuation does not prove execution and must not overlap runs.
Work in whole connected conversion domains; compiler repair and broad behavioral/
installed/platform stabilization are deferred to their recorded consolidated passes. Preserve
unverified/partial states. Save coherent checkpoints on the active integration branch. No publication or
default-launcher cutover. The older ts-to-rust-only instructions below are historical.

Latest scheduling instruction: follow PLAYBOOK.md's full conversion sweep, then
integration repair, then workflow/platform verification. Older per-helper compiler/
test scheduling is superseded. Next implementation batch is native editing end to
end (session edit state → read provenance → mode/schema → sync/async/RPC/SDK →
benchmark guidance and eligible caller deletion). Checklist/ownership is in the
common register. Continue that unfinished domain across runs; do not stop after
another small primitive or ask for continue. Uncompiled/unverified checkpoints
are allowed and must record checks not run. No stubs, unsafe bypasses or premature
source/default-launcher removal. Preserve all final acceptance requirements.

## Latest read-domain continuation (base 7a5257c)

Native selector schema/path parsing now connects sorted/merged disjoint ranges,
open/count/dot aliases, tail and raw compounds to the same bounded read owner.
JailedFs literal-entry probing prevents colon filenames, inaccessible entries or
dangling symlinks from redirecting to another source. Raw reads and omitted gaps
grant no numbered edit authority. Metadata retains selection and actual shown rows.
Three new workflow scenarios written; compiler/tests/installed/platform execution
NOT RUN. No TS deletions or verified advances. Full contract/resume is in the common
register/card; next stays on rich-read/native rendering and editor-service consumers.

## Latest native editing domain conversion (base ea95e01)

Whole native read → snapshot/seen rows → mode schema/config → approved shared
engine/writer → sync/async/RPC/SDK/resume → benchmark guidance/evidence graph is
written and connected. Current code is **uncompiled/unexecuted** by user-directed
conversion scheduling. Rustfmt syntax/formatting and control-record/diff hygiene
only; no new receipts, parity, installed evidence or verified removal advances.
Zero TS files deleted: live production app/SDK/addon/metaharness still need them.

Read the common current checklist and updated HASHLINE-HOST-BRIDGE.md first.
Session EditStore is bounded/volatile/workspace-bound; changes of mode clear it,
direct writes invalidate it, reopened logs require fresh reads. Shared strict
policy is opt-in and preserves addon defaults. It rejects unknown/empty provenance,
stale tags, unseen resolved blocks and removed rows from boundary repair; move/
delete require whole-source reads. All five native modes use existing engines.
Final approval includes original call identity and full args plus host mode.

Native CLI run/RPC/config and benchmark select mode; Rust SDK exposes the same
ToolRuntime/AgentSession/SessionEngineOptions. Benchmark guidance is explicitly
oracle-assisted rust-hashline-guidance-v1, regenerated per attempt and recorded
with skipped reasons; it grants no read authority. Single-file metadata shape is
retained; multi-file activity aggregates once per call and separately per file.

Next resume continues remaining editing-domain consumers: rich read selectors/
block-context surfaces and streaming preview/native rendering, LSP/ACP/cache/
auto-repair contracts and live legacy callers. Core native mode/read/state/guidance
code already exists. Do not create another engine/store/registry or stop after a
helper. Formal leaf remains planned/reviewed until prerequisite verification;
active coordinator owns conversion. Preserve user scheduling and final gates.

## Current written connections

Native edit-writer continuation base 2538615: application:edit-write-contract is
reviewed/in progress. Complete-stage preflight now precedes callbacks; native
exact replace and explicit Rust SDK multi-file modes share one jailed/resource-
owned/preimage-checked writer, existing engines/store and final-argument approval.
Create/update/delete/move commits are sequential; earlier bytes remain on later
failure and committed-path/partial-move diagnostics are explicit. No all-file
transaction/CAS/crash-recovery promise. Read EDIT-WRITE-CONTRACT.md for conservative
collision/alias limits, exact bounds, recovery and remaining consumers.
Eight new contracts written/compiled, not executed; compiler/format/ownership only.
No TS deletions, verified counts or addon/installed/platform parity advances.
Receipts/logs: target/migration/conversion-edit-writer-*.

Edit-source continuation base fd657c6: application:edit-file-source is reviewed/
in progress. Session owns an injected Send+Sync FileSource with matching policy,
preserves Session::new/FileCache, and shares one fresh all-path stage across apply
and native exact replacement. Native source/writer retain one jailed capability;
final-argument approval, bounded metadata and encoding/notebook rules remain.
Read tasks/EDIT-FILE-SOURCE.md. Four contracts written/compiled, not executed;
aggregate pi-edit/application compiler/format/playbook only. Addon, behavioral,
installed/platform/removal verification pending. Zero TS deletion or verified count
advance. Receipts/logs: target/migration/conversion-edit-source-*.

Multi-target publication is now written as above; its consolidated evidence is
pending. Hashline still needs normal read provenance, mode/configuration and
guidance connections. Public source-only stage commits no persisted snapshot/
clipboard; the full native writer uses shared session application instead.

Preserve other written owners: calibration/quota planning, exact-hunk shape and
source-bound region constraints, optional target-content SHA-pair dedup, immutable
batch source snapshot/writer/archive/run; native trace report CLI/SDK through
HTTP/offline/selected session branch; cleanup discovery with the prior actual
492-line TS scanner deletion. Details: RUST-BENCHMARK.md, reviewed trace/cleanup
cards and common register. The last two batches have compiler-only evidence;
cleanup alone has recorded focused historical-oracle/copied Linux evidence.
Default product still launches TS; do not confuse native connections with parity.

## Next connections and remaining consumers

- Consolidate the written native writer; connect shared read snapshots/seen-line display,
  mode configuration and final multi-target approval before hashline guidance.
- Benchmark specialized prompts, full syntax eligibility/source acquisition and
  metaharness run/report/queue/native views remain. Keep generate.ts, its legacy
  formatter and shared fixture/hunk consumers; native quota/dedup is not parity.
- Native trace reporter retains trace-report.ts/legacy-auth alias for stored/OAuth
  users. application:legacy-provider-credentials needs shared storage/discovery/
  refresh review; never add a per-report credential store or silently invoke JS.
- First-provider-event/idle deadlines need transport notification contracts;
  local Started is not remote activity. OAuth/accounts/models/KDL parity remains.
- Full settings/schema/discovery/migration/conflict/quarantine/persistence/UI,
  native plugin TUI choices/durable reconnect and broader pending register remain.

## Gates and continuation

Generate honest counts via cargo xtask playbook status --json. Keep original
source baseline/evidence. 35 inherited removals lack evidence; removal gate first
rejects crates/pi-natives/tools/bench-natives.ts. New actual removals also stay
pending source verification. Legacy session/runtime failures reproduce 2ff0193;
whole-addon Clippy has 14 existing diagnostics. Release binary exceeds unchanged
20 MiB budget; eight-platform installation/release evidence is pending. Preserve
these gates/failures. Use cargo xtask test, not direct cargo test, in consolidation.
Disk is tight; reclaim only ordinary unused caches, never sources/history/receipts.
