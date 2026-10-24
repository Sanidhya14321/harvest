# Jailed edit-session file source prerequisite

ID application:edit-file-source; reviewed, in progress. Prerequisites
control:playbook, review:primitives, review:application. Base fd657c6.
Capabilities tool:read/tool:edit. Sole current writer owns shared pi-edit source/
session and native edit boundaries; do not create another file or staging owner.
Source ledger ownership of legacy edit/index.ts remains migrate:application;
this task implements the source boundary, not every old tool behavior.

## Reviewed contract and implementation

Inputs: crates/pi-edit/src/{session,files,path_policy,engine,store}.rs; native
rust-agent/src/{edit,tools}.rs, pi-natives/src/edit.rs Session::new/Shared/TsfnWriter
and shared tests/common; JailedFs, cancellation and native tool approval owners.
The default Session constructor and its FileCache behavior stay supported for
existing addon consumers. Native host reads must never fall back to ambient
FileCache or suffix recovery when approval covered an exact target.

Written: Session retains Box<dyn FileSource + Send + Sync>; with_file_source owns
that source for the call lifetime, rejects source/config policy mismatch before
any read, and retains existing engine/args/store/preview/apply owners. PathPolicy
now supports structural Eq for this authority check. Send/Sync is compile-checked
for the existing Tokio-mutex bridge; no raw lifetime pointer or unsafe cast.
Default Session::new builds the same FileCache with identical supplied policy.

Public Session::stage extracts the existing final-stage logic: clear source,
require complete arguments, stage through the chosen engine, check every source
and move destination against policy, enforce all write restrictions, return the
actual staged files without a host write. Session::apply calls that same method
then retains its existing sequential writer/store/clipboard/result flow. Calling
stage again rereads the source; preview bytes cannot authorize final writes.
Stage does not commit persisted snapshots/clipboard or create a write transaction.

Native replace_detailed now uses with_file_source → finished canonical argument
JSON → stage, instead of its own direct ReplaceEngine/ArgSnapshot staging. It
requires exactly one stage and retains the actual diff/noop/result metadata and
bounded atomic writer. The host source and writer share Arc<JailedFs> (one pinned
capability); cancellation is retained through the existing cloneable token.
Generated-file guard, text/UTF-8/BOM/CRLF/notebook decoding and exact authored-path
resolution still use the same owners. Native tools::invoke_detailed approval of
final arguments still precedes staging. Every sync/async/RPC/SDK/benchmark native
replace consumer therefore reaches the common stage; no new runtime or adapter.
The native application still advertises exact replacement only, not hashline.

## Consumer/deletion and recovery rules

Keep pi-natives default constructor, native single-file replace interfaces, shared
TS edit/read/hashline/SDK/UI consumers and original immutable baseline. No TS
source removed. Native legacy row stays partial/retained/unverified. No broader
read provenance, cross-call EditStore, hashline guidance or all-mode parity claim.

Separate prerequisite application:edit-write-contract now owns the unresolved
native multi-target publication/failure review. Shared apply is sequential and
leaves earlier successful writes on a later writer error. Review retained legacy
semantics before promising rollback; do not narrow grammar or weaken authorization
silently. HASHLINE-HOST-BRIDGE depends on both tasks and still needs read snapshots,
seen-line rendering, mode configuration and final-target approval connections.

Recovery: revert this coherent checkpoint to fd657c6, restoring concrete FileCache
and direct native replace staging. Preserve user files/records. Neither constructor
nor stage may read/write outside the selected capability to recover from failure.

## Checks and required evidence

Conversion: cargo fmt -p pi-edit -p harvest-omp -- --check;
cargo check --locked -p pi-edit -p harvest-omp --all-targets --all-features;
existing xtask playbook check and affected source-audit --plan. Aggregate compiler/
format/ownership only; receipts prefix target/migration/conversion-edit-source-*.
Four host_source contracts are written/compiled, not executed: injected private
bytes/fresh apply after preview, stage without writes, revoked source with an
existing ambient file, conflicting policy before reads, incomplete args rejection
(the stage/no-write transition is combined with freshness in one test).

Consolidation: cargo xtask test -p pi-edit --test host_source;
cargo xtask test -p harvest-omp --test native_edit; all pi-edit session/mode tests;
affected native addon constructor/stream/flush/write suites and strict Clippy.
Verify native detailed metadata, BOM/CRLF/notebooks, generated and ambiguous files,
read bounds/cancellation, jailed parent/leaf symlink/alias paths, source-policy
mismatch, cache invalidation, all-stage preflight and default-constructor output
compatibility. Run copied installed sync/async/RPC/SDK/benchmark native replacement
and promised platforms; shared source-audit/lifecycle validators still apply.
No addon rebuild, behavioral, installed/platform or source-removal verification is
claimed by the compiler check. Task stays in progress until those checks pass.
