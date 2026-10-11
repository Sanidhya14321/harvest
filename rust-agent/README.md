# Rust Harvest runtime

This experimental application belongs to the root Cargo workspace. It is not yet
a feature-compatible replacement for the installed TypeScript application. The
migration inventory and remaining gates are in `docs/rust-migration/README.md`.

```sh
cargo build --locked -p harvest-omp --bin harvest
cargo run --locked -p harvest-omp --bin harvest -- --help
cargo xtask test -p harvest-runtime -p harvest-brain -p harvest-omp
cargo xtask test -p harvest-omp --features full
```

The `core` profile provides bounded workspace inspection and approved file
writes. `full` additionally reuses Harvest's existing Rust shell engine with a
bounded output channel and timeout. Both produce `harvest` and `omp` binaries.
The host must approve each mutation or shell call; unattended runs deny them.
The experimental `omp run` and `omp tools` commands use the same library as
`harvest`.

```sh
cargo run --locked -p harvest-omp --bin harvest -- \
  run --prompt "Summarize README.md" --workspace .
cargo run --locked -p harvest-omp --bin harvest -- \
  brain --scope project organize --dry-run --json
cargo run --locked -p harvest-omp --bin harvest -- \
  /orgnize-skills --scope project --dry-run --json
```

Provider configuration currently uses flags, then `OMP_MODEL`, `OMP_BASE_URL`,
`OMP_API_KEY`/`OPENAI_API_KEY`, then the prototype's `.omp/config.toml`.
Help and version never install shell integration. Explicit setup is
`harvest completions bash --install`.

The prototype's headerless `.omp/session.jsonl` format remains separate from
legacy session v3. It has not been silently substituted for that format.

Inspect an existing session without changing it:

```sh
harvest session --workspace /path/to/session-directory --leaf entry-id session.jsonl
```

The bounded codec preserves raw v3 bytes and opaque provider payloads. V1/v2
records migrate in memory; inspection does not persist the migration. Session
resume and compatible runtime persistence are still being ported.
