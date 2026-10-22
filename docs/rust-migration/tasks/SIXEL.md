# media:sixel — share bounded native terminal image conversion

Kind: migration. Prerequisite: `control:playbook`. Capability remains
`surface:media-local-inference` **partial** after this narrow extraction.

## Inputs, contract and destination

Read `crates/pi-natives/src/sixel.rs`, its image decoder and native exports,
`packages/natives` SIXEL tests/callers, terminal image renderers, workspace image
dependencies, and `crates/harvest-media/{Cargo.toml,BUILD.bazel,src/lib.rs}`.

Move encode/decode engines into `harvest_media::sixel`. Export ordinary Rust byte
slice/owned-byte APIs with structured Rust errors; keep thin N-API ownership/error
adapters while TS consumers remain. The Rust SDK must export the same engine.
Preserve valid SIXEL/PNG outputs, filtering, error mapping and byte handling.

Before decoding/resizing, validate input bytes, source/target dimensions, pixel
counts and bounded output. Keep the existing decode limits. Reject oversized
encode targets before allocations. Consult the actual `image` decoder limits:
its non-exhaustive allocation limit alone does not protect every decoder.

## Ordered steps

1. Capture normal/error baseline fixtures from the preceding implementation and
   rebuild. Do not recreate oversized-allocation failures on the old encoder.
2. Extract the shared engine, add explicit bounds and meaningful roundtrip/error
   regressions, and connect Cargo, Bazel, SDK exports and native adapters.
3. Exercise native Rust calls and rebuilt addon calls against the baseline.
4. Run existing host rendering tests. Remove duplicated Rust engine logic from
   N-API, but keep TS consumer/adaptor sources until their last consumer moves.
5. Record measurements and limitations; do not claim the full media capability.

## Checks, recovery and completion

```sh
cargo fmt -p harvest-media -p pi-natives -p harvest-omp -- --check
cargo clippy --locked -p harvest-media -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-media -p harvest-omp
bun run build:native
```

Consumer checks after rebuilding the addon:

```sh
FORCE_COLOR=3 bun test packages/coding-agent/test/utils/terminal-graphics.test.ts packages/coding-agent/test/bash-execution-sixel.test.ts packages/coding-agent/test/tools/bash-sixel-render.test.ts
```

The callers are `packages/tui/src/terminal-capabilities.ts` (encoding) and
`packages/coding-agent/src/utils/terminal-graphics.ts` (decoding). Add a direct
addon conversion contract if these host checks do not cover an engine boundary.
Installed Rust must perform conversion without Bun/Node on PATH. If native prerequisites are unavailable,
record the missing package/command and keep installed parity unverified.

Recovery: restore the prior adapter and engine together from the preceding
commit; leave session data untouched. Complete only with shared/native/installed
evidence and updated task records. The retained TS application remains operative.
