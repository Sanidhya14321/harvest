# Native editing domain: read, mode, session and guidance

ID application:hashline-host-bridge. **Native core contract reviewed; code written
under application:conversion-first, uncompiled/unexecuted.** Formal leaf remains
planned/reviewed until edit-file-source/edit-write-contract verification dependencies
are done. This current section supersedes the historical planning below. Preserve
all prerequisite/task gates; no done or parity inference from written code.

## Inputs and selected contract

Base ea95e01; source ownership remains migrate:application. Read current
rust-agent/src/{edit,tools,chat,configuration,agent_session,session_engine,cli}.rs,
edit/{read,writer}.rs, edit_benchmark/{runner,guidance,shapes,evidence}.rs, shared
pi-edit session/engine/modes/hashline/{patcher,input,apply,format}, files/store and
historical addon constructor. Legacy review: packages/coding-agent/src/tools/
read.ts and read-format.ts, edit/index.ts/store.ts/hashline plus
packages/metaharness/adapters/edit/runner.ts buildGuidedHashlinePatch/Context.

Native mode is host config (replace/patch/apply_patch/hashline/sloppy), never a
model-supplied switch. Default replace retains path/old/new and prior text/metadata
shape; native aliases translate after approving the exact original model fields.
Final approval receives original call identity with complete {mode,arguments}.
All modes reuse Session, WorkspaceFiles, NativeWriter, jailed IO and locks.
No new executor, snapshot engine, parser, registry, resolver or credential owner.

One bounded EditStore is retained by each ToolRuntime. The first native read/edit/
write binds a canonical workspace; reuse with another workspace fails. Repeated
turns/follow-ups retain state; a mode change resets it. Direct write invalidates
its target history. Reopened session logs grant no authority: state/clipboard/noop
are volatile and host config selects mode again. Do not reconstruct provenance
from untrusted/replayed text or persist unlimited file content into session logs.

Native hashline read uses existing FileRead for BOM/CRLF/notebook normalization,
canonical keys and hashline formatting. Positive one-based offset/limit select
windows; output <=20,000 bytes with complete rows and continuation notices. Actual
emitted indices are recorded directly (never parsed from user file text). Oversized
rows are not shown or marked seen. DOCX extraction grants no underlying edit
provenance; notebook editable cells use the shared projection. Ordinary unwindowed
non-notebook replace reads preserve prior raw bytes/truncation behavior.

IMPORTANT source finding: historical enforce_seen_lines accepts missing/None/empty
snapshot provenance and may promote diagnostic excerpts. Native sessions now call
Session::require_read_provenance; shared HashlineEngine/patcher opt in to strict
validation without altering default addon policy. Strict native mode requires an
exact live snapshot and shown-line set, checks resolved block edits and final
removed source lines from boundary repair, refuses stale-tag recovery, and requires
whole-source reads for REM/MV. Diagnostic excerpts never grant native authority.
BOF/EOF operations require the corresponding boundary line to have been read.
Unknown/evicted/empty/stale provenance means read again, not implicit authority.

Sequential writer semantics remain: earlier committed files can survive later
failure; NativeEditFailure's committed path/op/unknown-progress data is persisted
by the same sync/async/RPC metadata owners. Reserve 4KiB of metadata envelope space
in preflight before writes. No all-file transaction/CAS/crash-journal claim.

## Connected consumers and guidance

ToolRuntime string/detailed/event invocations share native read/edit/write state;
its schemas follow host mode. AgentSession configure_edit (inactive only) and
SessionEngineOptions edit_mode connect sync/async SDK owners and existing RPC.
Native run/RPC accept --edit-mode; native config/profiles accept edit_mode, selected
flag/OMP_EDIT_MODE overrides profile. Config inspection reports mode. Old stateless
free functions stay replace-only; SDK read/edit workflows use ToolRuntime.

