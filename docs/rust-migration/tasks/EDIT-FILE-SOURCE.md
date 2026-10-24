# Jailed edit-session file source prerequisite

ID application:edit-file-source; planned, review pending. Prerequisites
control:playbook, review:primitives, review:application. Capabilities tool:read,
tool:edit. Source ledger ownership stays with the existing parent tasks until
review. Same active writer owns native tool/edit and shared pi-edit boundaries.

Concrete discovery at c99df19: native replace staging uses WorkspaceFiles over
JailedFs and FileSource. Shared pi-edit Session::new instead owns a concrete
FileCache whose reads open ambient paths, with missing-path recovery. It cannot
accept the host's existing jailed/cancellable source. Connecting that Session
blindly would bypass the native capability-relative read owner and exact-target
approval contract. Hashline host/guidance must not ship that shortcut.

Inputs: crates/pi-edit/src/{session,files,path_policy,engine,store}.rs;
rust-agent/src/{edit,tools}.rs; pi-natives callers/tests of Session::new; shared
JailedFs and native sync/async tool/approval owners. Before ready, review an
injectable source/cache lifetime/Send/ownership boundary while preserving the
current constructor for existing native addon consumers. Cache clearing,
normalization, actual opened path aliases, cancellation and final-path approval
must use one owner. No second path resolver, parser, snapshot store or writer.

Separate issue discovered in the same review: pi-edit Session::apply stages all
files, then awaits sequential EditWriter::write calls; it does not itself roll
back earlier successful writes after a later failure. Do not call that a whole-call
transaction. Review legacy multi-file failure semantics and the native host writer
contract, with final targets approved/jailed before mutation, lock ownership,
preimages/rollback, rename/delete/collision handling and cancellation/crash recovery.
Record any missing journal/transaction prerequisite explicitly; do not infer it
from atomic single-file writes or silently narrow the advertised hashline grammar.

After review, extend the existing source boundary, connect the native host and
all shared-engine consumers, then implement the hashline card. During conversion
use aggregate checks of affected crates; stabilization must run cargo xtask test
-p pi-edit and -p harvest-omp plus affected addon/read/edit suites and copied
installed path/alias/symlink/partial-write/seen-line/snapshot/cancellation tests.
Exact selectors need review before ready. Delete nothing at this prerequisite.
Recovery restores the prior source constructor and native replace path; preserve
existing user files and historical evidence. This card claims no implementation,
behavioral/installed parity or source removal.
