# tooling:host-detect — connect native host identity to actual addon builds

Prerequisite: `control:playbook`. Source: `scripts/host-detect.ts`, removed in
concurrent checkpoint `8379766`. Destination: `xtask/src/host_detect.rs`.

The inherited port returned Rust architecture spellings and snake_case fields;
its consumers require Node spellings (`x64`/`arm64`), `x64Variant`, and the exact
addon filename. The Bazel wrapper also needs platform/arch in the JSON payload.
The corrected native detector emits those fields, uses native runtime AVX2/OS
instruction detection and rejects unsupported hosts before producing a filename.
The transitional wrapper must surface detector failure, rather than silently
selecting a fallback. No user data is changed by detection.

Inputs: the original source at `043709e`, both native-build wrappers, local addon
filenames and Rust detector serialization/unsupported-host tests. Preserve existing
consumer selection and generated binding exports. Do not claim that simulating
host identities is installed verification on Windows or macOS.

Verification:

```text
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask -p harvest-protocol
cargo xtask host-detect
bun run build:native
bun test packages/utils/test/docx.test.ts packages/coding-agent/test/utils/markit-cache.test.ts
cargo xtask playbook check
```

Run the copied detector with `PATH=/nonexistent` and verify its emitted filename
exists and the rebuilt addon loads. Rust tests cover supported platform/ISA JSON
branches and unsupported errors; the actual builder verifies the Linux connection.
Recover by reverting detector and consumers together, preserving unrelated work.
Completion needs executable builder/loader proof and a fully documented source row.
Other inherited removal claims stay blocked in CONCURRENT-REMOVALS.md.

Verified Linux checkpoint: strict xtask Clippy and 136 combined xtask/protocol
contracts pass. The copied detector runs with an empty PATH. The full native
builder consumes its identity, installs `pi_natives.linux-x64-modern.node`,
regenerates 139 exports, and actual DOCX/Markit consumer checks load that addon.
All 49 workspace groups pass. Full native TUI and unrelated whole-addon lint
failures remain tracked; this task certifies host identity/build connection only.
