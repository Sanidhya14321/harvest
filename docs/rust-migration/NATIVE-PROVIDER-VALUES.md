# Native provider credential and header values

Conversion base `cb4216d`. Code is written/connected; behavioral, installed and
platform verification remain deferred. This is not OAuth, credential-storage,
account-rotation or complete provider parity.

## One shared owner and its consumers

`configuration/values.rs` owns file-sourced provider values, one bounded workspace
command cache, exact environment lookup and flattened header materialization.
`values/command.rs` uses existing `pi_shell::execute_shell_streams` and shared
cancellation/process cleanup; it is not another shell or child-process owner.
`HttpModel::from_configuration` is the CLI/SDK builder retaining full resolved
configuration, explicit wire API, value sources and usage options. Sync/async run,
RPC and edit-benchmark run all use it. Both OpenAI-compatible and Anthropic
Messages requests resolve their headers through the same owner before HTTP I/O.
The existing transport continues rejecting redirects and owning JSON/SSE framing.

Native file/profile configuration accepts `api_key` and a `headers` map:

```toml
api_key = "HARVEST_PROVIDER_TOKEN"

[headers]
x-organization = "HARVEST_PROVIDER_ORG"
```

Headers replace an earlier layer's whole map, like the existing native service
arrays; this does not claim legacy model/provider override merge parity. A file
API key becomes a live source only when no CLI/environment API key overrides it.
Flags and `OMP_API_KEY`/`OPENAI_API_KEY` stay literal credentials. Standalone
`HttpModel::new(Settings)` preserves literal-key behavior. SDK hosts selecting
native configuration must keep `configuration::Resolved` and use
`from_configuration`, rather than discard headers/value sources through the
primitive `config::resolve` compatibility API. `with_config_values` supports
explicit SDK selection/replacement; an empty value set clears earlier sources.

## Value and header contract

A non-command value first looks up the exact environment key, including real key
casing on Windows. A nonempty environment result wins; empty/missing/non-UTF8
values use the literal source. Environment results are not recursively interpreted
as commands. Values are read again per request; successful commands are cached.
SDK `ValueResolver::headers` folds a bounded flattened source sequence once in
order, with later nonempty values winning. Failed/empty resolutions do not erase
an earlier value. No proxy/recursive property enumeration or generated JS is used.

Header names follow the HTTP token grammar and compare case-insensitively on the
wire. Case aliases in one native config map are rejected. Host, Content-Length,
Transfer-Encoding, Connection, Content-Type and Accept remain transport-owned.
Control characters (except horizontal tab) and oversized values are rejected
without including them in diagnostics. Native limits: 128 headers, 128-byte names,
16 KiB per source/value, 64 KiB aggregate source/resolved headers. Generic SDK
folding accepts at most 32 sources and 1,024 input entries and checks output bounds
incrementally. Authentication is added last when a key is available: Bearer for
OpenAI-compatible requests and x-api-key for Messages. Messages retains its
explicit protocol-version default. These are wire API choices, not model-name or
provider-name policy; KDL catalog policy remains separate.

## Explicit credential commands

Sources beginning with `!` select a configured shell command. Commands require
`--allow-credential-commands` on native run/RPC/benchmark run, or explicit SDK host
opt-in. The setting cannot enable itself from a file. Config inspection/migration
never executes credentials. These are host shell commands, not an OS sandbox;
the flag authorizes the selected immutable configuration's commands. Commands run
in the selected invocation workspace, with inherited environment and the existing
shell owner's null stdin and process/cancellation handling. Benchmark clones use
the invocation workspace, never mutated fixture working directories.

The resolver uses a scoped worker thread so synchronous SDK calls can originate
inside Tokio without nested block_on. The full shell build is required. Command
execution has a 10-second native budget plus caller cancellation, separate raw
stdout/stderr drains, an eight-chunk output queue and a 64 KiB stdout capture cap.
Overflow aborts through the shared token. Stderr is discarded; captured stdout is
never emitted as a tool result or added to session logs by this resolver. A
successful zero exit must yield nonempty UTF-8 stdout, trimmed and valid within
the value bound. Failed/empty/non-UTF8/oversized output is unavailable. A missing
configured API credential fails before provider HTTP I/O; failed header commands
are omitted as in the existing value resolver. Commands disabled/missing shell
support return explicit generic errors instead of invoking another language.

A cache instance is scoped to its selected workspace and shared across model
clones. A cancellable mutex serializes competing command resolutions. Positive
results persist until invalidation or bounded eviction; failures back off for 30
seconds using monotonic time. Caller cancellation is not negative-cached. Cache
identity hashes the trimmed command; command text is not a cache key. At most 128
entries and 1 MiB of key/value bytes are retained, with deterministic eviction.
`resolve(..., refresh=true, ...)` invalidates before resolving;
`HttpModel::invalidate_config_commands` clears API-key AND header commands without
running a request. Automatic 401/account/quota retry ownership is still pending.
Debug output omits command text, sources, values and workspace paths; resolver
errors discard underlying command/parser error chains.

## Remaining contracts and stabilization

The retained TS `model-config-values.ts` still serves the active model registry,
custom provider overrides, live-header proxy mutations and auth refresh cascade.
Do not delete it until those consumers move. Rust does not yet own the full models
schema/discovery/KDL merge policy, stored accounts/OAuth refresh, account stickiness,
quota/usage backoff, sibling rotation or automatic retry integration. No success in
this decoder establishes those features or unrelated provider families.

Consolidate fixture and installed tests for exact env lookup (Windows case too),
empty/literal fallback and nonrecursive results; mixed header sources, casing and
transport-owned names; source/header bounds and redaction; key precedence; real
local OpenAI-compatible/Messages requests through sync/async/RPC/SDK/benchmark;
commands disabled and core-build rejection; successful/nonzero/empty/non-UTF8/
oversized stdout; stderr isolation, stdin/fd isolation, cancellation/timeout,
process descendants, queue draining and invocation cwd; concurrent clone cache
sharing, 30-second failures, forced refresh and all-header invalidation. Add config
inspection no-execution and original-file immutability scenarios. Request body,
streaming/usage and final-argument tool approval regressions remain required.
Use `cargo xtask test -p harvest-omp` during stabilization. Compiler/format/ownership
checks only are recorded at this checkpoint; no runtime/performance claim is made.


Benchmark recovery continuation (unverified): Model exposes a default cancellable
invalidate_command_credentials seam; HttpModel delegates to the existing full
command-source invalidation above. The benchmark owner invokes it for selected
typed 401/403 failures before a mutation and inside its bounded retry/deadline
policy. This does not grant command execution or introduce general session auth
retry, OAuth refresh, account rotation or installed evidence. See RUST-BENCHMARK.md.
