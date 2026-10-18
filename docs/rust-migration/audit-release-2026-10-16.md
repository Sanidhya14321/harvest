# Build, installation and public-release audit — 2026-10-16

The Rust migration is not ready for public cutover. The current capability
command reports 20/284 retained capabilities with partial or complete Rust
implementations, 0/284 with verified parity, and 0/210 retired interfaces with
verified removal. The public CLI and npm entrypoints still start TypeScript.
Passing the existing primitive checks cannot certify a replacement application.

This audit inspected the executable workflow, installer, package and release
sources listed below. It did not dispatch GitHub jobs, inspect repository
settings or credentials, publish packages, create tags, or release artifacts.
Offline Cargo results establish local reproducibility; they do not establish
current registry availability or the latest public release state.

## Current pipeline behavior

| Source | Implemented behavior | Remaining release gap |
| --- | --- | --- |
| `.github/workflows/release.yml` | A `v*` tag calls the reusable Rust distribution workflow with the requested tag and inherited secrets. The independent Bun public publisher was removed in this audit; transition checks in `ci.yml` remain. | The default CLI remains TypeScript. The release path deliberately fails closed until both capability parity and complete migration status pass. |
| `.github/workflows/rust-release.yml` | Reusable/manual requests share one publishing lock. Both readiness gates and portable Rust generator freshness run at the requested tag; subsequent checkouts use the gated commit SHA. It validates Cargo/npm versions, rejects existing releases, attempts a real full-feature Cargo package, preserves eight Rust builds/ZIPs/checksums/manifests, publishes `harvest-omp`, creates a GitHub release without overwriting existing assets, and updates Homebrew. | Its build smoke only executes `--version`, uses `OMP_SKIP_INTEGRATIONS=1`, and is not installed-product verification. It publishes only the final crate, not the unpublished dependency closure. There is no Rust npm launcher/package stage, browser/plugin product stage, attestation step, or actual signing step. These remain blocked product work. |
| `.github/workflows/ci.yml` | TS checks, release-script contracts, root Rust checks, TS workspace suites, input/output native contracts, ownership and CLI worker smoke. | The heavier coding-agent/runtime/UI/install suites are not all prerequisites of this workflow. Keep the sanctioned suites until equivalent replacements are exercised. |
| `.github/workflows/rust-ci.yml` | Required aggregate depends on formatting/lint, selected Rust contracts in core/full profiles, installer fixtures, docs, feature powerset, portable terminal assets on three operating systems, cross checks, coverage, package/path install, Homebrew and toolchain jobs. | Cross jobs check four targets, rather than building and executing all eight artifacts. `cargo package --list` plus `cargo install --path` cannot prove registry consumers. Homebrew can skip live installation when no suitable release is found. Repository branch-protection requirements are not observable from YAML alone. |
| `.github/workflows/performance.yml` | Full-feature release builds, version smoke and a 20,000,000-byte size ceiling on five targets. | The name "Release comparison" does not represent paired startup/RSS/CPU/paint/streaming/browser/plugin measurements. It has no baseline/replacement statistics or transcript latency gate. |
| `.github/workflows/nightly.yml` | Repaired in this audit to install nextest, run the sanctioned nextest/doctest runner from the workspace root, include terminal/WASM/xtask packages, and select compatibility toolchains explicitly. | Scheduled and compatibility jobs still need execution on GitHub. A scheduled job cannot substitute for required release checks. |
| `.github/workflows/deep.yml` | Optional/labeled Miri job. | Useful additional diagnostics, not a release gate or a replacement for normal contract coverage. |
| `.github/workflows/security.yml` | Cargo audit/deny, unused dependency tools, secret/static/container scanning, SBOM and CodeQL jobs. | The candidate release must depend on the required results for its exact commit and preserve generated evidence; declaring permissions does not produce a signature or attestation. |
| `.github/workflows/hygiene.yml` | Rust/secret-scanner contracts and repository hygiene checks with aggregate result. | Ensure new product crates and assets remain in explicit scopes as the migration adds them. |
| `.github/workflows/pages.yml` | Publishes `rust-agent/install.sh` and `.ps1` when they change on main. | These installers can advance independently of the product release. Test the publicly served bytes against the candidate manifest and retain a known working previous installer. |
| `.github/workflows/winget.yml` | Submits manifests after a stable release containing `omp-winget-manifests.zip`, or explicit dispatch; verifies its helper binary checksum. | It remains a second publishing path. The generated template currently covers Windows x64 only. Keep submission tied to validated release manifests and add Windows ARM64 coverage. |
| `scripts/release.ts` | Updates package versions, Rust versions, sentinel, lockfiles and changelogs; runs checks; then commits, tags, pushes and watches CI. | It remains authored TS and combines preparation with irreversible release operations. Replace it with reviewable Rust preparation and separate promotion. Existing watch/concurrency comments look for a `v` in the commit subject, whereas the produced subject omits that `v`. |
| `scripts/ci-release-publish.ts` | Has topological legacy workspace/npm publishing, declarations/bin rewrites, platform addon leaves and npm dist-tag handling. | It is not called by the active GitHub release workflow. Its TS SDK/N-API topology is intentionally being replaced and cannot become the Rust distribution launcher by renaming outputs. |
| `scripts/install-tests/run-ci.sh` | Exercises binary, source link, tarball, worker/subprocess and dashboard assets; installs local tarballs through explicit overrides. | Preserve its observable contracts in Rust tooling. Its local overrides do not prove an independent public-registry consumer. |

