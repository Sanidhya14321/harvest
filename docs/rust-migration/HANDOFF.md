# Current Rust migration handoff

Continue ts-to-rust from its actual remote tip, preserving history/user/other-agent
changes. Immutable baseline d811cb2; imported history through 6eefffc is preserved.
Previous detail: archive/HANDOFF-2026-10-24-before-edit-source.md and earlier archives.

## Rules and ownership

Read AGENTS.md, PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative source/
capability manifests. Coordinator application:conversion-first; reviewed leaf
ownership is in the common card. One writer; inspect actual run/process state.
Existing hourly continuation does not prove execution and must not overlap runs.
Work in whole connected conversion domains; compiler repair and broad behavioral/
installed/platform stabilization are deferred to their recorded consolidated passes. Preserve
unverified/partial states. Commit/push this branch without routine approval or a
continue prompt; no main merge/publication/default-launcher cutover.

Latest scheduling instruction: follow PLAYBOOK.md's full conversion sweep, then
integration repair, then workflow/platform verification. Older per-helper compiler/
test scheduling is superseded. Next implementation batch is native editing end to
end (session edit state → read provenance → mode/schema → sync/async/RPC/SDK →
benchmark guidance and eligible caller deletion). Checklist/ownership is in the
common register. Continue that unfinished domain across runs; do not stop after
another small primitive or ask for continue. Uncompiled/unverified checkpoints
are allowed and must record checks not run. No stubs, unsafe bypasses or premature
source/default-launcher removal. Preserve all final acceptance requirements.

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
