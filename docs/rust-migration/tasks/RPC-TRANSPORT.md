# services:rpc-transport — native versioned RPC transport batch

Prerequisites: `control:playbook`, `review:services`, `services:rpc-transcript`.
Capability: `surface:rpc-v1-v2`. This completes the native transport boundary,
not the unported application commands or the production TS consumer cutover.

Inputs: legacy `rpc-frame.ts`, its tests and client negotiation/decoding in
`packages/coding-agent/src/modes/rpc`; native RPC, async owner and durable codec.
Baseline: `81de404`; compact inert transport fixtures record actual legacy v2
wire hashes and Git blob provenance in the protocol fixtures README.

Destination/connections: `harvest_protocol::rpc_frame` owns physical/logical
ceilings, lazy v2 chunk emission and strict reassembly. `harvest_omp::rpc::serve`
owns negotiation and one ordered writer. `rpc::reader::Reader` is the public
native SDK reader; its physical input engine also serves the server. Copied CLI,
Rust SDK and remaining TS client decoders exercise the transport. No addon, new
dependency, generator or executable language glue is introduced.

Contract: start at v1; advertise v1/v2, 1 MiB physical frames (newline included)
and 64 MiB logical JSON for v2. Only protocol version 2 may be negotiated; queue
the acknowledgement before changing output encoding. Chunk payloads are 256 KiB
with strict base64, sequential indices and stable id/count/declared length.
Clients enable v2 only after acknowledgement. Reject invalid/interrupted chunks,
invalid UTF-8/JSON/nonobjects, unsafe metadata and truncated process exit.
Framing errors terminate the native client stream; never resume ambiguous data.

Serialize within limits and emit physical chunks lazily without interleaving.
Bound queued logical bytes to 64 MiB as well as 32 queue entries; one current
producer serialization may additionally retain up to 64 MiB. Account for one
physical chunk's encoding overhead. The whole logical write shares the existing
10-second output deadline. Preserve cancellation-safe partial physical/chunk
reads, EOF shutdown, writer ownership, final-argument approval and persistence.
Keep command inputs bounded to one physical line, as in the legacy server.

Steps: capture legacy wire oracles; implement portable framing; connect native
negotiation and writer; share bounded physical reads with the native client;
verify native endpoint/client and cross-language decoder interoperability in
both directions; update scope and evidence once for the finalized batch.

Checks (focused selectors while editing; one full affected integration pass):

```sh
cargo fmt -p harvest-protocol -p harvest-omp -- --check
cargo clippy --locked -p harvest-protocol -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-protocol -p harvest-omp
bun test packages/coding-agent/test/rpc-frame.test.ts packages/coding-agent/test/rpc-messages.test.ts
cargo xtask playbook check
cargo xtask source-audit verify --scope affected --base 81de404 --plan
```

Also exercise a copied CLI with language runtimes absent from PATH and the native
SDK reader, including negotiated large histories and streaming/EOF/reopening.
Store logs and source-bound focused receipts in `target/migration/rpc-transport-*`.
Completion requires real compatibility, bounds, cancellation, queue/transport
failure and installed checks. Unexecuted full transition and existing broad gate
failures remain explicitly separate from batch success.

Deletions: none until the production RPC command/client consumers move. Keep
legacy v1 display shortening and terminal-frame compaction pending; the native
transport preserves full v2 messages and explicitly fails oversized v1 output.
Do not mark whole RPC parity or legacy removal from transport completion.
Recovery: revert this bounded transport batch, preserving all session files.
