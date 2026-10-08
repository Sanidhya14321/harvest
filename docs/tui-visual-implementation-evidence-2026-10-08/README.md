# TUI verification evidence — 2026-10-08

These artifacts are intermediate evidence, not final-screen or release certification. The maintained harness is `packages/coding-agent/bench/tui-visual-capture.ts`; it mounts eight real InteractiveMode routes with isolated settings, authentication storage and sessions. Its provider/backend data is deterministic fixture data.

## Existing captures

| Set | Routes and sizes | Files | Limits |
| --- | --- | --- | --- |
| Dark | Home, session, palette, settings, models, sessions, tree, agents; 100×30 → 24×4 → 100×30 | 24 VT JSON and 24 PNG | Captured before the harness resize-settle repair and final integration. Tiny views are not reliable final evidence. Only three representative PNGs were inspected. |
| Light | Same routes and sizes | 24 VT JSON | Includes the resize-settle repair; predates final capped dialogs/Composer/management updates. No PNG inspection claimed. |
| ASCII | Same routes and sizes | 24 VT JSON | Includes the resize-settle repair; predates final integration. It does not certify all application-owned glyphs. |

No additional screenshot batch was generated for the final integration. The user's request to minimize screenshot processing takes precedence over the plan's original broad screenshot checklist. Actual VirtualTerminal input/cell tests now prove capped mouse coordinates, full-row selection, tiny margins, focus restoration and resize behavior. Source and test evidence is reconciled in [the coverage ledger](../tui-visual-coverage.md).

## Final integration contract evidence

- `packages/tui/test/overlay-allocation.test.ts`: four passing actual-VT contracts, including local pointer coordinates, outside/stale-click rejection, bottom-clipped rows and one-cell margins.
- Dialog/production selector packet: 111 passing tests across 20 files; centered 60/88/116 mounting, nested child focus, replacement approval ownership, full selection background and locked session resume after resize.
- Composer/Home/status packet: 183 passing tests across 15 suites; five follow-up suites passed 38 tests. Width/height 1–4 retains cursor/draft/chips; restored allocation reveals attachments and bounded errors. Tests restore theme/global state.
- Skill revision cycles: six passing contracts, including visible task/outcome input with full submitted values, inspection reaching the final content line, and dispose cancelling a run without recording results or late redraws.
- Auxiliary surfaces: three passing allocation/state contracts for completed side answers, rule failure/confirmation and cancellable hook loaders.
- Compaction divider and summary: ten passing contracts; the method remains readable before optional decoration at narrow widths.
- Working accent: six passing contracts, including disabling color after a colored render and restoring color without stale cached ANSI.

These packets overlap in some consumer checks; their totals must not be added into an invented count of unique tests. Management and final audit follow-ups are recorded in the execution checkpoint.

## Broader verification and known limits

The TUI package gate passes. Coding-agent types pass in focused checks, but its full gate remains blocked by existing formatting drift outside the UI packet. The integrated UI runner completed 69 chunks covering 344 files: 39 chunks passed and 30 failed. Some stale fixtures were repaired afterward; native Markdown/edit/output, search/tool, Git and platform/input failures remain. This broad run remains a failure until a new complete successful run exists.

The provider review passed 115 mock/local checks and identified custom Codex endpoint/header routing, cancellation and OAuth accounting defects. No live credentials or provider requests were exercised; see [the provider report](../provider-integration-review-2026-10-08.md).

Latency evidence is limited: deferred JS evaluator loading reduced the CLI import probe from 41 modules to 31. Median time changed from 109.5 ms to 106.6 ms, within noise; no full-startup improvement percentage is claimed. Focus changes promote deferred paints while preserving writer backpressure. Explicit setup splash skip opens controls immediately; decorative animation is not a measured provider/backend startup delay.

The earlier smoke harness requested a 45-second timeout but recorded 245,762 ms elapsed with blank output. It awaited stdout EOF during cleanup, so it could outlast its requested watchdog; the exact cause remains unconfirmed. A later isolated import-only probe completed all 14 stages in 14.269 seconds. Neither record proves worker readiness or packaged installation success. Stage-labeled bounded readiness, asset and teardown checks, followed by source/npm/binary install smoke, remain release requirements.

Raw timing/smoke baseline records live outside the repository under `C:/Users/sanid/.codex/visualizations/2026/10/03/01a100b7-e8ce-7682-992c-f4ca1247f42c/tui-implementation-start/`. They are comparison evidence, not a restore/reset instruction. No screenshot in this folder is labeled as final proof for later source changes.
