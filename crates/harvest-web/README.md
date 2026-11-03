# Rust browser replay

This first browser slice mounts Leptos CSR from `harvest-protocol`'s selected
branch projection. Rust renders text, reasoning, inline raster images, tool
calls/results, historical executions, compaction, and branch summaries. Search
is handled in Rust/WASM. Text and tool arguments are DOM text nodes, never HTML.
Provider signatures, hidden context, and opaque replay payloads do not enter
the browser snapshot.

Build a self-contained HTML artifact inside the workspace:

```sh
cargo xtask web-replay --session path/to/session.jsonl --leaf entry-id \
  --out target/web-replay/session.html --check-browser
```

The build requires the `wasm32-unknown-unknown` target and `wasm-bindgen` CLI
0.2.128 on `PATH` (or `HARVEST_WASM_BINDGEN` pointing to that executable).
`xtask` builds the production WASM and generates its bindings. The HTML embeds
both those bindings and the WASM bytes, so it can open through `file://` without
an adjacent WASM fetch. The stylesheet and startup are static packaging assets;
application rendering and search live in Rust. `--check-browser` requires headless
Chrome (or `HARVEST_REPLAY_BROWSER`) and verifies branch order, safe DOM rendering,
Rust search, and zero external fetches through the exported file URL.

This is a read-only replay foundation. It does not replace the current terminal,
HTML export, collaboration, statistics, automation, MV3 relay, or their routes.
There is no Markdown/math/theme parity claim or TypeScript cutover yet.
