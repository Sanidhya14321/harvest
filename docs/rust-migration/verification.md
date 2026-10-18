# Migration verification — 2026-10-12

These are local Linux x64 results for the current migration work. They do not
certify complete application parity, either Rust UI, or a release on other hosts.
The default CLI remains TypeScript. The capability gate reports 18 of 475 entries
with partial or complete Rust implementations and zero with verified cutover
parity.

## Implemented boundaries

- Shared `harvest-terminal` legacy/Kitty keyboard decoding, selector scoring, SGR coalescing, UTF-16 ConPTY
  chunking and carried-width padding checks; N-API consumers use these engines.
- Shared kernel session ownership with the legacy canonical `.lock` identity.
  The Rust session owner resolves missing leaves through jailed ancestors.
  Windows ordinary and extended drive/UNC spellings resolve to one mutex name;
  ordinary absolute paths are accepted beneath canonical Windows jail roots.
  Those Windows-specific contracts compile, but still require execution in CI.
- Cancellable asynchronous HTTP transport for the experimental Rust provider;
  synchronous callbacks stay on the calling thread and can reenter the provider.
- Rust release checksum and installer archive preparation in `xtask`, including
  sorted manifests, all eight standalone names and ZIP payloads with both aliases.
- Rust release-version preparation synchronizes Cargo package versions and exact
  first-party path requirements before lockfile regeneration. CLI fixtures prove
  stable/canary dependency resolution through Cargo's metadata reader.
- CI/release path, platform-smoke and version validation repairs. Rust release
  dispatch uses an existing tag and still requires complete capability parity.

Selector and ConPTY fallbacks were replaced and deleted in the portable-terminal
step documented below. The input-scanner TypeScript fallback remains during
transition. SGR coalescing falls back to unchanged terminal bytes.

## Local results

| Check | Result |
| --- | --- |
| Complete TUI suite, serial execution | 1,475 passed across 86 files |
| Complete sanctioned Rust workspace runner | 2,866 passed, six skipped, followed by the doctest pass |
| Terminal boundary contracts against rebuilt native bindings | 89 passed across 12 files |
| Missing-addon selector/ConPTY contracts | 30 passed across four files |
| TypeScript/Rust writer handoff using actual executables | One passed, 10 assertions |
| Experimental provider, including stalled headers/body and reentrant callbacks | 43 passed with `full`; doctest pass completed |
| Runtime and release-tool contracts, including archives | 24 passed; doctest pass completed |
| Latest runtime, portable terminal and release-tool contracts | 48 passed; one runnable terminal doctest passed |
| Shared release-version drift check and release-version helper tests | Passed; 10 existing TypeScript release tests passed |
| Sanctioned TypeScript workspace runner | All eight package groups passed |
| Release/script unit contracts | 26 passed across five files |
| Shell installer fixtures | Passed |
| Coding-agent, natives and TUI package checks | Passed |
| Repository TypeScript tooling lint/format gate | Passed with two pre-existing unused-symbol warnings |
| Selected Rust crates, all targets/all features, strict Clippy | Passed |
| Root Rust format and Clippy gate | Passed |
| Strict private Rustdoc for migration crates | Passed |
| `harvest-terminal` targeting `wasm32-unknown-unknown` | Passed compilation check |
| Runtime and ownership contracts targeting Windows MSVC | Passed strict Clippy compilation check; execution pending |
| Fresh native build and generated exports | Passed; 122 exports and 11 generated enum fixes |
| Source CLI worker/subprocess smoke probe | Passed |
| Modified workflow YAML and Bash syntax | YAML parsed; 92 Bash steps checked |
| Distribution cutover gate | Correctly rejected incomplete parity |
| Real full-feature Cargo package | Blocked by first-party registry dependency declarations |

The original TUI implementation reproduced two failures: context-sensitive
Unicode padding and a resize fixture that assumed CRLF row placement. Padding
now falls back to measuring the combined row for unsafe prefixes; the resize
fixture also recognizes absolute cursor positioning. The complete suite then
passed without disabling either contract.

The first TypeScript workspace run failed its two-second JSON cursor assertion
under concurrent Rust compilation. Repeating the sanctioned runner with one
package job after compilation passed all groups; the threshold was unchanged.

