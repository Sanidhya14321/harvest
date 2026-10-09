# Harvest TUI implementation evidence — 2026-10-09

This checkpoint covers the uncommitted follow-up to externally updated HEAD `bfd36a7`. The previous implementation was preserved. The external agent's reservations were released by the user; there are no outstanding file reservations in this checkpoint. No screenshot batch, commit, push or release was performed.

The OpenCode-style implementation is progressing, but **the complete TUI and release requirements are not yet satisfied**. Use [the coverage ledger](../tui-visual-coverage.md) and [execution checkpoint](../tui-visual-execution-state.md) for the remaining scope. The October 8 captures predate later changes and remain historical evidence.

## Changes in this follow-up

| Area | User-visible behavior | Evidence and limits |
| --- | --- | --- |
| Composer and Cleanse | Inline panels receive their actual height before clipping. Cancel/dismiss controls survive small allocations; log/live/error output wraps within the panel. Completed checker and worker rows follow later theme, ASCII and no-color changes. | Actual Composer frame contracts at 24×4, fullscreen and normal layout; auxiliary allocation and settled-state tests. Full live generation and long error navigation remain unverified. |
| Advisor configuration | Toggle/current/selection markers and navigation hints follow the theme. Resize and theme changes preserve the selected advisor. | Advisor and auxiliary contracts; all custom themes and remapped keys are not certified. |
| Debug choices | Decorative spacer/divider rows no longer prevent a single selector from receiving its real available height. | Auxiliary allocation contract retains selection/navigation under pressure. |
| Tiny-model download and compression preview | Progress, text fallbacks and controls use bounded themed panels, with ASCII alternatives and disposal protection. | Allocation/state contracts; no fresh Kitty graphics capture or live download claim. |
| Setup and extension dashboard | Setup artwork has ASCII/no-color fallbacks and a shorter intro; dashboard navigation uses themed separators and plain key names. | Automatic first setup controls are interactive by 1.1 seconds in the timing contract. Explicit skip remains immediate. Backend initialization and network latency were not measured. |
| Project model settings | New project model-role writes target `.harvest/config.yml`. Existing legacy `.omp/config.yml` remains the write target when it was the selected source. An existing canonical file wins, including an empty mapping. | Reload/CWD/precedence/concurrent-write contracts; no forced migration or deletion of legacy configuration. |
| Terminal text fallback | Truncation preserves OSC-8 hyperlinks, Kitty commands and cursor markers. Unicode segmentation uses one lazy pass instead of repeatedly segmenting suffixes. ASCII ellipses fit one cell; scaled-text clipping stays readable; segment extraction inherits styles and avoids split emoji; custom tab widths are honored. | Nine direct fallback contracts, including a large emoji input; central clipping/extraction and real collab URL rendering checks pass. This fixes clipping, not the whole native fallback. |

Settings, MCP and model-assignment fixtures now use the canonical project directory. Session resume fixtures use real file sessions. Tests explicitly select a color-capable theme when checking different color roles, restore shared settings/theme state, and await external-editor writes before simulated exit. The attachment fixture now supplies the current Home-state contract. These fixture repairs preserve the tested behavior rather than hiding backend failures.

## Verification

| Check | Recorded result |
| --- | --- |
| `bun --cwd=packages/coding-agent run check` | Passed after the final source packet and attachment/task fixture follow-up: lint, formatting and types. |
| `bun --cwd=packages/tui run check` | Passed after the text-fallback change. |
| `bun --cwd=packages/natives run check` | Passed after adding the internal fallback factory declaration. |
| Changed coding-agent contracts before the last two fixture follow-ups | **446 passed, 0 failed**, 19 files, 2,807 assertions, 37.67 seconds. [Raw log](changed-contracts.txt). |
| Final changed-contract packet, including native text fallback | **476 passed, 0 failed**, 22 files, 2,922 assertions, 24.04 seconds. [Raw log](changed-contracts-final.txt). |
| Original attachment/task chunks after fixture follow-up | **64 passed, 1 skipped, 0 failed**, 10 files, 248 assertions. |
| Original collab/copy/debug chunk after hyperlink fix | **30 passed, 0 failed**, 5 files, 110 assertions. |
| Direct JavaScript text fallback | **9 passed, 0 failed** within the focused text run; always tests the fallback even if an addon is installed. |
| Central clipping/extraction plus direct fallback | **28 passed, 0 failed**, 3 files, 75 assertions. |
| Broader central text contracts, including wrapping | **33 passed, 4 failed**, 4 files, 93 assertions. The remaining failures concern wrap background/color/style continuation. [Raw log](text-fallback-contracts.txt). |
| TUI allocation, foundation, editor atoms, image clipping and overlay focus | **30 passed, 0 failed**, 5 files, 190 assertions after the protocol follow-up. |
| Existing native fallback VCS/feature checks | **10 passed, 0 failed**, 2 files, 60 assertions. These discovery/shape checks do not prove dirty Git status, diffs or AST operations. |
| First October 9 integrated UI run | **53 chunks passed, 16 failed**, 69 commands/344 files, 156.5 seconds. [Raw log](ui-suite.txt). |
| Integrated run before the last attachment/task/hyperlink follow-up | **58 chunks passed, 11 failed**, 69 commands/344 files, 161.0 seconds. [Raw log](ui-suite-final.txt). |
| Integrated run after attachment/task/hyperlink follow-up | **61 chunks passed, 8 failed**, 69 commands/344 files, 122.3 seconds. [Raw log](ui-suite-after-text-fallback.txt). |
| Final source-state integrated UI run | **61 chunks passed, 8 failed**, 69 commands/344 files, 124.4 seconds. [Raw log](ui-suite-final-state.txt). |

