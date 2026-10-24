# Current Rust migration handoff

Continue ts-to-rust from its actual remote tip, preserving history/user/other-agent
changes. Immutable baseline d811cb2; imported history through 6eefffc is preserved.
Previous detail: archive/HANDOFF-2026-10-24-before-edit-source.md and earlier archives.

## Rules and ownership

Read AGENTS.md, PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative source/
capability manifests. Coordinator application:conversion-first; reviewed leaf
ownership is in the common card. One writer; inspect actual run/process state.
Existing hourly continuation does not prove execution and must not overlap runs.
Work in connected conversion batches; aggregate compiler/format/ownership checks
at checkpoints, broad behavioral/installed/platform stabilization later. Preserve
unverified/partial states. Commit/push this branch without routine approval or a
continue prompt; no main merge/publication/default-launcher cutover.

## Current written connections

Edit-source continuation base fd657c6: application:edit-file-source is reviewed/
in progress. Session owns an injected Send+Sync FileSource with matching policy,
preserves Session::new/FileCache, and shares one fresh all-path stage across apply
and native exact replacement. Native source/writer retain one jailed capability;
final-argument approval, bounded metadata and encoding/notebook rules remain.
Read tasks/EDIT-FILE-SOURCE.md. Four contracts written/compiled, not executed;
aggregate pi-edit/application compiler/format/playbook only. Addon, behavioral,
installed/platform/removal verification pending. Zero TS deletion or verified count
advance. Receipts/logs: target/migration/conversion-edit-source-*.

application:edit-write-contract separately owns multi-target publication/failure
review. Shared apply stays sequential and stage commits no persisted snapshot/
clipboard. Hashline still needs read provenance, mode/approval and writer
connections; do not expose hashline/guidance or claim an all-file transaction.

Preserve other written owners: calibration/quota planning, exact-hunk shape and
source-bound region constraints, optional target-content SHA-pair dedup, immutable
batch source snapshot/writer/archive/run; native trace report CLI/SDK through
HTTP/offline/selected session branch; cleanup discovery with the prior actual
492-line TS scanner deletion. Details: RUST-BENCHMARK.md, reviewed trace/cleanup
cards and common register. The last two batches have compiler-only evidence;
cleanup alone has recorded focused historical-oracle/copied Linux evidence.
Default product still launches TS; do not confuse native connections with parity.

## Next connections and remaining consumers

- Finish native edit write contract and shared read snapshots/seen-line display,
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