After the keyboard extraction, a sandboxed TUI run had a debug-socket failure and
a timing failure during parallel Cargo package compilation. Repeating the full
suite with local sockets enabled after compilation passed all 1,475 tests.

The tokenizer reference suite contained 15 stale counts across five text rows.
All 493 rows were independently checked against Python `ctok==1.0.0`; only those
15 recorded values changed. The Rust token-count implementation was unchanged.

## Performance observations

The existing selector benchmark uses a 400-item corpus and eight typing steps,
with 21 repetitions. A locally optimized native build measured warm-session
median 7.6516 ms versus 14.3291 ms for the original TypeScript implementation;
cold filtering measured 1.4104 ms versus 1.7962 ms. Contract score comparisons
matched. These are preliminary single-host measurements, without formal
statistical confidence or cross-platform certification. The final development
addon is unoptimized and is not the build used for that comparison.

The final development-addon committed-history transcript benchmark measured a
5,000-block p95 of 3.5147 ms and a median scaling ratio of 1.012, within the
existing 10 ms/1.3 limits. This measures the current terminal consumer, not a
completed Rust TUI. The earlier optimized-addon sample measured 0.3392 ms/1.003.
Startup, RSS, CPU, browser interaction and plugin overhead remain
unmeasured for the final architecture.

## Revised-scope implementation checks (2026-10-13)

The shared terminal engine now includes bounded CSI/mouse/OSC/DCS/APC/SS3
scanning with UTF-16 event ranges and resume offsets, plus printable Kitty/xterm
text decoding. The transition bridge returns a packed offset array, preserving
unpaired UTF-16 units without allocating and transferring a separate JS string
for every character. The production host keeps its timing, paste dispatch and
focus behavior.

Schema 3 records 284 retained capabilities and 210 intentional retirements across
494 entries. Twenty retained entries have partial/complete implementations; none
has verified full cutover. Retiring JS interfaces never raises the parity count.
Pending interface removal also blocks distribution. Rust SDK/plugin replacement
capabilities and the 17 SDK operations remain retained. Python Eval cannot be
retired by the JS-removal decision. Approved scope and operation mappings are
recorded in `scope.md` and `sdk-mapping.md`.

| Latest check | Result |
| --- | --- |
| Shared terminal and xtask nextest plus doctests | 49 tests passed; terminal doctest passed |
| Input host contracts, including native UTF-16/numeric parity | 122 passed across three files |
| Missing-addon input host contracts in an isolated staged loader | 119 passed across two files |
| Sanctioned native/TUI/collaboration/edit-benchmark runner | All four package groups passed |
| Root Rust format/Clippy gate | Passed |
| Terminal and native package lint/format/types | Passed |
| Terminal `wasm32-unknown-unknown` compilation | Passed |
| Optimized native addon and generated bindings | Passed; 123 exports; original audio declarations preserved |
| Workflow YAML and whitespace validation | Passed |
| Revised distribution cutover gate | Correctly rejected missing retained parity and pending retirements |

The broader native suite exposed two existing fixture mistakes: fallback tests
loaded the actual addon and expected empty fallback search results, and cleanup
fixtures hard-coded version 15 as older than the repository's 0.1 release.
Fallback contracts now explicitly use the fallback factory with typed arguments;
cleanup tests use independent older/current/newer fixture versions. The version
sentinel check now loads the artifact instead of inspecting Rust source text.
The complete sanctioned native suite passed after those corrections.

The WASM primitive job is now a dependency of the aggregate Rust CI result; its
failure can no longer be hidden by a passing aggregate. Rust release parity is
checked from the requested release tag, matching the version/build checkout.
No workflow was dispatched, and no commit, tag or publication was created.

### Input performance decision

Paired baseline/replacement scans alternated order over 21 samples using the
optimized native addon. A fresh 100 KB OSC partial measured medians of 0.2291 ms
(TypeScript) and 0.0885 ms (native bridge); the paired median ratio was 0.385,
with a preliminary 95% bootstrap interval of 0.374–0.393. Fresh 4 KB partial
and complete OSC payloads also improved (paired ratios 0.431 and 0.645).

The same bridge regressed short packets, raw text bursts and resumed long
strings: scanning a 100 KB accumulated string with only a 100-unit new tail
measured a paired ratio of 47.7 because N-API recopies the entire string. Those
consumer cutovers were rejected. The host routes only fresh long string replies
through Rust; small/raw/resumed input and printable text consumers retain their
existing implementation. Native Rust and WASM callers can use the shared engines
without N-API. These observations do not certify the full terminal product or
other platforms.

