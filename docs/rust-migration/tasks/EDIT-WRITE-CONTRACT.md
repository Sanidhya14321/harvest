# Native multi-file edit publication

ID application:edit-write-contract; reviewed, in progress. Base 2538615.
Prerequisites control:playbook, review:application, review:primitives. Capabilities
tool:edit/surface:benchmarks. Same current writer owns shared session/native edit
boundaries; no competing transaction, approval, filesystem or store implementation.

## Reviewed retained behavior and selected native contract

Inputs: pi-edit Session/apply/stage/store/WriteRequest/EditWriter and staged files;
native edit.rs/tools/approval, all sync/async/RPC/SDK/benchmark exact-replace
callers; pi-natives EditSession/TsfnWriter; legacy edit/index.ts apply and #write
(lines ~702, 806 onward at base). Legacy host writes each callback sequentially;
move writes destination then deletes source. Earlier successful writes survive
later callback failure. Preserve that explicit partial-write behavior; do not
invent an all-file transaction or rollback guarantee from single-file atomics.
Legacy LSP/bridge/auto-repair/mutation-cache/diagnostics remain separate consumers.

Written: shared Session::apply_with_preflight stages the whole set and invokes a
host hook before any writer callback, retaining its returned guard to call end.
Existing apply delegates with an empty hook and preserves writer/store/clipboard/
result flow. Native host uses its existing injected jailed/cancellable source,
then one NativeWriter (src/edit/writer.rs) for final publication. It uses existing
resource locks and JailedFs, not ambient opens. Nothing widens readable/writable
roots or allows missing-path suffix recovery after final-argument approval.

Public Rust edit::apply_with_approval(root, mode, argument_json, store, policy,
cancellation) provides the explicit SDK path. Approval sees the exact final mode
and entire canonical argument object before staging; native exact-replace tool
keeps its existing preceding approval and calls the same private authorized owner.
The model registry still advertises exact replacement only. SDK modes use shared
engines and enforce_seen_lines=true; caller-supplied EditStore provenance is
required for hashline, never auto-filled to bypass it. Normal read-tool provenance,
per-session mode configuration and benchmark guidance remain unfinished.

## Publication and failure boundaries

Static shared-engine inspection rejects >32 declared targets/intents or >4096-byte
paths/128 KiB aggregate path labels before file reads. Native source reads are
bounded to 256 operations/128 MiB admitted bytes per call; one overflowing read
(up to 4 MiB) fails and permanently exhausts that call. Existence probes use
jailed metadata.
Native preflight accepts 1..32 stages, <=4 MiB per persisted file, <=16 MiB total
raw-before/persisted bytes, <=256 KiB aggregate selected metadata. Actual diff
previews are capped at 32 KiB per file with an explicit unavailable reason.
Final arguments <=1 MiB. Validate all source/destination paths, reject physical
alias/overlapping targets, acquire all locks in sorted order, then compare every
preimage to actual bytes and require move destinations absent before any write.
Noop stages still have freshness/ownership checks; they have no writer callback.

Each callback must exactly match its prepared path/op/destination/content. Recheck
path resolution and preimage just before its operation, retain resolved paths for
mutation, and check cancellation. Create/move destination is atomic create-only;
update atomically replaces; delete uses jailed remove. Moves publish destination
then clean the source. Do not cancel between these two steps; cleanup failure is
an explicit partial move. Revalidate destination absence/identity; never overwrite
an occupied destination to make a mode pass. Native collision/alias restrictions
are conservative and require retained-mode compatibility review before cutover.

NativeEditFailure exposes committed path/operation records (including a landed move
destination); Display names the committed count and directs reread. Poisoned state
marks progress unavailable rather than zero. Shared Session commits its snapshot/
clipboard after each successful callback, retaining existing sequential semantics.
Earlier landed bytes and records survive later failures. Advisory locks coordinate
participating owners; freshness checks are not a filesystem CAS or a crash journal.
No rollback, crash-resume, power-loss or hostile noncooperating-writer proof is claimed.

NativeWriter callbacks perform synchronous jailed I/O and never suspend. The sync
host uses the existing Futures now_or_never adapter, creating no nested executor,
Tokio runtime or thread. Unexpected yielding becomes a controlled edit failure with
recorded progress. Existing workspace async-trait/Futures dependencies are reused;
Cargo.lock gains only those direct dependency edges, no new package versions.

## Connections, deletions and remaining work

Native replace_detailed now runs the shared full session application, retaining
actual staged metadata and its edited/unchanged output. Thus all existing native
sync/async/RPC/SDK/benchmark exact-replace consumers reach the writer. Explicit SDK
multi-file ApplyPatch/other engine paths use the same owner; no separate engine.
Persisted formats stay workspace bytes (BOM/line ending/notebook restoration) and
existing session/tool records. Committed-change errors are Rust SDK diagnostics;
full structured partial-result RPC/UI integration still needs consolidation/review.

No TS source removed. Keep edit/index.ts, read/hashline/SDK/UI/LSP/ACP/bridge callers
until complete consumers move; implementation stays partial/unverified. Host mode
configuration, read snapshots/seen-line rendering, all-mode bridge/repair/cache
behavior and guided benchmark consumers are still pending in HASHLINE-HOST-BRIDGE.
Recovery reverts this coherent checkpoint to 2538615; never erase user data or
try automatic rollback of an already failed sequential operation.

## Evidence and stabilization

Conversion checks: cargo fmt -p pi-edit -p harvest-omp -- --check;
cargo check --locked -p pi-edit -p harvest-omp --all-targets --all-features;
existing xtask playbook check, capabilities and source-audit --plan. Source hashes/
receipts/logs prefix target/migration/conversion-edit-writer-*. Compiler/ownership
only, no behavioral/addon/installed/platform/source-removal gate is certified.

Eight new contract scenarios are written/compiled, not executed: six SDK cases
in native_edit_writer.rs; one private writer external-change/committed-prefix
case; shared host_source preflight rejection before callback. They cover actual
create/update/delete/move, exact approval, denial/locks, seen-line rejection and
new snapshots, nested executor safety, occupied destination and stale later target.

Consolidation commands: cargo xtask test -p harvest-omp --test native_edit_writer;
cargo xtask test -p harvest-omp --test native_edit;
cargo xtask test -p harvest-omp --lib edit::writer;
cargo xtask test -p pi-edit --test host_source; all pi-edit session/mode contracts;
affected addon tests and strict Clippy. Use copied installed native sync/async/RPC/
SDK/benchmark edits for full approvals, concurrent/aliased/symlink paths, noops,
BOM/CRLF/notebooks, moves/collision/cleanup failure, cancellation between writes,
writer failure/partial status/store/clipboard, metadata bounds and target changes.
Run all promised platform/artifact and shared source/lifecycle validators before
claiming completion. Credential/platform/machine limits remain honest blockers.
