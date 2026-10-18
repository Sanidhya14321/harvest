# One integrated Rust cutover plan

Status: **blocked for public Rust release**. Audited on 2026-10-16 against the
existing dirty checkout. This is an execution order with acceptance gates, not a
claim that the remaining ports can be completed by running a packaging command.
Keep the working application available throughout implementation; promote one
fully verified Rust release after all retained contracts pass.

## Evidence and source coverage

The reproducible [source ledger](source-audit.json) reads and fingerprints every
Git-tracked or nonignored untracked checkout file, including assets, prompts,
tests, configuration and non-TypeScript services. It excludes ignored build
outputs, Git internals and its own output. Deleted tracked files are recorded
separately. Regenerate it after the checkout changes:

```sh
cargo xtask migration audit
cargo xtask migration status
```

At the initial audit, 7,584 files / 206,580,375 bytes were read. There were 4,936
JS/TS files outside the explicit generated-binding, vendor and language-fixture
exceptions: 2,196 application files, 103 tooling files, 2,583 test files, 38
examples and 16 declarations. Counts can change as audit fixes land; the ledger
and command output are authoritative for their recorded source fingerprint.
Inline scripts in HTML and Python service logic also require migration review;
the JS/TS extension count is not the complete product scope.

Reading bytes establishes inventory coverage. It does **not** establish semantic
review, correct classification or behavioral parity. The scoped semantic reads,
remaining unread implementations and concrete ownership gaps are recorded in:

- [Core and persistence audit](audit-core-2026-10-16.md).
- [Terminal, browser, routes, middleware and workers audit](audit-surfaces-2026-10-16.md).
- [Build, registry, installer and release audit](audit-release-2026-10-16.md).

The capability manifest has 284 retained capabilities, with 6 complete and 14
partial Rust implementations, **0 verified for production cutover**. All 210
intentionally retired interfaces still lack verified removal. Expand the
manifest wherever the route/surface audit identifies uncovered behavior; these
494 rows are an inventory, not proof that every feature has been identified.
For each file, classify authored/generated/vendor/fixture, finish semantic review
of its consumers and assign its retained behavior to a capability before deleting
it. A file named `*.generated.ts` is not automatically exempt: the current HTML
tool views are generated from authored React code.

## Architecture to implement

Rust libraries own domain state, session orchestration, persistence, catalog/KDL
resolution, provider transports, tools and integrations. Extract reusable native
engines from N-API adapters. A Tokio application service owns one engine per
session; callbacks execute without holding its session locks. A Cargo SDK and a
versioned WIT/Wasmtime component host use the same services. Initialize Wasmtime
only when plugins are enabled.

The terminal application uses shared Rust text/input/image/PTY engines. Its
Composer and TranscriptStore own semantic history; the renderer submits immutable
monotonic history batches and acknowledges only after accepted output. Rust owns
the complete editor, widgets, layouts, themes and restoration lifecycle.

Browser products use Leptos and `web-sys`; generated bindings only connect browser
APIs to Rust state and rendering. Rust owns shared display models and protocols
for terminal, browser, exports and services. A Rust HTTP/WebSocket/SSE service
layer owns routes **and** authentication, authorization, bounds, origin checks,
cancellation and error mapping. Keep loopback defaults and require authentication
for intentional non-loopback services. Static CSS/HTML may remain assets.

