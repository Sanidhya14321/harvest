# catalog-providers:anthropic-text-tools — native Messages application boundary

Prerequisites: `control:playbook`, `review:catalog-providers`,
`services:rpc-transport` (done). Capability: `surface:streaming-providers`.
This bounded foundation is independent of completion of catalog/KDL and credential
migration. It does not complete the broader provider migration task.

Inputs: `packages/ai/src/providers/anthropic.ts`, `anthropic-wire.ts`, native
provider/transport, chat/tool structures, usage, CLI and session lifecycle.
Baseline: `3e38027`. Synthetic inert native wire fixtures are identified in
`rust-agent/tests/fixtures/anthropic/README.md`; no live API credentials are used.

Destination/connections: `rust-agent/src/anthropic.rs` implements text/tool request
conversion and bounded JSON/SSE decoding. `sse.rs` shares framing with OpenAI.
`HttpModel::anthropic_messages(max_tokens)` selects the ordinary Rust SDK API;
`harvest run` sync/async and `harvest rpc` select the same path with
`--api anthropic-messages --model ID --base-url URL`. Model and host are explicit
until catalog policy resolution moves. `--max-output-tokens` defaults to 4096;
zero is rejected. Generic OMP API-key configuration still supplies credentials.
No model-name heuristics, embedded executable code or dependency is introduced.

Contract: preserve ordered initial system text, conversation text, assistant tool
IDs/names/object arguments and matched user tool-result blocks. Merge adjacent
same-role turns. Reject orphan/duplicate/unresolved tool exchanges before HTTP.
Use `x-api-key` and version headers; never send bearer authentication for Messages.
Prevent credential forwarding across origins on redirects for both wire APIs.
Preserve existing cancellation, response limits, bounded queues and host callbacks.

SSE requires a valid message start, ordered nonoverlapping content blocks,
compatible deltas, complete object tool input, successful matching stop reason
and message stop. Text deltas are provisional; cancellation, callback failure,
truncation, unsupported content and provider errors cannot commit an assistant.
Keep complete raw usage fields, including numbers beyond normal JSON values;
merge start/delta counters without losing unknown fields. Input, cache-read,
cache-write and output counts are additive; missing/invalid/overflowing required
counts stay unavailable. Pricing stays unavailable until catalog resolution.

Implementation: share bounded SSE framing; generalize existing cancellable
transport headers and constrain redirects; implement Messages conversion/decoder;
wire SDK/CLI/RPC; exercise actual tool execution, installed candidate and durable
replay; update records once and verify the final batch.

Verification (one package integration pass, no unrelated legacy rebuild/suites):

```sh
cargo fmt -p harvest-omp -- --check
cargo clippy --locked -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-omp
cargo xtask playbook check
cargo xtask source-audit verify --scope affected --base 3e38027 --plan
cargo xtask migration
cargo xtask capabilities
```

The package suite includes copied native CLI with language runtimes absent from
PATH, sync JSON/async SSE, lossless durable usage/replay, SDK header/endpoint
inspection, actual two-tool session execution, malformed/bounded/cancelled
streams, explicit configuration requirements and redacted HTTP failures.
Source-bound logs/receipt belong to `target/migration/anthropic-*`.

Completion of this leaf requires these native text/tool checks; whole capability
stays partial/unverified. Live Anthropic credentials, OAuth, thinking/signature
and multimodal replay, cache-control policy, beta/server tools, retries, hosted
variants and permissive nonconforming-gateway recovery remain in the parent
provider task. Explicitly reject unsupported blocks rather than discard them.

Deletions: none. The default TS application and remaining providers still consume
legacy implementations. Their ledger entries remain pending in the parent task.
Remove only after complete retained contracts and last-consumer cutover pass.
Recovery: revert this bounded provider batch; preserve existing session records
and explicit OpenAI default. Do not infer release readiness from fixture checks.
