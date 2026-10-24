# Native hashline host and guided benchmark prerequisite

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