Generated npm/Bun launchers select a platform and start Rust. They do not contain
application logic. JS SDK/extension APIs are intentionally replaced, and JS/Bun
Eval is intentionally removed; Python/shell execution, browser automation and
historical JS tool-card display remain retained features. See [scope](scope.md)
and [SDK migration mapping](sdk-mapping.md). The relevant upstream boundaries are
documented by [Leptos](https://book.leptos.dev/web_sys.html),
[Wasmtime](https://docs.wasmtime.dev/wasip2-plugins.html) and
[Chrome CSP](https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy).

## Dependency order and deletion gates

Every unit follows the same sequence: read the implementation and all consumers;
record deterministic fixtures from the working program; implement Rust; run the
same observable-contract fixtures against both implementations; switch consumers;
verify installed assets; remove the replaced implementation and obsolete
dependencies; rerun the relevant suites. Keep baseline fixtures and results after
their TS runner is replaced. A deletion is the final step of a verified unit,
never the evidence that its replacement works.

| Unit | Implementation and consumer scope | Exit evidence before dependent units proceed |
| --- | --- | --- |
| 0. Review and baselines | All ledger files; `coding-agent/DEVELOPMENT.md`; commands, SDK exports, browser routes, workers, prompts, generators, installers and Python services | Every retained capability has ownership, consumers, observable fixtures and platform requirements; every retired API has an explicit migration/removal contract. Record source fingerprint, paired performance samples, installed baseline and previous stable release digest. |
| 1. Shared primitives | `harvest-runtime`, `harvest-terminal`, native TTY/PTY/image/text/edit/AST/VCS engines and all adapters | Escape/paste/focus input split at every relevant boundary; Unicode width/slicing/wrapping/math; image/output bounds; jailed delete/rename/nonexistent leaves and symlinked ancestors. Shared ownership and cancellation remain interoperable with the transition CLI. |
| 2. Production state | `session/*`, settings/config/secrets, catalog discovery/KDL/OAuth, blobs, memory engines | Branch/import/fork/compaction and opaque/signed/multimodal replay; private permissions; concurrent writers, interrupted appends and recovery; exact tool/result pairing; versioned migrations and rollback. Replace whole-log hashing per append only after conflict-detection contracts and large-session performance pass. |
| 3. Async application and providers | `sdk.ts`, `main.ts`, agent loop, providers, policy, scheduling, approvals, delegation and shutdown | Subscriptions and event order; steering/follow-up queues; streaming usage; retry/backoff; queued work; cancellation of pending headers/body reads; compaction/reload/shutdown; injected clocks/services and reentrant callbacks. Test every retained provider wire family and KDL-owned policy. Replace the fixed-step prototype loop. |
| 4. SDK/plugins and tools | Rust session/tool/provider/hook/editor/renderer/dialog APIs; WIT components; bundled extensions; all canonical/hidden tools | Detached Cargo consumer examples; veto/precedence, provider/tool streaming and timers; policy-mediated filesystem/credentials/processes; active reload drains/cancels and detaches registrations; stale handle rejection. Retired JS consumers receive migration examples and explicit removal errors. |
| 5. Terminal product | `tui/src`, `coding-agent/src/modes`, `tui`, theme/editor/input/renderers and worker-facing lifecycle | Golden terminal traces for live/rebuilt previews, history/replay/ACK/retries and failed writes; resize modes, anchors, overlays/focus, images, streaming/backpressure, signals and terminal restoration. Platform PTY/ConPTY and tmux/screen runs; 5,000-block transcript p95 <10 ms and scaling ratio <=1.3. |
| 6. Services and connections | Every service family in the surface audit: RPC/ACP, collab relay/host/guest, stats, metaharness, auth broker/gateway/OAuth, blobs/callbacks, launch broker, internal URIs, MCP/LSP/DAP/SSH, tiny inference, debug and VM forwarder | Compare framing, IDs, ordering, status/header/error shapes, authentication and read-only/disabled behavior. Test reconnects, disconnect cancellation, bounded slow clients, owner-only sockets, non-loopback auth and workspace containment. Preserve unknown stored/forwarded protocol variants. |
| 7. Browser products | Collab guest, all 11 stats sections, metaharness, shared tool cards, HTML/share viewer, MV3 options/relay and robomp dashboard/backend/proxy | Production WASM routes, charts, navigation, themes, keyboard/accessibility, loading/errors and reconnects. `file://` export works with embedded WASM and no adjacent fetch. Missing assets 404, WASM MIME correct. MV3 listeners register during initial worker evaluation, local WASM/CSP works, termination/revival recovers. Automation preserves durable queues, HMAC over raw query/body, replay protection, allowlists, cancellation and redacted logs. |
| 8. Retire JS and tools | `eval/js`, JS schemas/settings/prompts/workers, N-API application adapters, first-party build/generator/test TS, old UI bundlers and dependencies | Python/shell/browser services already pass through Rust; historical JS records still render, new JS execution returns explicit removal error. Equivalent Rust/browser tests replace TS runners. Rust `xtask` owns codegen, WASM, package/asset assembly and install verification. Source ledger has no authored first-party JS/TS beyond reviewed exceptions and generated launch/binding code. |
| 9. Seal and promote | One version, manifest, tag and candidate commit across standalone, Cargo SDK/plugins, WASM, npm/Bun payloads and installer metadata | All functional, performance, registry and installed-artifact jobs pass for the same immutable candidate. Only then switch defaults, publish public channels, install from those public channels, test update/rollback, and retain prior stable assets. |

After unit 1, storage/catalog work and plugin ABI design can run in parallel.
After unit 3 defines stable services/display models, terminal, connections and
browser product ports can run in parallel. Route contracts and browser consumers
must converge before deleting either implementation. Keep the public promotion
at unit 9; do not ship a partial Rust application under the normal default.

## CI and release pipeline

Implemented prerequisites in this checkout:

- One tag entrypoint delegates to `rust-release.yml`, which also accepts manual
  requests. The reusable workflow owns one publishing lock. Candidate checkouts
  use the gated commit SHA and recheck the public tag before publication.
- Both capability readiness and source-removal readiness run before release
  builds and again at publication. Existing GitHub releases cannot be overwritten.
  The separate Bun tag publisher has been removed; Bun transition checks remain.
- Nightly uses workspace paths, the pinned toolchain, nextest plus doctests and
  explicit beta/nightly overrides in compatibility jobs.
- Rust PR checks cover the foundations. Native integration CI includes the
  browser/server package suites; the all-file source audit is retained as a CI
  artifact. These are prerequisite checks, not complete product certification.

The final authoritative release DAG still needs the following jobs as their
implementations land. Names here describe required jobs, not existing commands:

```text
candidate SHA + version + capability/source review
  -> Rust lint/nextest/doctests + transition differential contracts
  -> Rust SDK consumer + plugin reload/policy + production browser/MV3/export
  -> paired performance and transcript gates
  -> registry dependency closure + verified archives + detached consumers
  -> build WASM once; assemble all payloads from the same asset manifest
  -> eight target artifacts + signatures/attestations/checksums
  -> installed-artifact host matrix + update/rollback
  -> release readiness aggregate
  -> publish immutable registry/npm/GitHub candidate + installer metadata
  -> public-channel install verification and promotion
```

Make the corresponding PR aggregate checks required in GitHub repository rules
after the jobs exist. Local YAML edits do not change repository protection rules,
prove secret/environment configuration, or certify remote job results. Preserve
the working Bun gates until equivalent Rust/browser tests actually replace them.
Do not use skipped jobs, `--no-verify`, package file listings or `--version`
alone as evidence of installation or feature parity.

## Packaging blockers and public promotion

Resolve the complete publishable dependency graph first: first-party crates must
have registry versions, the patched shell fork needs an owned package identity,
and detached registry consumers must compile without workspace path overrides.
Reproduce and fix the pinned Cargo staging failure (`no hash listed for
harvest-runtime`) with a verified multi-package archive and consumer test. It is
still open; a generated checksum being present did not make Cargo accept it.
The current release workflow publishes only the application crate; extend it to
the complete dependency order and SDK/plugin packages after closure is valid.

Preserve these standalone names and `SHA256SUMS.txt`, plus both executable aliases:

```text
omp-linux-x64             omp-linux-arm64
omp-linux-musl-x64        omp-linux-musl-arm64
omp-darwin-x64            omp-darwin-arm64
omp-windows-x64.exe       omp-windows-arm64.exe
```

Test the **downloaded candidate**, not a different local build, on each matching
host. GNU and musl must run in their intended environments; x64 and arm64 must
execute on matching hosts. Exercise installer paths, source-link/npm/Bun, Cargo,
archives, Homebrew/WinGet, workers, browser serving, protocol stdout, permissions,
checksums, signatures, update and rollback. Installer metadata and npm platform
selection derive from the same validated artifact manifest. Embed the exact
once-built browser outputs into binaries, npm payloads, exports and MV3 packages.

Registry/npm versions are immutable and publishing across services is not an
atomic transaction. Seal and verify all candidate bytes before the first write;
record per-channel publication receipts and digests for safe resumption. Never
rebuild or replace bytes for an already published version. Keep GitHub in draft
until candidate validation is complete, then promote public channels together as
closely as each registry permits. If a public verification fails, stop promotion,
retain the prior stable release and publish a new correction version rather than
overwriting assets. The current workflow still needs this full candidate/draft
promotion and npm/signing implementation before release readiness is achieved.

The user's latest request authorizes work toward public releases. This checkout
cannot yet provide one that preserves the existing product: retained parity is
0/284, source removal is incomplete, and registry/artifact verification is
blocked. No commit, tag, workflow dispatch or publication is part of this audit.
Complete units 0–9 and record their evidence before advancing that release goal.