Native benchmark RunConfiguration/CLI uses the same session mode. --guided is
explicit hashline-only oracle-assisted rust-hashline-guidance-v1, separately
recorded in run-plan.json. Recompute from current workspace every attempt/retry.
Reuse native bounded pi-diff placements, hashline formatting/parser/applicator;
only advertise patches that reproduce expected editable text exactly. <=25ops,
16KiB body,20,000B JSON; <=256KiB sources and existing hunk work budget. Unsupported
final-newline transformations/path syntax/targets/budgets become recorded skip
reasons in guidance-NNNN.json. Guidance itself is never a read. Static Handlebars
prompt instructs actual read windows first; no seen-line bypass. No historical
score equivalence. Multi-file successful tool metadata now supplies one aggregate
per-call shape sample and separate per-file activity groups; single-file metadata
and existing noop/error evidence contracts remain available.

## Remaining conversion and exact legacy consumers

- Native rich reads: selectors, block-context/elision, archives, PDF/image, SQLite,
  URLs/internal artifacts, SSH, directory/profile views and renderer integration.
  Retain tools/read.ts and read-format/read-selector/read-archive/read-path-resolution/
  read-renderer dependencies used by the production SDK/tool registry.
- Streaming argument previews/native TUI rendering and legacy edit write bridges,
  LSP/ACP synchronization, cache invalidation and auto-repair consumers. Review
  their real shared owners; do not fake diagnostics or create another writer.
- Production packages/coding-agent SDK/session/tool registry and native addon
  EditSession still consume edit/index.ts and modes/hashline/store. The TS
  metaharness runner still consumes buildGuidedContext and legacy task/tool APIs;
  the native benchmark is connected, but full metaharness cutover is unfinished.
- No TS deletion eligible in this domain batch. No orphan dependency removed.
  Standard launchers remain TS until final candidate-installed acceptance.

Continue these connections as the same domain, not another tiny prerequisite run.
Missing unresolved contracts become bounded prerequisites in the common register.

## Integration and acceptance (deferred, NOT run)

Only Rustfmt syntax formatting, control-record ownership and diff hygiene belong
to this uncompiled checkpoint. No source-bound behavioral receipt. Current code
and new tests have NOT been compiled; older source/writer compiler receipts do not
certify this tree. At consolidated integration run:

- cargo check --locked -p pi-edit -p harvest-omp --all-targets --all-features
- cargo fmt -p pi-edit -p harvest-omp -- --check
- cargo clippy --locked -p pi-edit -p harvest-omp --all-targets --all-features -- -D warnings
- cargo xtask test -p harvest-omp --test native_edit_sessions
- cargo xtask test -p harvest-omp --lib edit_benchmark::guidance
- cargo xtask test -p harvest-omp --test native_edit_writer
- cargo xtask test -p harvest-omp --test native_edit
- cargo xtask test -p harvest-omp --test agent_session
- cargo xtask test -p harvest-omp --test session_engine
- cargo xtask test -p harvest-omp --test rpc
- cargo xtask test -p pi-edit
- Rebuild addon once for shared engine changes, then its explicit legacy suites;
  retain original no-strict policy contracts. Include benchmark shape/evidence,
  caller/config precedence and guided first/retry/no-guidance workflows.

New scenarios cover displayed-only windows, oversized/forged rows, state isolation/
root mismatch/mode reset/direct-write invalidation, denial, actual sync/async model
read/edit and durable error/metadata, resume with known tag but no authority,
profile/host/invalid mode precedence, guided repair requiring real read and retry
skip. They are written, not executed. Consolidate stale/eviction/alias/clipboard/
noop/blocks/repair/move/delete/multi-file partial/final args/cancellation plus native
copied installed CLI/RPC/external SDK and all promised platforms. Existing source/
capability/installation gates remain mandatory. Recovery reverts this coherent
conversion to ea95e01; preserve legacy sources and user/other-agent commits.

---

## Historical discovery and prerequisite notes

### Original native hashline host and guided benchmark prerequisite

ID `application:hashline-host-bridge`; planned, review pending. Capabilities:
`tool:read`, `tool:edit`, `surface:benchmarks`. Prerequisites: `control:playbook`,
`review:application`. Execution ownership is currently reserved by
`application:conversion-first`; do not start another writer in its shared files.

