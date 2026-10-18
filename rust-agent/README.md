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

New runs create session-v3 logs at `.omp/session.jsonl`; subsequent runs resume
that log. Existing headerless prototype logs are rejected and preserved. V1/v2
logs need an explicit import to a separate v3 file before writing.

Inspect an existing session without changing it:

```sh
harvest session --workspace /path/to/session-directory --leaf entry-id session.jsonl
```

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
The HTTP transport uses bounded asynchronous reads; cancellation interrupts pending
headers and stalled body reads while callbacks stay on the calling host thread.
Compatible OAuth, catalog/settings policy and the remaining lifecycle APIs are
still being ported.


Read-only terminal replay uses the same display projection as the browser viewer:

```sh
harvest replay --workspace /path/to/session-directory --leaf entry-id --columns 80 session.jsonl
```

Control characters render as visible text; hidden context, provider signatures
and opaque payloads stay out of the transcript. Oversized projections fail before
any terminal output. The transactional frame writer is a library foundation;
this command does not implement an interactive editor or complete terminal UI.

The opt-in `harvest rpc` command uses the asynchronous session owner and JSONL v1.
It supports text `prompt`, FIFO `follow_up`, generation-scoped `abort`, `get_state`
and exact persisted `get_messages`. `resume_follow_ups` explicitly resumes work
parked by cancellation. Disconnection cancels work and releases the writer lease.
It rejects steering, image prompts, slash commands, protocol v2 and other unported
commands explicitly. Mutations are denied because this CLI has no approval host;
the library accepts a host-owned policy. Configuration still uses the prototype
formats above; production settings/auth parity remains required. The normal
TypeScript `--mode rpc` stays active until its complete contract passes.
