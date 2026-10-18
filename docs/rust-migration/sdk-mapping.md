# SDK and extension migration status

The revised scope replaces authored JS import interfaces with Cargo APIs and
portable Rust/WASM plugins. This document describes working experimental Rust
foundations and pending replacements. It does not declare a stable SDK or an
available Wasmtime host. The old interfaces remain present during transition.

## Existing Rust foundations

| Legacy operation | Current Rust entry point | Remaining contract work |
| --- | --- | --- |
| `createAgentSession` | `harvest_omp::agent_session::AgentSession::open` | Production asynchronous lifecycle, settings/services injection, subscriptions, queued work, reload and shutdown |
| Submit a prompt | `AgentSession::submit` and `AgentSession::execute` | Steering, retries, compaction, delegation and complete provider replay |
| Abort the active operation | `AgentSession::begin` returning `RunHandle::abort` | Concurrent production scheduling and shutdown/draining |
| Observe streaming and persisted messages | `SessionEvent`/`EventKind` callback passed to `submit` | Persistent subscriptions and full production event vocabulary |
| Inject a model/provider | `harvest_omp::provider::Model` | Complete catalog/OAuth/provider transports and asynchronous streaming plugin bindings |
| Host tool approval | `harvest_omp::tools::Approval` | Full production policy, tool inventory and plugin host services |
| Session branches/storage | `DurableSession` and `SessionDocument` | Signed/multimodal replay, blobs, imports, crash repair and compaction |
| Native keyboard and terminal transforms | `harvest_terminal::{keys,input,printable,output,fuzzy}` | Complete TTY/PTY lifecycle, Unicode/text engines, widgets, Markdown, math and terminal product |
| Memory retrieval/maintenance | `harvest_brain` | Production committed-outcome adapters, leases, safe global promotion and extraction |

These APIs currently implement the experimental Rust CLI. Existing Rust tests
under `rust-agent/tests/agent_session.rs` exercise injected providers, approved
tool execution, durable events, cancellation generations and replay. They are
working embedding examples, but do not prove parity for the production JS SDK.
The same application core must ultimately serve both the CLI and Cargo consumers.

## Replacement interface mapping

The manifest retains one Rust SDK domain for each retired JS package interface:

| Retired JS package | Retained manifest domain |
| --- | --- |
| `@harvest/pi-agent-core` | `rust-sdk:agent` |
| `@harvest/pi-ai` | `rust-sdk:providers` |
| `@harvest/pi-catalog` | `rust-sdk:catalog` |
| `@harvest/pi-coding-agent` | `rust-sdk:application` |
| `@harvest/pi-mnemopi` | `rust-sdk:memory` |
| `@harvest/pi-natives` | `rust-sdk:native-engines` |
| `@harvest/omptype` | `rust-sdk:schema` |
| `@harvest/snapcompact` | `rust-sdk:snapcompact` |
| `@harvest/omp-stats` | `rust-sdk:stats` |
| `@harvest/pi-tui` | `rust-sdk:terminal` |
| `@harvest/typescript-edit-benchmark` | `rust-sdk:benchmarks` |
| `@harvest/pi-utils` | `rust-sdk:utilities` |
| `@harvest/pi-wire` | `rust-sdk:wire` |

The 17 separately inventoried SDK operations remain retained requirements. Their
JS signatures and object identity are replaced, while the operations themselves
must work through Rust APIs. Package-level mappings do not substitute for review
of each export's observable behavior.

Extension factories and in-process identity map to `rust-plugin:hooks-and-services`;
editor subclasses map to `rust-plugin:editor`; custom component factories map to
`rust-plugin:renderers`. All three replacements are unimplemented. Their WIT
contract must cover hooks/veto, subscriptions, timers, commands, streams, session
operations, UI/editor input and policy-mediated services. The host must reject
stale handles after reload and invoke plugin callbacks outside session locks.

New JS/Bun Eval execution has no replacement interface. Python/shell execution,
browser automation services and display of historical JS records are retained.
The corresponding removal entries remain pending until schemas, prompts,
settings, workers and dependencies are removed and rejection/history contracts
pass.