The existing transcript benchmark measured 5,000-block p95 of 0.4781 ms and a
median scaling ratio of 1.191, within its 10 ms/1.3 targets. Application startup,
RSS, CPU, browser interaction, plugin overhead and installed-host comparisons
remain unverified.

## Portable terminal deletion — 2026-10-13

The authored `packages/tui/src/fuzzy.ts` implementation (441 lines) and the
transitional `fuzzy-compat.ts` and `conpty-compat.ts` algorithms are deleted.
The selector API is now generated binding code; both native-addon and
missing-addon consumers use the shared Rust engines. Generated declarations
preserve transitional import paths and call shapes.

`harvest-terminal-wasm` provides a bounded synchronous memory ABI with
caller-owned prepared-search handles and explicit release/stale-handle errors.
`cargo xtask terminal-wasm` builds one payload, embeds its bytes in generated
bindings and assembles declaration files. It uses the workspace filesystem jail
and atomic replacement; `--check` rejects stale artifacts. No adjacent WASM
fetch is required for a bundled or compiled consumer. Rust CI requires payload
freshness and portable installation contracts on Linux, macOS and Windows;
those remote jobs have not been run from this workspace.

Local production-payload tests passed for scoring fixtures, stable ties,
prepared-handle disposal, embedded NUL, unmatched UTF-16 surrogates, SGR color
boundaries and byte/line chunk boundaries. A real detached bundle and compiled
executable ran without an addon or adjacent WASM file, preserving callback
order, reentrant callbacks, blank/punctuation-query behavior, original item
identity and prepared searches. Terminal and application type checks passed. All four sanctioned native test
package groups passed. The optimized native addon rebuilt with 123 exports and
preserved the existing audio declarations. The selected Rust nextest/doctest
runner, strict selected Clippy, root Rust check, generated-payload freshness
check and shared-version check passed. A real npm archive included both the
embedded payload and generated declarations, with the old selector source
absent. The actual CLI returned `harvest/0.1.0` and its worker/subprocess smoke
probe returned `smoke-test: ok`. Native consumers load the portable binding
module only when it is needed, keeping its initialization off their startup
path.

The initial JSON bridge regressed output chunking and was replaced before
accepting the deletion. Binary requests/results and direct host UTF-16 encoding
avoid that overhead. Paired alternating runs (21 samples, ten operations per
sample) compared the previous TypeScript implementation against the final
optimized WASM payload. The 1,500-candidate selector measured 1.4663 ms vs
2.2089 ms, with a paired median ratio of 0.6844 (preliminary 95% bootstrap
interval 0.6226–0.7238). The dense Unicode output workload measured 0.1713 ms
vs 0.2469 ms, ratio 0.7002 (interval 0.6170–0.7145). These are local warmed
workload results; they do not verify application startup/RSS or other hosts.

The capability manifest records the implementation deletion and installation
path without claiming complete application or cross-platform parity.

## Remaining release and completion gates

- Native CLI/application ownership, the complete Rust terminal product and the
  Leptos/WASM browser applications remain implementation work.
- The Rust SDK and Rust/WASM plugins replacing JS interfaces,
  provider/catalog/OAuth parity, complete persistence and the remaining
  integrations have not been cut over.
- Real Cargo packaging now reaches registry resolution and stops at unpublished
  first-party dependencies, beginning with `harvest-brain`. An owned registry
  closure for the patched `brush-core` dependency is also required. Local staged
  packaging verified the runtime archive, but brain verification hit a pinned
  Cargo 1.99 nightly internal error: no hash listed for the staged runtime package.
  Path installs and archive listings do not satisfy this gate.
- An additional native-addon test-target Clippy probe found seven existing test
  lint errors; the required root library gate passes. Building every optional
  native-addon feature also needs the unavailable PipeWire development library.
- Bazel shipping builds, GNU/musl and all macOS/Windows installed artifacts,
  PowerShell installation, signatures, update/rollback, production browser
  assets and registry-consumer builds still need verification on their hosts.
- The release workflows were edited and locally inspected; no GitHub workflow,
  commit, tag or publication was triggered.

The capability manifest stays conservative. A passing local primitive test
does not make an entire application capability verified.