All the ancillary workflows (PR metadata, labeler, stale and wiki) remain separate
repository administration concerns. They should not publish product assets.

## Artifact and installation contracts

The authoritative Rust matrix preserves these eight standalone names. The
canonical constant already exists at `xtask::checksums::PLATFORM_ASSETS`, but
the workflow matrix, installers, binstall metadata and legacy build scripts still
maintain their own selection logic.

| Rust target | Required public filename | Matching alias ZIP |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `omp-linux-x64` | `omp-linux-x64.zip` |
| `aarch64-unknown-linux-gnu` | `omp-linux-arm64` | `omp-linux-arm64.zip` |
| `x86_64-unknown-linux-musl` | `omp-linux-musl-x64` | `omp-linux-musl-x64.zip` |
| `aarch64-unknown-linux-musl` | `omp-linux-musl-arm64` | `omp-linux-musl-arm64.zip` |
| `x86_64-apple-darwin` | `omp-darwin-x64` | `omp-darwin-x64.zip` |
| `aarch64-apple-darwin` | `omp-darwin-arm64` | `omp-darwin-arm64.zip` |
| `x86_64-pc-windows-msvc` | `omp-windows-x64.exe` | `omp-windows-x64.zip` |
| `aarch64-pc-windows-msvc` | `omp-windows-arm64.exe` | `omp-windows-arm64.zip` |

`cargo xtask package archives` emits deterministic ZIPs containing `omp` and
`harvest` (or both `.exe` names), preserving the raw downloads. Its contract tests
cover exact extracted payloads, permissions, repeatability, failed-producer
preservation and symlink escapes. `cargo xtask checksums --require-platforms`
rejects missing or empty platform files, duplicate basenames, invalid manifest
names and escaped paths before replacing `SHA256SUMS.txt` atomically. This
proves asset-set integrity, not target-machine compatibility.

Both `scripts/install.*` and `rust-agent/install.*` remain production contracts.
The former retain source/Bun installation and explicit-ref behavior; replacing
them wholesale with the Rust installer would remove that method. The Rust Unix
installer chooses musl on Linux, verifies SHA-256, installs both aliases and
initializes completions. PowerShell installers also verify SHA-256. Keep those
methods while changing npm/Bun distribution to the generated Rust launcher.

Cargo-binstall references the eight ZIP names in `rust-agent/Cargo.toml`.
Homebrew consumes raw macOS and Linux musl binaries. WinGet consumes a Windows
x64 ZIP. GNU Linux also has a Debian-package stage. Install/update/rollback
tests must execute these actual products, including the launchers and asset
servers, rather than a separately built workspace binary.

There is a separate update-routing mismatch to resolve before cutover:
`packages/coding-agent/src/cli/update-cli.ts` defaults to `harvest/harvest`,
`harvest/tap/harvest` and `github:harvest/harvest`, while current release URLs
point to `Sanidhya14321/harvest`, and the Rust tap flow installs the `omp` formula
from the repository owner's `homebrew-tap`. The Rust application has no complete
update replacement. The unified manifest must own repository/package/channel
identity, retain existing update metadata behavior, and prove old installed
versions can reach the new release without an environment override.

The npm package is still `@harvest/pi-coding-agent` with both bins pointing to
`src/cli.ts`. It has broad TS SDK exports and TS bundling `prepack` work. The
accepted replacement needs generated startup/platform-selection code, exact
platform payload versions, standalone binaries and a documented Rust SDK/plugin
migration. Legacy JS SDK interfaces must be recorded as removed breaking changes,
not certified as parity. Private browser/metaharness packages currently use
different package versions; select explicit product version ownership when
those assets enter the unified release manifest.

## Registry closure and reproduced checksum failure

