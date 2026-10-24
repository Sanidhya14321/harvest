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
Help and version never install shell integration. Executable shell completion and
bootstrap generation is retired; `harvest completions` returns migration guidance
without emitting scripts or installing files. Native in-application completion
remains under migration. Native archives/package managers retain real command aliases.

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
Text deltas and bounded declarative plugin events are provisional; committed events
follow synced writes. Plugin events include parent tool-call correlation and also
reach RPC v1/v2; TUI rendering and interactive choice replies remain unfinished. The synchronous
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
Configuration parse errors retain only configuration identity and byte or line/column
positions; SDK settings,
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

## DOCX documents

`harvest document --workspace /path/to/workspace --format markdown relative.docx`
converts a workspace-confined document without a language runtime. Formats are
`markdown` (default), `html`, and `json` (HTML plus diagnostics). Native agent
`read` also recognizes `.docx`, including uppercase extensions. Markdown omits
embedded image bytes and retains alt text with an omission link. HTML/JSON keep
inline images. Inputs and workspace files are never modified.

The Rust SDK exposes `harvest_omp::docx::{convert, prepare, markdown, Options}`;
operations accept `harvest_runtime::cancel::CancelToken`. `prepare`/`finish`
allow callers to supply image attributes without reimplementing document parsing.
ZIP input is limited to 64 MiB, expanded data to 256 MiB, each member to 64 MiB,
XML parts to 16 MiB, XML nodes to 200,000 per part, nesting to 64, and final HTML
to 64 MiB. Archive entries, images and table columns are each limited to 4,096.
External relationships are never fetched. Malformed XML, DTDs, invalid archives,
limit violations and cancellation return errors rather than partial success.

The legacy Markit importer currently uses a temporary native callback bridge.
Its TypeScript API remains pending migration; this does not complete all media
or document-import capabilities.

## Experimental native RPC transcript queries

`harvest rpc` serves bounded JSONL over stdin/stdout using the native session
owner. `get_messages_page` returns committed messages, `totalMessages` and an
optional `nextCursor`; pass that cursor to obtain the next page. `limit` defaults
to 100 and must be between 1 and 256. Cursors identify the session, branch leaf
and message count. Error codes `session_busy` and `stale_cursor` tell clients to
wait or restart pagination. Messages retain their original provider fields.

`get_last_assistant_text` returns `{ "text": "…" }`, or `{}` when no text is
available. It combines only text blocks from the most recent eligible assistant.
The full history query `get_messages` remains available. An individually oversized
message produces an explicit v1 frame-limit error; history is never truncated.
Steering, compaction and other production RPC commands remain pending. This
opt-in adapter has not replaced the production TypeScript RPC mode.

### Native RPC transport and clients

The initial `ready` frame advertises versions 1 and 2. Send
`{"id":"version","type":"negotiate_protocol","protocolVersion":2}` and wait for
its successful response before accepting v2 chunks. Physical JSONL lines remain
limited to 1 MiB including the newline. Negotiated v2 carries logical JSON objects
up to 64 MiB using ordered `rpc_chunk` frames with 256 KiB base64 payloads.
Large histories and events retain their complete contents. Commands sent to the
server remain limited to one physical line, matching the legacy command input.

Rust clients can use `harvest_omp::rpc::reader::Reader::new(stdout)` and its async
`next()` method. After acknowledgement, call
`set_version(harvest_omp::protocol::rpc_frame::Version::V2)`. The reader bounds
physical reads, reassembles logical messages and preserves partial input if a
read future is cancelled. Invalid/interrupted chunks and incomplete messages at
EOF terminate the reader; reconnect after a transport error. The portable
`protocol::rpc_frame::{encode, Decoder}` APIs support other native I/O hosts.

The server uses one ordered writer, a 64 MiB queued-byte budget and a 10-second
deadline for each complete logical write. Serialization itself enforces the
negotiated size limit; chunks are encoded lazily as output drains.

### Native Anthropic Messages text and tools

The experimental Rust candidate supports an explicit Messages wire path:

```text
harvest run --api anthropic-messages --model YOUR_MODEL --base-url https://api.anthropic.com/v1 --max-output-tokens 4096 --prompt "Read the workspace"
```

Set `OMP_API_KEY` rather than putting credentials in command history. The same
flags select the provider in `harvest rpc`; `--async-session` uses the shared
session owner. Rust embedders select
`HttpModel::new(settings).anthropic_messages(max_output_tokens)?`.

Text/tool calls, matched results, streaming cancellation and raw usage are
supported. Thinking/signatures, images, OAuth, beta/server tools, retry policy
and catalog configuration remain pending. Unsupported response blocks fail
explicitly. This opt-in candidate does not replace the default application yet.

### Native MCP conversion checkpoint

