# TUI visual implementation checkpoint

Started 2026-10-08 (Asia/Calcutta) at HEAD 5c63880 with existing uncommitted runtime repairs. The user authorized implementing opencode-tui-visual-consistency-plan.md. No commit/reset/stash/clean is authorized or used.

The user confirmed the external repair agent stopped midway because of rate limits and released the shared UI files to this implementation. Preserve its existing changes and validate integrated behavior; no reset or replacement of runtime repairs is authorized.

## Ownership

- Root: settings-schema defaults; theme.ts configured initialization and command entrypoints; workspace-sidebar; documents/coverage/captures; eventual integration in Composer/controllers/palette/management after ownership release.
- /root/tui_foundation_audit: packages/tui/src/components, central scrollbar helpers after announcement; coding-agent theme color/schema/theme-class/tui-adapters; focused tests.
- /root/ui_shell_audit: overlay-box; old selectors/dialogs/managers/extension UI/standalone terminal surfaces; exclude palette/agents-hub/agent-hub/tabstrip/attachments/revisions; focused tests. Announce new file packets.
- /root/transcript_audit: coding-agent/src/tui; transcript/message/tool/manual/notice components; exclude controller/runtime/management/attachment/theme files; focused tests. Controller requests go to root.

## Shared contracts

- Host owns dialog cap/placement and actual per-frame height; renderers must not cap widths independently of pointer geometry.
- Theme adapters expose semantic surfaces and symbol policy; preserve legacy themes, nested styling, cursor/image/link protocols and reference stability.
- Completion endcaps remain at turn tail in completion order, consistently in live and rebuilt views. Tool and prose chronology remains separate and unchanged.
- Existing session authority/lifecycle/archive/deletion/revision/evaluation/attachment repairs are preserved; no runtime rewrite.

## Baseline

Root-owned starting bytes, starting status and diff are saved under C:/Users/sanid/.codex/visualizations/2026/10/03/01a100b7-e8ce-7682-992c-f4ca1247f42c/tui-implementation-start/. This is a comparison snapshot, not a restore/reset instruction. Workers inspect current bytes before patching.

## Progress checkpoint (2026-10-08)

The plan is in progress, not release-certified. The former external reservations were released by the user during this turn. Existing dirty changes remain the implementation baseline.

Implemented and focused-tested: configured Harvest dark/light startup themes; semantic panel/selection backgrounds; color-free/ASCII primitive rendering; narrow editors and actual overlay height allocation; sidebar focus/geometry; transcript rails, output panels and completion-tail ordering; interleaved prose/read groups; dialog/managers/extension surfaces; usage dashboard; plan review/save overlays; deferred JS evaluator import and immediate worker inbox; focus-triggered render scheduling.

Completed subsequent packets: HistorySearch, TreeSelector, SessionInfo and plan allocation/wrapping; setup body/inset, bounded sign-in controls and immediate explicit splash skip; Copy/Rewind/outline; pause/error; authentication/hook/MCP/plugin dialogs; standalone setup/cleanse selectors. Root combined plan/history/tree/setup regression run: 99 pass, 0 fail across 10 files. No further screenshot batches are planned.

Released integration ownership: root owns InteractiveMode, modes/types, event-controller, ui-helpers, SDK read-only review, setup scenes, auxiliary surfaces and documentation. selector_finish owns Composer, Home/welcome, composer preview, footer/status-line and their UI tests. dialog_finish owns selector-controller production mounting and SessionSelector. standalone_finish owns palette, Agents/Activity/revisions, tab strip, attachment presentation and related UI tests. Do not edit another worker's files without handoff.

Verification: the TUI package gate passed. Focused suites passed across foundation, themes, transcript, dialog, plan, sidebar, scheduling and worker-init contracts. The coding-agent full gate is not yet green: existing and concurrent formatting drift remains; the most recent type pass also identified a dialog readonly-array issue being repaired. Some native text-layout/Markdown and edit-preview failures were reproduced at unchanged HEAD 5c63880; record them separately rather than concealing them.

Latency evidence: the CLI import probe loads 31 modules versus 41 before deferring the JS evaluator. Median probe time changed from about 109.5 ms to 106.6 ms, within noise; this is not a measured full-application startup improvement. Focus changes now promote queued adaptive paints while preserving backpressure. No provider/network startup percentage is claimed.

Provider audit is secondary and read-only. See provider-integration-review-2026-10-08.md: 115 mock/local tests passed, with custom Codex endpoint/header routing, cancellation, and OAuth accounting defects found. Live authentication/requests were not tested.

Release blockers: finish integrated screen families, reconcile failing legacy UI/runtime tests, pass the coding-agent gate, resolve provider defects, and validate packaged worker smoke/install paths. A fresh isolated source --smoke-test timed out after 45 seconds with no stdout/stderr; its verified process tree was terminated and the bounded result recorded outside the repo. Do not repeat without investigating that failure. Session ownership/close/archive bundle passed 68/68; broader input/Activity bundles include failing fixture and obsolete-border assumptions and are not green. No release command, commit, reset, stash or push has been performed. A reliable release date cannot be inferred from focused UI tests.
