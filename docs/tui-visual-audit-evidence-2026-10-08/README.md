# Harvest TUI audit evidence — 2026-10-08

These are **before-change** virtual-terminal captures from the working tree at local HEAD 5c63880 plus active uncommitted repairs. They accompany [the coordinated visual plan](../opencode-tui-visual-consistency-plan.md).

## Mounted production components

The isolated capture harness mounted the real InteractiveMode/Composer and invoked the actual palette and Settings controls. It supplied temporary sessions, in-memory settings, isolated agent directories, and a bundled model definition. It did not submit a prompt, request a provider response, or authenticate an account. Both the VT and process-reported dimensions were 100×30.

| Capture | What it demonstrates |
| --- | --- |
| [settings-default-home.png](./settings-default-home.png), [cells](./settings-default-home.json) | Effective fresh settings selected titanium, despite bare theme initialization having newer defaults. |
| [harvest-home.png](./harvest-home.png), [cells](./harvest-home.json) | Explicit harvest theme with newer Home composition and prompt. |
| [harvest-session.png](./harvest-session.png), [cells](./harvest-session.json) | Mounted conversation containing a real user message; not evidence of assistant streaming or provider/tool execution. |
| [harvest-palette.png](./harvest-palette.png), [cells](./harvest-palette.json) | New palette placement with inherited rounded outline and partial selected-label background. |
| [harvest-settings.png](./harvest-settings.png), [cells](./harvest-settings.json) | Real Settings manager still using the older outline/split-list presentation. |

Successful capture generation is visual evidence only. It does not certify mouse geometry, runtime authority, persistence, or every state.

## Direct component samples

A separate isolated gallery produced 33 renders: 11 families at 100×30 Unicode, 80×10 Unicode, and 24×4 ASCII. Only the representative files below are copied here; [component-metrics.json](./component-metrics.json) preserves measurements for all 33.

| Capture | Evidence type |
| --- | --- |
| [thinking-100x30-unicode.png](./thinking-100x30-unicode.png), [cells](./thinking-100x30-unicode.json) | Actual selector render; full-width legacy outline, no modal surface. |
| [login-manual-100x30-unicode.png](./login-manual-100x30-unicode.png), [cells](./login-manual-100x30-unicode.json) | Actual Login manual-input render; no browser/authentication call. |
| [settings-100x30-unicode.png](./settings-100x30-unicode.png), [cells](./settings-100x30-unicode.json) | Actual Settings component with fresh settings values and harvest paint. |
| [ask-24x4-ascii.png](./ask-24x4-ascii.png), [cells](./ask-24x4-ascii.json) | Direct Ask output larger than the four-row VT. The VT scrolls/clips it; this is not proof of its production overlay interaction. |

Direct component rendering does not use the production compositor, focus owner, or pointer routing. Use it to diagnose layout, then reproduce any unreachable control through the real mounting path.

## Interpretation and reproduction

The existing bench/render-terminal-captures.py rasterizer converts these VT JSON files to PNGs. It uses Consolas and a dark terminal-default background. Missing glyph boxes may be font artifacts; underlying cells distinguish that from an ASCII policy failure. Do not infer light-theme fidelity from default dark cells.

The mounted fixture script is retained at:

    C:/Users/sanid/.codex/visualizations/2026/10/03/01a100b7-e8ce-7682-992c-f4ca1247f42c/tui-audit/capture.ts

The larger direct component gallery and its fixture script were generated at:

    C:/Users/sanid/AppData/Local/Temp/harvest-oct8-ui-audit-78a2608d00834496bfcbe58c3e1c5ec5/

Those paths are local audit artifacts and may disappear. The copied JSON/PNG/metrics files here are the durable evidence. The implementation agent should regenerate evidence against its final tree through maintained isolated test/capture infrastructure rather than depending on these temporary scripts.

Full feature parity, fallback behavior, and runtime integration remain implementation acceptance work defined in the plan.