A failed historical run is never relabeled green because some later focused tests passed. Counts from overlapping packets are not added into a unique test total. The final integrated run still fails in eleven test files, grouped into eight chunks: apply-patch preview; Git sidebar; custom editor file URLs; external plugin scope aliases; apply-patch/glob/invalid-path/think renderers; hyperlinks; and grep path lists/rendering. Exact failures are in its raw log.

## Latency observations

The same local read-only probe clipped `"🙂界".repeat(2000)` to 40 columns using the public native text API:

- Before the fallback fix: 8,000 input cells, 39 output cells, about **909.8 ms**.
- After the fix: the same input/output, about **0.8 ms**.
- After the fix with 400,000 input cells: 39 output cells, about **8.8 ms**.

A later probe while the integrated runner was active recorded about 2.3 ms for the 8,000-cell input and 43.5 ms for the 400,000-cell input. This variation is another reason not to turn these probes into a general speedup percentage.

These are individual micro-probes on this Windows/Bun host, not statistical full-application benchmarks. They identify and remove one concrete rendering stall. The shorter setup intro has its separate first-controls timing contract. Earlier CLI import timing (109.5 → 106.6 ms) remained within noise; no general startup or screen-transition speed percentage is claimed.

## Why release is still blocked

1. **Native/backend availability can make connected views appear empty.** In this checkout the loaded JavaScript VCS fallback returns an empty porcelain status and all-zero status summary even though the checkout is dirty. Diff helpers return empty arrays, and AST search remains stubbed. Capability/version sentinels do not establish operational support. The Git sidebar and edit/search failures must be resolved through the central native/VCS layer, with real dirty-repository and diff/search contracts. Do not add separate Git subprocess logic in each UI component or treat an empty result as success.
2. **A native rebuild is blocked by this host's build tools.** `bun run build:native` was attempted and failed before compilation with `ENOENT` for `C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe`. No addon was installed, and no generated bindings were replaced by that attempt. An operational addon or truthful, tested unsupported states are required; the fallback clipping fix does not provide the missing backend.
3. **Text and tool rendering are not fully green.** Four central wrap style/color continuation contracts still fail; the clipping/extraction/OSC-66 follow-up passes. The integrated log includes edit-preview, Git, search, tool-heading/color expectations, platform file-URL/symlink cases and external plugin resolution. Investigate each failure before changing assertions. Symlink `EPERM` on this Windows host is a test-environment limit, not proof that the feature works elsewhere.
4. **Packaged worker readiness remains unverified.** The earlier smoke watchdog recorded 245,762 ms despite requesting 45 seconds and returned blank output. Import-only success is not worker readiness. Fix the stage labels, bounded pipe draining and teardown before testing source, npm/tarball and compiled installs.
5. **Provider findings remain open.** [The secondary provider review](../provider-integration-review-2026-10-08.md) records Codex endpoint/header routing, device-login cancellation, selected-credential billing classification and workspace identity gaps. It contains 115 passing mock/local tests; no live Claude/Codex/API account request was certified.
6. **Complete visual coverage remains partial.** The original 150-module inventory is mapped, but all states, custom themes, optional backends, terminal protocols and production routes have not passed their full acceptance matrix. No full OpenCode parity or autonomous fine-tuning claim is made.

## Next execution order

1. Restore operational native support or implement honest central fallback capability/error behavior; verify Git status/actions, diff content and search results. Preserve the new clipping contracts.
2. Repair central wrap style/color continuation and reconcile tool-renderer contracts with the agreed semantic surfaces, using explicit color/no-color modes. Preserve the passing clipping/extraction/OSC-66 contracts. Repair platform and plugin-resolution fixtures only where the fixture is the actual cause.
3. Resolve the packaged smoke lifecycle and provider findings with their recorded reproduction contracts. Keep credentials isolated and logs bounded.
4. Finish the still-unverified feature/state rows using real component input and VirtualTerminal cells. Use a few representative captures only if a visual question cannot be resolved from those contracts.
5. Run the affected package gates and the integrated UI runner, then source/npm/binary readiness and installation checks. Report failures plainly. Update this ledger before making any public-release recommendation.

No reliable release date follows from the current evidence.
