# Recent changes review — 2026-09-28

## Scope and confidence

This is a risk-focused review of `HEAD~10..HEAD` on `main` (`51e8f94` at review start): 10 commits touching 188 files. I traced the coding agent, session, Laya, setup, packaging, and related validation paths. This is not a claim that every branch in every changed file has been exhaustively proved correct. Fixes were pushed to `codex/session-manager-laya-review` in three feature-scoped commits (`3585039`, `6ca23c6`, `1c6fca6`).

The review also compared Harvest's existing session features with the [OpenCode session commands](https://opencode.ai/v2/docs/cli/commands/), [TUI session navigation](https://opencode.ai/v2/docs/cli/tui/), and [tab keybindings](https://opencode.ai/v2/docs/cli/keybinds). The [Laya model card](https://huggingface.co/convaiinnovations/laya-typed-decisions) was used to assess the limits of its training and calibration claims.

## Changes made during this review

| Area | Change and reason | Verification |
| --- | --- | --- |
| Laya authentication | The client now reads the sidecar's managed token file and picks up token rotation. Automatic file-token use is restricted to loopback URLs; an explicit token still takes precedence. Previously, normal setup could leave `/v1/decide` returning HTTP 401. | Live local HTTP test, including token rotation, passed. |
| Laya distribution | The npm bundle and binary build now embed the six required Python sidecar files. The agent materializes a versioned copy when it runs outside a source checkout. Previously, a packaged install had no reliable sidecar source directory. | Materialization contract test and npm bundle generation passed. Standalone binary execution remains unverified. |
| Laya setup isolation | Virtual environment creation now fails with a diagnostic instead of installing into the host Python environment; the `--break-system-packages` fallback was removed. | Laya focused tests and coding-agent TypeScript check passed. Fresh-host installation has not been exercised. |
| Session storage | Session breadcrumbs are created with private directory and file permissions, using the existing storage helpers. | Permission test passed. |
| Schema validation | Optional and required object properties now require own-property presence in compiled validators, closing inherited-property acceptance. | Focused omptype tests passed. |
| Tool safety | Edit targets are jailed before native parsing; extension-revised tool input is checked again by pre-read enforcement and Laya gating. | Workspace jail suite passed, including preserved outside-file contents. The extension revision path still needs an end-to-end test. |
| Session navigation | Added `/sessions`, `/timeline`, a visible tab strip, tab cycling/selection, close/reopen, history navigation, and configurable keyboard shortcuts using existing session switching. | Command routing, tab state, controller transition, and strip rendering tests passed. |
| Laya startup and disk check | Interactive startup honors `laya.autostart` for an installed managed environment. Unknown free disk space now stops setup before downloading. | Disk-check tests and TypeScript check passed. Fresh-host autostart remains unverified. |
| Test repairs | Repaired async-job fixture capacity and assertion, a Laya mock type error, and a stale breadcrumb fixture. Removed two Laya tests that duplicated coverage or accidentally launched a real setup. | Relevant focused suites passed. |

The package changelogs contain user-facing entries for the code changes.

## Findings that still need work

### High priority

1. **Coding-tool gate calibration is unproven.** The published Laya typed-decisions model card describes four synthetic workflows (observability, customer service, invoice processing, security incidents), not a coding-agent tool-authorization benchmark. The repository calibration file is a synthetic bootstrap. Do not use its score as evidence for silently approving coding tool calls. Collect labeled coding harness traces and measure false approvals, false denials, timeout fallback, and latency by tool class before changing gate thresholds.

### Medium priority

1. **Extension revision needs integration coverage.** A follow-up check now applies pre-read enforcement and Laya gating again when an extension revises tool input, while retaining any original approval requirement. The revised path still needs an end-to-end extension test that proves the final arguments reach the gate.
2. **Loopback health alone does not prove process identity.** Existing sidecar detection accepts a ready `/health` response. A different listener can claim that port; a later authenticated decision will then fail. Make identity/authentication part of sidecar reuse, without terminating foreign processes.
3. **Cross-system setup is not yet established.** Python bootstrapping is tailored to Windows `winget` and macOS `brew`; Linux supplies instructions. The review did not test clean Windows, macOS, Linux, restricted permissions, offline/cache-only setup, or GPU/CPU combinations. The setup should report unsupported environments accurately.
4. **CPU wheel selection needs measurement.** The CPU torch install uses `--extra-index-url`, which does not alone guarantee a CPU wheel. Validate the actual resolved wheel and disk requirement on supported platforms. Disk-space probe failures now stop the download with a diagnostic.

## Session manager assessment

Harvest already has persisted JSONL sessions, `/resume` with a picker, `/new`, `/fork`, `/rename`, `/pin`, deletion, and CLI resume/continue behavior. This pass added `/sessions` as a picker alias, `/timeline` as a tree navigator alias, and a visible `/tab` strip with switching, back/forward history, close, and reopen controls. Tabs use existing transactional session switching; they are transient and do not keep inactive agent runtimes alive. Cross-restart restoration remains open design work.

## Verification performed

- Coding-agent `check:types`: passed after the changes.
- Focused Laya authentication, payload, and self-healing tests: 20 passed.
- Earlier combined Laya/session focused suite: 58 passed.
- Metaharness targeted suite: 24 passed.
- Decision-sidecar Python unit suite: 38 passed, 1 skipped.
- Omptype own-property/prototype tests: 5 passed.
- Npm bundle generation: passed; generated `dist/cli.js` (~20.62 MB).
- Workspace jail suite: 5 passed after an early path check was added.
- Session tab and command routing tests: passed for cycling, history, close/reopen, failed-switch rollback, the visible strip, `/sessions`, `/timeline`, and `/tab next`.
- Full coding-agent `check`: blocked by formatting differences across 68 files on the latest run. Root Python lint reports 41 issues in `python/omp-rpc`; direct ruff over `decision-sidecar` reports 90 issues. These are not presented as passing gates.
- `git diff --check` on the current working changes: passed. No fresh-host install, standalone binary smoke, full TS suite, or end-to-end Laya model inference was run.

## Suggested next sequence

1. Define supported installation targets and exercise a clean Laya install and autostart on each target.
2. Build a coding-specific labeled Laya harness and retain fail-closed tool gating on sidecar error until measured evidence supports policy changes.
3. Validate the session tab strip in a live TUI on narrow terminals and decide whether tabs should restore after process restart.

## Follow-up from the Windows Terminal screenshot

- The extra window was traced to the Laya sidecar launch using `detached: true` on Windows. It now uses the existing console-aware daemon spawn policy, so a terminal launch shares its console and a console-less launch hides the child. This path is covered by the spawn policy tests; a packaged Windows launch with a real Laya checkpoint still needs a live check.
- A single open session no longer adds a tab row above the prompt. Unnamed tabs display `New session` instead of a timestamped JSONL filename, and the active tab reads the current session title while rendering.
- Switching away from an unsaved startup session now removes its nonexistent file from the open tabs and navigation history. A tab whose file was deleted outside Harvest is removed on navigation while the active session stays in place. Persisted prior sessions remain available.
- Focused session strip and controller tests passed, the daemon spawn policy tests passed, and the coding-agent TypeScript check passed.
