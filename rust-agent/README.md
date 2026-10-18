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

Rename an intact v3 session without reserializing its header or message records:

```sh
harvest session --workspace /path/to/session-directory --title "Project review" session.jsonl
```

This writes a 256-byte UTF-8 title slot and marks its source as `user`.
Long titles truncate at codepoint boundaries; an empty title clears the slot.
Import or recover legacy/damaged sessions first. Title changes share the Rust
writer lease with resume, import and recovery. Legacy writer lease coordination
is still pending; do not run both applications against the same writable log.

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

Replay supports text, ordinary function calls/results, text compaction,
handoffs and branch-return summaries. The latest compaction supplies the summary
and kept tail; a later durable `/clear` boundary starts context after that marker.
`--leaf` selects the actual branch, and new messages append to that leaf while
all original history remains unchanged. Missing, sibling or forward kept-tail
references and incomplete tool pairs are rejected before writing.

Active opaque provider/bitmap compactions, signed/multimodal messages and custom
messages need replay adapters and fail explicitly. Damaged logs require explicit
recovery. Summary templates use Handlebars without HTML escaping; native replay
preserves summary whitespace rather than applying the legacy prompt formatter.
Records without usable accounting mark `usageUnavailable` alongside zero-valued
compatibility fields; complete provider accounting is described below.
The synchronous HTTP transport checks cancellation between reads and has a
60-second request timeout; cancellation cannot yet interrupt a stalled read.
Compatible OAuth, catalog/settings policy and the remaining lifecycle APIs are
still being ported.

### Provider accounting

The native OpenAI-compatible transport preserves raw `providerUsage` metadata
and records token/cache counters for complete integer usage reports. Partial,
invalid or overflowing reports remain lossless and set `usageUnavailable`.
Reasoning tokens remain output metadata rather than being counted twice.
Pricing is not ported: `costUnavailable` stays true, and compatibility cost
fields remain zero. Do not treat them as a billed-cost estimate.

`harvest run --request-usage ...` explicitly sends
`stream_options: { "include_usage": true }`. The default omits that field until
host compatibility policy is catalog-resolved. Usage returned by the host is
captured with either setting, including JSON fallback responses. Only completed
responses enter the durable log. Replay sends chat content and tool protocol
blocks; accounting metadata stays in storage.

Native provider failures expose typed HTTP status or transport categories.
Configuration parse errors retain only the filename and byte offset; SDK settings,
environment and override debug output omit API keys and endpoint URLs.
Error chains and `HttpModel` debug output omit endpoint URLs, credentials,
upstream status text and response bodies. Raw HTTP failures are not attached as
error sources. Provider authentication/retry policies still require their
separate compatibility ports.

### Native file search

The native `grep` tool uses `pi-walker` for traversal and shared ignore rules.
Matching is case-sensitive and respects ignore files by default. Tool arguments
`case: false` and `gitignore: false` override those choices; an explicit file
operand is searched directly. File reads remain workspace-jailed and bounded.
Binary, invalid UTF-8 and oversized files are skipped. Search stops at match,
entry and depth bounds and emits `[search limit reached]` when those limits
are reached. Transient application/build directories remain excluded.

This is a partial grep port: archive/SSH/internal URL inputs, selectors,
PCRE-specific matching, pagination and the legacy renderer still need their
own compatibility work. Tool descriptions live in static Markdown assets.