`cargo xtask version --check` passes at version `0.1.0`. Cargo metadata confirms
exact requirements for first-party path libraries. That prevents version drift;
it does not make those dependencies available to a detached registry build.

The current full-product dependency order includes:

1. `harvest-runtime`, `pi-walker` and the owned patched shell-core fork.
2. `harvest-brain` (runtime) and `pi-builtins` (walker plus shell core).
3. `pi-shell` (runtime, builtins and shell core).
4. `harvest-omp` (brain, runtime and optional/full shell).

Terminal, browser, SDK and plugin crates add their own closure when the product
actually consumes them. `harvest-terminal-wasm` currently depends on
`harvest-terminal`; this primitive payload is not a complete browser UI.

The shell fork still declares the upstream `brush-core` name/version `0.5.0`.
The root `[patch.crates-io]` makes local workspace builds consume it.
`pi-shell` has a path dependency and `pi-builtins` has registry `^0.5.0`.
A registry consumer does not inherit the workspace patch. Give the fork an
owned package name, preserve its library identity/aliases where useful, update
every dependency/Bazel/vendor reference, and exercise the actual packaged fork
before publishing dependent crates. Publishing only `harvest-omp` cannot solve
the missing first-party closure or preserve the patched shell implementation.

Fresh local reproduction on the pinned Cargo:

```sh
cargo package -p harvest-omp --all-features --locked --allow-dirty --offline
# Fails resolving harvest-brain from the local crates.io index.

cargo package -p harvest-runtime -p harvest-brain --locked --allow-dirty \
  --offline --target-dir /tmp/harvest-release-audit-package
# Packages both crates and verifies runtime, then fails verifying brain:
# no hash listed for harvest-runtime v0.1.0
```

The second failure is an internal error from
`cargo 1.99.0-nightly (c79e8f894 2026-08-04)`. The generated temporary-registry
index and the packaged brain lockfile both contain the same nonempty SHA-256 for
the runtime archive. Repeating without `--locked` still fails. Simply adding a
checksum field or removing `--locked` is therefore not a demonstrated fix.

Resolve this with a validated Cargo/toolchain fix or an explicit isolated
registry staging implementation that serves real archives and checksum-index
entries to detached consumers. Keep verification enabled, the workspace patch
absent from consumers, and package/lockfile reproducibility checked. Do not
substitute `--no-verify`, a package file list, `--path` installation, `[patch]`
overrides or an archive-existence assertion for this gate.

## One authoritative execution graph

The reusable/tag entrypoint consolidation is implemented. The complete graph
below is the remaining product and verification design; it is not all available
today. It carries one candidate commit through public promotion without waiving
a subsystem's parity tests to fit one run.

1. **Rust preparation and manifest.** Extend `xtask` to prepare versions,
   changelogs, generated bindings/launchers, package metadata and the complete
   artifact inventory without committing or publishing. One Rust-owned manifest
   supplies target triples, the eight names, aliases, archives, browser/plugin
   assets, npm package names, crate dependencies, legal files, repository,
   update channels and installer/template data. Reject inconsistent versions,
   stale generated files and unresolved or unowned registry dependencies.
2. **One reusable verification/build graph.** PRs, merge groups and release
   candidates invoke the same workflow using an immutable commit. Rust nextest
   plus doctests, retained TS baselines, runtime/storage parity, terminal traces,
   Wasmtime plugin lifecycle, Leptos/browser/route/middleware contracts,
   security, performance and real packaging are aggregate prerequisites.
   Require each result explicitly; skipped, cancelled or missing required jobs
   cannot produce a promotable candidate.
3. **Build browser assets once.** Build versioned Rust/WASM outputs once,
   record their hashes, and pass those exact bytes to every binary/npm/export/MV3
   assembly. HTML exports embed the WASM bytes for `file://` use. Test 404s,
   WASM MIME type, routes, accessibility, reconnect behavior, local MV3 payload
   and required CSP, listener registration at initial worker evaluation, and
   service-worker termination/revival. Generated startup glue is permitted;
   authored TS browser applications are removed only after their replacements
   pass these tests.
4. **Build immutable platform and registry products.** Produce all eight
   full-feature Rust targets, both aliases, ZIPs, GNU Debian packages, SDK/plugin
   artifacts, generated npm/Bun launcher and platform packages, legal files and
   source archives from the candidate commit. Verify the full crate closure
   through actual archives and a detached registry consumer, including the
   patched shell fork. A build matrix must consume the manifest, not recreate
   eight independent mappings.
