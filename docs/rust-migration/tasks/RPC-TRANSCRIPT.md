# services:rpc-transcript — native committed transcript queries

Prerequisites: `control:playbook`, `review:services` (both done).
Capability: `surface:rpc-v1-v2`, which remains partial. This leaf completes two
native query commands; it owns no legacy removal or full RPC parity claim.

Inputs: `packages/coding-agent/src/modes/rpc/rpc-messages.ts` and its existing
tests; `agent-session.ts::getLastAssistantText`; `rust-agent/src/rpc.rs`, the
shared async owner and durable session codec. The captured pagination oracle is
in `crates/harvest-protocol/tests/fixtures/rpc-pages.json` with Git provenance.
Legacy source ledger ownership stays with `migrate:services`/`migrate:application`.

Destination: ordinary `harvest_protocol::transcript` APIs and the existing native
RPC adapter. CLI and Rust embedders using `rpc::serve` share this implementation.
No new N-API binding, runtime dependency, worker or generator is introduced.

Contract: `get_messages_page` uses one committed owner snapshot and its session,
leaf and message count for legacy-compatible base64url v1 cursors. Default limit
100, maximum 256, target payload 768 KiB. Reject malformed options, stale cursors
(`stale_cursor`) and changing sessions (`session_busy`). Retain opaque original
message payloads, order and oversized individual records; the existing v1 sink
must explicitly reject an oversized frame instead of truncating it. Transport v2
remains follow-up work. Whole JSON numeric values such as 1.0 are accepted.

`get_last_assistant_text` skips only assistants aborted with empty content,
concatenates text blocks, applies ECMAScript trimming and omits the `text` field
when unavailable. Tool-only/blank newer assistants do not expose older text.
No persistence mutation or provider invocation occurs. EOF/cancellation and
writer lease ownership remain with the existing session lifecycle.

Steps:
1. Capture actual legacy pagination output, cursor bytes and errors.
2. Implement bounded cursor parsing and shared lossless query helpers.
3. Connect native RPC, including optional machine-readable error codes.
4. Verify byte-compatible fixture results, large-history reconstruction,
   oversized preservation, invalid cursor/limit handling and copy semantics.
5. Exercise live owner busy/stale behavior, then compiled CLI over JSONL with a
   Rust loopback provider; confirm durable history and clean shutdown.

Verification:

```sh
cargo fmt -p harvest-protocol -p harvest-omp -- --check
cargo clippy --locked -p harvest-protocol -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-protocol -p harvest-omp
cargo xtask playbook check
cargo xtask source-audit verify --scope affected --base 25314f6 --plan
```

Also run the existing Bun `rpc-messages.test.ts` for the unmoved legacy consumer.
Store receipts under `target/migration/rpc-transcript-*`. Completion requires all
affected Rust contracts, compiled CLI checks and strict lint to pass. Broader
source-removal/platform failures remain failed and RPC remains partial. The
shared-input planner selects a full transition; preserve that plan and report it
as unexecuted rather than claiming the focused checks certify a full transition.

Deletion: none in this leaf. Production TS RPC and its SDK clients still consume
the legacy module. Remove it only after their complete protocol and application
consumers move. Recovery: revert the bounded query commit; preserve sessions.

Subsequent transport work is owned by `services:rpc-transport` in
`RPC-TRANSPORT.md`, including the v2 work recorded as pending at this checkpoint.
