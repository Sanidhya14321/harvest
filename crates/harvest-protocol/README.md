# Harvest protocol foundation

This portable Rust library owns the existing bounded session JSONL codec and a
read-only display projection of a selected branch. It has no filesystem,
application runtime, N-API or browser dependency and builds for native and WASM
targets.

`session::SessionDocument` preserves original bytes, unknown raw JSON values,
provider replay data, signed payloads and legacy import behavior. The existing
`harvest_omp::session_format` path re-exports this codec.

`display::project_branch` selects causal branch order and extracts explicit
display fields for messages, images, tool calls/results, summaries, model
changes and historical executions. Hidden custom bodies and opaque provider,
signature, redacted-thinking and unknown payloads stay outside serialized views.
The original records remain available separately. Tool arguments remain raw JSON
so large numbers are not rounded.

Rendering hosts must still sanitize terminal text, safely escape DOM content,
apply publication redaction and enforce their input/output budgets. A display
projection is never a provider replay history or a replacement session file.
The complete runtime/collaboration protocols, rich tool views, browser products
and terminal lifecycle remain migration work; this crate does not certify their
parity.

Run contracts through the sanctioned runner:

```sh
cargo xtask test -p harvest-protocol -p harvest-omp
cargo check --locked -p harvest-protocol --target wasm32-unknown-unknown
```
