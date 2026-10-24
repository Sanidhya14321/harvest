# Native multi-file edit publication review

ID application:edit-write-contract; planned, review pending. Prerequisites
control:playbook, review:application, review:primitives. Capabilities tool:edit and
surface:benchmarks. Current migration writer owns this review; no second writer.

Concrete blocker to the native hashline host: pi-edit Session::apply validates
all stages, then calls EditWriter sequentially. It intentionally returns on a
writer error and leaves earlier persisted files written. Session::stage now lets
the native host perform its own publication but provides no transaction or
snapshot/clipboard commit after a host-only stage. Current native single exact
replacement uses that stage and one atomic jailed write; do not advertise other
modes as connected or assume all-file rollback exists.

Inputs: shared pi-edit session/store/staged metadata/writer contracts, native
edit.rs/tools.rs/approval/session engines, legacy edit/index.ts and runner/host
write paths, pi-natives EditSession/TsfnWriter. Review actual retained partial
failure behavior first; do not invent a stronger transaction contract merely
because individual writes are atomic. Define the final-argument/multi-target
approval order, read/preimage freshness, alias/lock ownership, output limits,
rename/delete/create collisions, no-op/clipboard/snapshot outcomes and bounded
failure/cancellation/crash reporting. If retained behavior needs a journal or
rollback owner, add the bounded dependency after evidence; never improvise one
in the model-facing tool. Reuse the existing jailed filesystem and pi-edit owner.

Before ready, choose the host write/publication interface and all sync/async/RPC/
SDK/benchmark consumers, exact recovery semantics and behavioral/installed tests.
Then connect it along with shared read provenance and mode configuration in
HASHLINE-HOST-BRIDGE.md. Keep current replace callers working, maintain missing
TS consumers, and do not claim parity from the source constructor alone.

Verification when reviewed: cargo xtask test -p pi-edit and -p harvest-omp with
named writer/failure/snapshot/approval selectors; affected addon suites; copied
installed final-path authorization, symlink/alias/lock/rename/delete collision,
partial write/cancellation/recovery, no-op/clipboard and terminal/RPC error checks.
No implementation or tests have executed for this planned review. Source removal
is zero; recovery preserves current native replace and legacy mode consumers.
