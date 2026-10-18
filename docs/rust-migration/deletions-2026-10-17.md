# Verified cleanup — 2026-10-17

These changes preserve the active TypeScript application during migration.
File removal is independent of complete installed-product parity.

| Removed implementation | Replacement and consumer evidence |
| --- | --- |
| `scripts/sync-versions.ts` | `xtask npm-versions`; three captured legacy transformations, read-only drift checks, invalid-input and symlink rejection contracts. Release preparation uses Rust for public versions, explicit internal dependencies and the Bun catalog; private versions and protocol links stay intact. |
| `scripts/ci-release-notes.ts` | `xtask release-notes`; captured legacy output fixtures cover silent-tag ranges, deduplication, category order and canaries; errors preserve existing notes; the authoritative Rust release workflow uses the replacement. |
| `scripts/ci-update-brew-formula.ts` and its unit test | `xtask package homebrew`, already used by Rust CI and release tooling; generated URL/checksum contracts now also cover bare-binary `:nounzip` and writable `HOME` during completion generation. The unused TS generator had no remaining caller beyond its own test. Actual target-host installation remains a release gate. |
| `rust-agent/src/session_format.rs` | Portable `harvest-protocol::session`; the existing public import path remains a direct re-export. Lossless codec and CLI tests retain original provider payloads and bytes. |
| Duplicate JS wrapping, clipping, slicing and overlay algorithms in `packages/natives/native/loader-state.js` | Extracted Rust text engine through bounded UTF-16 WASM bindings; captured native transformations, existing text consumers and detached bundle/compiled-install tests pass. Bun's small-packet width bridge remains pending its performance gate. |
| N-API-owned text engine implementation | Moved to `crates/harvest-terminal/src/text.rs`; native adapters retain public shapes and unchanged-string identity. The optimized addon rebuild preserves all 123 exports. |

Earlier verified cleanup (selector implementation, ConPTY fallback, checksum
script and duplicate platform lock implementations) remains preserved. Active
Composer/TranscriptStore, provider/settings/auth logic, JS workers and browser
services still have retained consumers and cannot be deleted yet. JS Eval's
removal remains ordered after the retained Python/browser execution replacement.
