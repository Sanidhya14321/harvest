# Consolidated Rust migration development

`ts-to-rust` is the active development branch. Its existence is intentional even
when it temporarily has the same files as `main`. Integration and verification
happen here; publishing changes to `main` is a separate action.

The integration uses development tip `765e0d15` and preserves the history of
`codex/rust-migration` at `8a647531`. The latter branch can be removed after the
combined branch is pushed and its history is verified reachable.

## Integrated capabilities

- A portable lossless session codec and allowlisted display projection live in
  `harvest-protocol`. The codec retains the development branch's newer import,
  recovery and title behavior. Hosts implement its atomic storage boundary;
  browser consumers do not depend on the filesystem runtime.
- The async `SessionEngine`, bounded follow-up/event queues, cancellation,
  shutdown and experimental JSONL RPC adapter are available alongside the
  existing synchronous SDK. `run --async-session`, `rpc` and read-only `replay`
  are explicit entrypoints. The async driver uses the current lossless usage
  response interface and compaction/context replay instead of the older driver.
- Provider I/O uses cancellable asynchronous transport while preserving typed,
  redacted status/transport errors and caller-thread callbacks.
- Native and Rust session writers share one kernel lock implementation and
  canonical `.lock` identity. New-file symlink ancestors are resolved through
  the authorized directory capability. Existing bounded, nonblocking regular
  file reads and cleanup-safe atomic staging are retained.
- Portable frame/history transactions, fuzzy matching, terminal replay and WASM
  bindings are integrated in `harvest-terminal`. Its key and text compatibility
  facades delegate to the newer `harvest-tui` engines. The newer input framer,
  mouse/highlight/geometry APIs, output writer, media and tokenizer remain intact.
- The read-only Rust/WASM browser replay viewer uses allowlisted display fields,
  escaped text, bounded images and search. It does not execute persisted content.
- `cargo xtask npm-versions`, `path-versions`, `registry`, `source-audit`,
  `terminal-wasm` and `web-replay` expose package preparation, detached registry
  verification, dependency-aware check receipts, rollback bundles and portable
  viewer generation. Release preparation uses the newer workspace version
  updater followed by path-dependency synchronization. The legacy npm sync
  entrypoint is a thin adapter to the Rust implementation.
- Update downloads use the repository's actual artifact names and preserve each
  installed command alias. Local collaboration binds to loopback; stats assets
  enforce path containment and distinguish missing assets from SPA routes.

## Preserved newer implementations

The older codex checksums, ZIP/installers, release notes, workspace version CLI,
terminal keys/text engines and shell fork are superseded by the newer development
implementations. They are not installed as competing implementations. The current
source baseline and deletion ledger remain authoritative. The byte-level source
audit is an additional tool, not a substitute for behavioral migration evidence.

The existing capability inventory is retained and augmented conservatively for
the integrated partial surfaces. The older, differently scoped 494-entry
inventory and its retirement claims are not substituted for the current evidence.
Transition check receipts do not certify full application parity, and release
verification also requires the existing source-deletion gate.

## Acceptance

Validation covers native application/session/provider contracts, async ownership
and RPC, portable protocol/frame/replay/WASM contracts, registry/version tooling,
the rebuilt addon, legacy writer handoff and the affected TypeScript consumers.
The verification results below describe this consolidation only.

This consolidation does not finish the overall TypeScript-to-Rust migration.
The normal application still launches TypeScript, the Rust RPC and browser viewer
remain experimental subsets, and the existing distribution cutover gates remain
blocked until full application and installed-platform parity are established.

## Verification

- `cargo xtask test -p harvest-protocol -p harvest-runtime -p harvest-tui -p harvest-terminal -p harvest-terminal-wasm -p harvest-web -p harvest-brain -p harvest-omp -p xtask`: 386 tests passed, followed by the doctest pass.
- Strict Clippy (`--all-targets --all-features -- -D warnings`) and formatting passed for the integrated Rust packages. The application also checked with `--no-default-features --features core`.
- `cargo check --locked -p harvest-web --target wasm32-unknown-unknown` passed. The release WASM export rendered in Chromium over a loopback HTTP server: selected history/order, Rust search, escaped hostile content, opaque-field redaction and no external resource requests passed. Managed-browser policy blocks `file://` navigation here, so the file-navigation variant of `web-replay --check-browser` could not run in this environment.
- `cargo xtask terminal-wasm --check` passed against the regenerated committed payload. Portable text, fuzzy matching and detached installation tests passed; the final payload consumer run passed 39 tests.
- Update/release contracts passed 115 tests; affected relay, stats, fuzzy and fallback contracts passed 37 tests. The coding-agent, natives and TUI package checks and tools checks passed.
- The Linux native addon rebuilt successfully, including regenerated declarations/exports. Its selector scores/ranking, Unicode chunking, SGR, width and input adapters passed against the portable contracts. `HARVEST_RUST_OMP=target/debug/omp bun test scripts/rust-session-ownership.test.ts` passed against the rebuilt addon: legacy ownership blocks Rust writes before provider I/O, release permits durable completion, and the successor reacquires ownership.
- The source ledger and capability inventory validate at 11/5,232 migrated source entries and 20/188 partially implemented capabilities. No capability claims complete Rust parity. `migration --require-complete` still rejects distribution cutover as intended.