The Rust candidate has an opt-in MCP stdio/streamable HTTP client shared by
sync/async sessions, RPC and the Rust SDK. Native config uses `[[mcp]]` with
`--enable-mcp`; explicit JSON files use `--mcp-config PATH`. The `harvest mcp`
command lists tools/resources/prompts and performs explicit calls. Model calls
still require host approval. See [native MCP migration status](../docs/rust-migration/NATIVE-MCP.md)
for configuration, APIs and unfinished OAuth/legacy SSE/event/platform behavior.
This conversion checkpoint compiles; behavioral and installed checks are deferred.

### Native language/debug conversion checkpoint

Host-selected `[[lsp]]` and `[[dap]]` native configurations expose the shared
`lsp`/`debug` tools using --enable-lsp/--enable-debug. The same Rust clients serve
sync/async sessions, RPC, SDK users and `harvest lsp`/`harvest debug` CLI commands.
`--interactive` keeps terminal CLI ownership for multiple structured requests.
See [native language/debug status](../docs/rust-migration/NATIVE-LSP-DAP.md) for
approval, configuration and unfinished discovery/recovery/platform contracts.
Behavioral and installed verification remains deferred for this checkpoint.

Native file `api_key` and `headers` sources can resolve live environment values;
configured credential commands require explicit `--allow-credential-commands`.
Sync/async run, RPC, benchmark and SDK `HttpModel::from_configuration` share the
same resolver. Standalone `HttpModel::new(Settings)` keeps literal credentials.
See [provider value contracts](../docs/rust-migration/NATIVE-PROVIDER-VALUES.md)
for cache/size/cancellation limits and pending OAuth/registry/retry work. This
conversion code has not yet passed consolidated behavioral/installed checks.

Native editing now has one volatile, bounded `pi_edit::EditStore` per live tool
session. Select `--edit-mode replace|patch|apply_patch|hashline|sloppy` on native
`run` or `rpc`, `OMP_EDIT_MODE`, or `edit_mode` in native configuration/profiles.
Flag/environment selection overrides the selected profile. Exact replacement is
the default. Model tool schemas describe the selected engine; models cannot switch
it through arguments. The host approval receives the original call identity and
`{"mode": ..., "arguments": ...}` containing the complete final model request.

Hashline reads return `[path#TAG]` and complete numbered lines. `offset` and `limit`
are positive one-based line windows. Output is bounded; truncated rows grant no
line authority. Use further reads before editing unseen rows. BOM/CRLF and
notebook cell views use the shared edit decoder. DOCX extraction is read-only.
Native strict provenance rejects absent/empty state, stale tags, unseen resolved
block spans and repairs that remove unseen lines. Whole-file moves/deletes require
all current source lines to have been read. Historical addon defaults are retained.

Rust hosts use `ToolRuntime::configure_edit`, `AgentSession::configure_edit`, or
`SessionEngineOptions::edit_mode`. This state survives turns/follow-ups within that
owner; mode changes clear it. Reopening/resuming a durable log creates fresh state:
replayed tags and metadata grant no read authority. Host config selects the mode
again on resume. Direct `write` invalidates snapshots. Existing stateless free tool
functions remain exact-replacement conveniences; use `ToolRuntime` for a read/edit
workflow. Structured native partial-write progress is persisted by sync/async/RPC
owners, while writes remain sequential rather than an all-file transaction.

`harvest edit-benchmark run --edit-mode hashline --guided ...` enables explicitly
oracle-assisted `rust-hashline-guidance-v1`. Each attempt regenerates a bounded
patch from current workspace bytes, validates it with the shared parser/applicator,
and records arguments or a skip reason in `guidance-NNNN.json`. Guidance grants no
read authority; the model must still call `read`. Run plans record mode, policy and
version. Unguided and historical results remain separate. Conversion code and new
contracts are uncompiled/unexecuted pending the consolidated integration pass;
this does not certify full legacy tool parity or change default launchers.

Retained state bounds: snapshot versions/paths/text use EditStore's existing
limits. No-op path counters now obey the same maximum path count (deterministic
key eviction; a zero-path store retains none). Native writer preflight admits at
most 256 named clipboard registers, 256 bytes per register name and 16Mi UTF-16
units across retained names/rows, before any publication. Anonymous clipboard
state remains batch-local. Consolidate retention/eviction/clipboard bounds with
the workflow checks; these conversion changes are uncompiled/unexecuted.

Native read also accepts `selector` or a trailing `path:selector` when the complete
literal entry is confirmed absent. Selectors include `N`, `N-M`, `N+K`, `N-`,
`N..M`, comma-separated ranges, `-N` tail and `raw` compounds. Bare `N` reads from
that line onward. Ranges sort/merge, omitted gaps stay unseen, and raw output grants
no new numbered-line edit authority. Use either selector or offset/limit. Conflicting,
malformed, overflow and unimplemented named readers fail explicitly. Literal colon
filenames and dangling symlinks take precedence. This continuation is also uncompiled
and unverified; the production TS read/grep consumers remain in place.
