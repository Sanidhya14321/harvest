---
name: napi-extract
description: Extract logic from a crates/pi-natives N-API module into an ordinary Rust library crate, leaving a thin N-API adapter so TypeScript callers keep working. Use when porting a pi-natives module (sixel, pdf, grep, text, tokens, etc.) for direct use by the Rust app (rust-agent / harvest-omp).
---

# Extract a pi-natives module

Reference implementation: `crates/pi-natives/src/html.rs` → `crates/harvest-media/src/html.rs` (commit 6253f50).

1. Read the module in `crates/pi-natives/src/<mod>.rs`, its TS callers (`grep -rn "<exportName>" packages/`), and the generated declarations in `packages/natives`.
2. Move the logic into the target crate (`harvest-media`, `harvest-tui`, `harvest-runtime`, `harvest-tokenizer`, or a new `crates/harvest-<area>`) with a plain Rust API: owned/borrowed Rust types, a typed error enum, options struct with `Default`. No `napi` types in the library.
3. Add explicit input/size/allocation bounds before work; check dependency types in `~/.cargo/registry` rather than guessing (e.g. `image::Limits` is non-exhaustive and not strict for all decoders).
4. Rewrite the N-API function as an adapter only: convert JS inputs (`crate::js::into_string`), call the library inside `task::blocking(...)`, map errors with `Error::from_reason`. Preserve the existing JS-facing signature, defaults and error text exactly.
5. Re-export from `rust-agent` (`harvest_omp::<area>`) if the Rust app needs it.
6. Wire: workspace `Cargo.toml`, `Cargo.lock`, `pi-natives/Cargo.toml` dependency, Bazel `BUILD.bazel` targets, Rust CI package list.
7. Verify: Rust tests (`cargo xtask test -p <crate>`), `bun run build:native`, then diff generated bindings — revert the four known unrelated audio declaration lines (`AudioCapture.droppedChunks`, `AudioPlayback.queuedSamples` + comments) if they appear. Compare old vs new addon output on an oracle input set; run focused TS consumer tests.
8. Continue with `/rust-migration-slice` steps 5–7 (ledger, docs, commit).
