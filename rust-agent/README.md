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

Default builds enable `full`, including Harvest's Rust shell engine with a
bounded output channel and timeout. For a lightweight build with bounded
workspace inspection and approved file writes, use
`--no-default-features --features core`. Both produce `harvest` and `omp` binaries.
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

New runs create session-v3 logs at `.omp/session.jsonl`; subsequent runs resume
that log. Existing headerless prototype logs are rejected and preserved. V1/v2
logs need an explicit import to a separate v3 file before writing.

Inspect an existing session without changing it:

```sh
harvest session --workspace /path/to/session-directory --leaf entry-id session.jsonl
```

Import an existing v1/v2/v3 log to a separate v3 file:

```sh
harvest session --workspace /path/to/session-directory --import-v3 imported.jsonl legacy.jsonl
```

The import keeps every branch and preserves the source bytes. It rejects damaged
records, broken parent chains and existing destinations. The destination
filesystem must support hard links so publication cannot overwrite another
writer's file. Reading or importing opaque provider data does not establish
support for resuming it; unsupported replay still fails explicitly.

Recover an interrupted final record from a v3 log into a new file:

```sh
harvest session --workspace /path/to/session-directory --recover-to recovered.jsonl interrupted.jsonl
```

Recovery preserves the original and reports its hash, the recovered byte count
and the excluded tail byte count. It accepts incomplete final JSON objects,
including an interrupted UTF-8 character. Complete records, malformed syntax,
interior damage and broken branch references are rejected. Import and recovery
require cooperating Rust writers to release ownership first. A pending tool
call whose result was interrupted still prevents resume; recovery never removes
its call or executes it again. Legacy writer coordination and automatic repair
remain pending.

The bounded codec preserves raw v3 bytes and opaque provider payloads. V1/v2
records migrate in memory; inspection does not persist the migration. Session
text/function replay and durable appends are available in the experimental CLI:

```sh
harvest run --workspace /path/to/workspace --session sessions/current.jsonl \
  --prompt "Continue the previous task" --events
harvest run --workspace /path/to/workspace --session sessions/current.jsonl \
  --leaf entry-id --prompt "Continue this branch"
```

`--events` emits JSON lines with session ID, run generation and monotonic sequence.
Text deltas are provisional; committed events follow synced writes. The synchronous
SDK exposes `AgentSession::open`, `begin`, `execute` and `submit`, with cancellation
handles tied to each run. A writer lock protects cooperating Rust processes;
compare-and-append also detects changes by outside writers holding the file lock.

Replay currently supports text and ordinary function calls/results. Opaque,
signed, multimodal, compaction and custom-message replay fail explicitly. Damaged
logs and unresolved tool calls need explicit recovery before resuming. Existing
bytes and other branches remain intact. Provider usage accounting is pending;
new assistant records mark `usageUnavailable` alongside zero-valued usage fields.
The synchronous HTTP transport checks cancellation between reads and has a
60-second request timeout; cancellation cannot yet interrupt a stalled read.
Compatible OAuth, catalog/settings policy and the remaining lifecycle APIs are
still being ported.
