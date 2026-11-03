# Approved strict Rust-only target

The user approved this replacement scope on 2026-10-22. It supersedes the older
browser-JavaScript and installer-script exceptions in archived plans. Development
and verified commits belong on `ts-to-rust`; publication and merging are separate.

## Executable ownership

First-party application, library, test, example, automation, generator and release
implementation must be Rust. No tracked or shipped executable JS/TS artifacts
remain, including generated bindings, declarations and browser bootstrap glue.
Port owned Python/Julia automation and shell/PowerShell installer logic to Rust.
Do not disguise executable scripts as data assets, templates or Rust strings.

Retain configuration, prompts, documentation, declarative presentation/build
assets and explicitly reviewed inert fixture data. Foreign-language benchmark
inputs may be preserved as inert data and materialized into temporary paths by
Rust tests. Third-party native dependencies and external Git/Chrome are allowed;
they do not permit first-party non-Rust helper implementations. The standard
build, tests, installation and operation must not require Bun/Node/Python/Julia.
Transitional adapters remain only while recorded consumers still need them.

## Approved interface changes

- Replace handwritten TypeScript/Python SDKs with the Rust SDK and documented
  versioned RPC. Replace TS extensions with executable plugin ABI v1 and a Rust
  authoring SDK. Use bounded JSONL stdio, negotiation, IDs, events, cancellation,
  structured errors and brokered final-argument approvals. No bundled JS runtime.
- Retire JavaScript/Python/Julia eval tools, workers, preludes and kernel helpers.
  Retain the ordinary shell tool for user-requested external commands. Preserve
  SSH and browser automation as separate native capabilities, not eval helpers.
- Replace interactive browser products with the native TUI: stats, collaboration
  guests, replay/search, benchmark runs and automation queues. Do not introduce a
  desktop GUI framework. Preserve reusable protocol and terminal engines.
- Replace the JS Chrome relay with direct Rust CDP control of a dedicated
  persistent browser profile. Users sign in there separately. Preserve ownership,
  tab lifecycle, bounded I/O, cancellation and cleanup; ship no injected JS.
- Replace executable installer scripts with native archives, Cargo and package
  managers. Keep both command aliases. Native interactive completion replaces
  shipped executable shell completion/bootstrap artifacts.
- Benchmark scoring becomes `rust-v1`: patch application, expected-output
  matching after CRLF-to-LF normalization, time and usage. Preserve inputs and
  historical scores; never mix versions or claim Babel/Prettier equivalence.

These are breaking interface changes, not permission to drop unrelated retained
features. Each replacement needs a retained capability and installed evidence.
Robomp automation, collaboration, stats, SSH, memory, providers, recovery and
normal shell behavior remain in scope. Retirement is pending until verified.

## References informing the playbook

- [OpenAI execution plans](https://developers.openai.com/cookbook/articles/codex_exec_plans)
- [Anthropic long-running agent harnesses](https://www.anthropic.com/research/effective-harnesses-for-long-running-agents)
- [Chrome debugging configuration](https://developer.chrome.com/docs/devtools/agents/get-started/configuration)