Discovery: metaharness buildGuidedContext is enabled only for hashline and emits
hash-tagged line operations. Native tools currently advertise path/old/new and
stage ReplaceEngine with a fresh EditStore. The shared pi-edit HashlineEngine,
parser, store, path policy and transactional host session already exist. Porting
prompt text alone cannot connect this retained behavior.

Inputs: packages/metaharness/adapters/edit/runner.ts buildGuidedHashlinePatch and
buildGuidedContext; packages/coding-agent/src/edit/hashline and tool/read paths;
crates/pi-edit/src/{session,store,files,path_policy}.rs and modes/hashline;
rust-agent/src/{edit,tools,chat,agent_session,session_engine}.rs and native benchmark.
The parent source ledger retains ownership for these TS sources; this card is a
bounded prerequisite, not a full source audit or a second engine.

Review before ready: select the native edit mode configuration/wire schema and
read display/snapshot contract; identify every sync/async/RPC/SDK tool consumer;
review the existing pi-edit Session/EditWriter multi-file transaction and final
argument approval boundary. Reuse those owners, not another hashline parser or
snapshot registry. Define stale/unknown tags, seen-line enforcement, path aliases,
clipboard transactions, no-op guard, cancellation, competing writes and rollback.
Do not disable seen-line protection merely to make generated guidance work.

Implementation sequence after review: retain exact-replace callers; configure
one bounded EditStore per native session; record read provenance through existing
read output; route hashline staging through shared engine/host writer with every
final affected path authorized and jailed; connect all native session consumers;
then generate retained guided patches via shared line diff/hashline formatting,
with bounded operations/text and explicit skipped-guidance reasons. Refresh from
current workspace each attempt. Store version/policy in run-plan; guidance is
explicit opt-in and must not change historical scores. Reinspect legacy complexity
guards rather than copying apparent bugs into a parity claim.

Deletions: none until last legacy consumer moves and its evidence exists. Keep
legacy edit/read/guidance sources and shared types. Rollback: revert only this
future coherent batch; existing native replace/default TS paths stay available.

Conversion verification: cargo fmt -p harvest-omp -- --check;
cargo check --locked -p harvest-omp --all-targets --all-features;
existing xtask playbook check. During consolidated stabilization run
cargo xtask test -p harvest-omp and copied installed native fixtures for stale
snapshots, final-path approval, multi-file failure/recovery, concurrent/aliased
paths, line provenance/noop/clipboard, streaming/commit/cancel ordering, guided
first/retry attempts and normal replace consumers. Completion requires those
behavioral/installed contracts and the documented remaining TS consumers, never
compilation alone. No Rust code or parity is claimed by creating this card.

## Concrete source/writer prerequisite (c99df19 review)

application:edit-file-source now names the missing jailed FileSource seam for
Session::new and the actual sequential writer semantics. Read that card before
making this task ready. Existing Session does not itself provide all-file rollback;
the earlier reference to a transactional host was a requirement to review, not an
implemented property. Keep native replace and legacy hashline consumers working;
no seen-line bypass, ambient FileCache shortcut or invented multi-file guarantee.

Source boundary continuation base fd657c6: application:edit-file-source is now
reviewed/in progress, with injectable source and native replace staging written.
Behavioral/addon/installed evidence remains pending. application:edit-write-contract
separately owns multi-target publication/failure review. Hashline still stays
planned/review pending; neither prerequisite establishes seen-line/mode/guidance
connections or a multi-file transaction. Read the updated cards before proceeding.

Writer continuation base 2538615: the write contract is now reviewed/in progress
and shared native publication is connected, including explicit SDK modes with
enforce_seen_lines=true. Normal read provenance/mode/guidance consumers remain
unconnected; no tool-schema/default-mode switch or parity follows from this SDK.
Read the updated writer card and pending consolidation checks before ready.

Retained state bounds: snapshot versions/paths/text use EditStore's existing
limits. No-op path counters now obey the same maximum path count (deterministic
key eviction; a zero-path store retains none). Native writer preflight admits at
most 256 named clipboard registers, 256 bytes per register name and 16Mi UTF-16
units across retained names/rows, before any publication. Anonymous clipboard
state remains batch-local. Consolidate retention/eviction/clipboard bounds with
the workflow checks; these conversion changes are uncompiled/unexecuted.