5. **Execute the installed artifacts.** On Linux GNU/musl x64/arm64, both macOS
   architectures and both Windows architectures, install actual archives/raw
   binaries/npm packages/Homebrew/WinGet products as applicable in clean roots.
   Exercise aliases, CLI flags, Python/shell execution, worker/subprocess entry,
   RPC/ACP clean stdout, session recovery, dashboards, browser assets, update
   ownership, checksum/signature rejection, failed-update preservation and
   rollback. Compile-only cross jobs and `--version` are insufficient. Preserve
   baseline and candidate performance results from paired runs.
6. **Seal the candidate.** Generate `SHA256SUMS.txt` after final product bytes
   are assembled; sign/attest the final assets, source commit and manifest with
   the selected trusted publishing identity. Verify those signatures and
   attestations from a separate consumer step. Define actual macOS/Windows
   signing requirements and prove the installer/update trust path. Permission
   declarations alone are not evidence. Re-run the complete capability/removal
   gate and installed-artifact aggregate against the same candidate.
7. **Promote once.** After all prerequisites pass, one authoritative release
   workflow creates/verifies the shared release tag, publishes the owned Cargo
   closure in dependency order with registry-availability checks, publishes
   prepared npm archives using the correct stable/canary dist-tag, and creates
   the GitHub release from the sealed artifacts. Advance update metadata and
   Homebrew/WinGet/Pages only from that release manifest. The competing Bun
   publisher is removed; retire the standalone WinGet publishing trigger after
   its retained installation contracts have moved into this graph. Preserve the previous
   public release and immutable candidate evidence for rollback/retries.

Cargo/npm publication is not a transactional multi-registry operation. The
promotion code must record successful immutable package/version publications,
verify existing versions on retry, and keep public update/latest channels from
advancing until the complete release is usable. A partially published dependency
set must not be mistaken for a successful product release.

## Executable local checks and current evidence

Existing commands for the implementation/verification phase:

```sh
cargo xtask version --check
cargo xtask capabilities
cargo xtask capabilities --require-complete
cargo xtask migration status --require-complete
cargo xtask terminal-wasm --check
cargo xtask test -p harvest-runtime -p harvest-brain -p harvest-terminal \
  -p harvest-terminal-wasm -p harvest-omp -p xtask --all-features
bun run check
bun scripts/ci-test-ts.ts all
bun run ci:test:smoke
bun run ci:test:install-methods
```

The complete capability command is expected to reject this unfinished migration.
The full suites, installed products and performance comparisons must run after
the replacements exist; listing these commands is not a claim they passed in
this audit. `ci:test:install-methods` currently tests the legacy installation
topology and rebuilds it. Do not use it as a Rust cutover certificate.

This audit executed the shared version check, capability report, locked offline
Cargo metadata, real package attempts above, nightly YAML parsing, shell syntax
validation and whitespace validation. The nightly workflow now installs
nextest on both legs and uses `cargo xtask test`, which runs nextest followed by
doctests. The compatibility job sets `RUSTUP_TOOLCHAIN` explicitly so the pinned
workspace file cannot silently select the original compiler. Beta/latest
nightly and remote jobs were not executed locally. The previous `/tmp` nextest
binary is absent in this environment; install the sanctioned runner before
claiming a new local nextest result.

The release consolidation passed reusable input/permission validation, job
dependency and immutable revision checks, eight-asset inventory validation,
and syntax checks for all 24 Bash blocks. Eight isolated shell scenarios
exercised release-not-found acceptance, rejection of existing/unauthorized/error
responses and transport failure, plus successful promotion, moved-tag rejection
and readiness failure. Tool stubs wrote only temporary local fixtures; no
network or publishing call occurred. The unchanged Windows PowerShell blocks
still require their target-host execution because PowerShell is unavailable on
this local host. The release gate pins the initial commit and rechecks the remote
tag immediately before registry and GitHub publication. GitHub release creation
fails if a release already exists instead of replacing public assets.

The initial readiness job now also installs the WASM target and requires
`cargo xtask terminal-wasm --check`, so excluded portable terminal bindings must
match their Rust generator before any distribution build. This is one concrete
generator boundary, not global proof of generated provenance. Source counts do
not validate N-API output provenance or every generated binding. Complete
Rust-owned generators and removal of application N-API adapters remain explicit
cutover blockers; no retiring Bun generation step was added to release readiness.
The new release step was checked as YAML and shell syntax, not executed locally
while the native rebuild was in progress.

Public release is blocked by missing product parity/removals, registry closure,
the reproduced Cargo staging error, incomplete Rust npm/browser/plugin products,
installed-artifact/performance/signature verification, and the remaining graph
implementation. Tag and manual release requests now use the same fail-closed
pipeline; no default executable or consumer was switched to Rust by this change.
No public release was attempted or published by this audit.
