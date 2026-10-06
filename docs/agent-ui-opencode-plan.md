# Harvest agent UI: OpenCode implementation plan

> **Current plan (2026-10-06, integration owner).** This section supersedes
> older restrictions in this document against changing session management,
> session persistence, and Laya. The user explicitly authorizes completing
> session and agent management, adding narrowly scoped missing model-callable
> management capabilities, improving skills/agent presets through versioning
> and evaluation ("fine-tuning" = versioned iteration, never model weights),
> and removing Laya completely. Older "done" entries are evidence claims to
> verify, not proof of completeness. Everything below this notice remains the
> visual/interaction reference; conflicts resolve in favor of this notice plus
> `docs/agent-ui-execution-state.md` (checkpoint) and
> `docs/agent-ui-feature-coverage.md` (per-feature evidence).
>
> Locked decisions: tab × close (`x` ASCII); close hides view, preserves
> runtime; closed running sessions reachable via Activity with Reopen/Stop;
> Close/Stop/Archive/Delete are separate; last-tab close displays Home; Home
> input never executes against the hidden session; model-created independent
> sessions start in the background without stealing focus; Laya removed from
> runtime/packaging/setup/settings/UI/active docs; approvals, provider
> safety, plan mode, memory retrieval, and unrelated local inference survive.
>
> Owners: LiveSessionRegistry (live runtimes), SessionTabs (visibility +
> history), SessionManager/storage (durable sessions), InteractiveMode
> (presentation/focus/order). One typed facade coordinates them:
> `src/session/session-management-facade.ts` (Create/List/Inspect/Select/
> Rename/Close/Reopen/Stop/Archive/Restore/Delete, stable IDs, serialized
> lifecycle ops, tombstones). One revision service versions managed skills
> and presets: `src/autolearn/revisions.ts` (immutable revs, parents,
> provenance, draft/active, eval records, atomic pointer, 20-retention +
> pinning, expected-revision conflicts). Message extraction for retrieval
> lives in `src/core/harvest/text-extract.ts` (relocated from laya-pruning).
> No second runtime registry, no alternative session engine, no UI-only
> lifecycle, no blanket auto-approval.
>
> Workstreams (exclusive file ownership; shared SDK/registry/schema/event/
> persistence files owned by the integration owner): A. session management +
> UI; B. managed agents/skills, revision/evaluation, model tools; C. Laya
> removal + verification. Workers request shared-file changes as patches.

Prepared: 2026-10-03. Repository: `C:\Users\sanid\Desktop\harvest-2.0\harvest`.
Baseline HEAD: `db336df501d6f36dddf0aa6f30544b6c3d1e3387`; the working tree already contains unrelated uncommitted work.
Concurrent repository activity advanced HEAD to `cca60c0a08244318056d195305784a51c8498a95` during planning. Recheck the current checkout and source before execution; preserve changes made by other work.

## 1. Execution brief

Make the **interactive terminal UI of `packages/coding-agent/`** recognizably close to OpenCode: the screen composition, surface hierarchy, prompt, transcript, tool presentation, sidebar, dialogs, and navigation must work together. A palette change alone does not fulfill this task.

When this document is handed to an agent to implement, execute the milestones in order through final verification in one continuous assignment. Do not stop after a prototype, a theme, or the main screen. Resolve routine implementation choices using the decisions below. Preserve existing agent capabilities and session behavior. Do not commit, publish, release, or comment on GitHub unless separately instructed.

This is a UI redesign, not a runtime/backend rewrite. Retain Bun, `@harvest/pi-tui`, `Composer`, `InteractiveMode`, the existing session model, tool implementations, and extension contracts. Do not introduce OpenTUI/Solid/React into the terminal application. OpenCode source is a design reference, not code to transplant wholesale.

Paths below are relative to the repository root. Proposed filenames are marked **new**; discover and reuse an existing equivalent before adding one. The intended end state and acceptance criteria are fixed; small implementation details may adapt to current source.

## 2. Fixed reference and evidence

Use the OpenCode **terminal** UI, not its desktop/web interface or the unrelated archived Go application. The inspected reference is `anomalyco/opencode`, commit `907b3bc518fa48e90e8ec24dd327d13eee71c36c`, dated 2026-10-03. Pin this commit when comparing source; do not silently chase `dev` during implementation.

Primary references:

- [Home composition](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/routes/home.tsx): centered identity and prompt, restrained surrounding information.
- [Session composition and messages](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/routes/session/index.tsx): scrolling main column, anchored prompt, distinct user/assistant presentation, inline and block tools, reasoning and completion metadata.
- [Prompt](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/component/prompt/index.tsx): filled surface, left accent rail, mode/model metadata, separate hints and busy state.
- [Sidebar](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/routes/session/sidebar.tsx) and [sidebar sections](https://github.com/anomalyco/opencode/tree/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/feature-plugins/sidebar): session context and supporting information.
- [Dialogs](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/ui/dialog.tsx) and [command palette](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/component/command-palette.tsx): bounded panels with search, clear selection, and focus restoration.
- [Reference palette](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/theme/assets/opencode.json), [official TUI documentation](https://opencode.ai/docs/tui/), and [theme documentation](https://opencode.ai/docs/themes/).

Source measurements: home prompt cap 75 columns; session sidebar 42 columns; automatic sidebar when terminal width is **greater than 120**; dialogs use 60/88/116-column caps. These are reference measurements. The responsive and short-screen rules in section 4 are deliberate Harvest adaptations, not claims that OpenCode implements exactly those rules.

Harvest evidence obtained during planning:

- `tui.fullscreen` already defaults to `true`; `Composer` already uses the alternate screen through `TUI.setBaseFullscreen()` and a `TerminalFrameProvider`.
- The current fullscreen home consists of lowercase `harvest` and centered chrome capped at 76 columns. Conversation view is full width with tabs, transcript, and stacked runtime chrome. There is no persistent sidebar.
- `Composer.#renderWorkspace()` omits `#header`, including mounted configuration warnings and startup changelog. Home/session selection currently depends on `transcript.children.length`, so non-conversation notices can change the layout.
- Fullscreen maps the default `band` composer to `rail`; startup status caching can still use raw `band`. Treat this as a consistency bug to fix with the redesign.
- Current status presets combine model/mode/run/Laya/collab/path/VCS/context/cost/title; specialized tool renderers often add multiple frames, sections, and technical metrics.
- Live streaming, restored main transcripts, and parked-agent transcripts have separate construction paths. Changing only one will leave visible inconsistencies.
- Ran `bun test packages/coding-agent/test/session-terminal-navigation.test.ts --test-name-pattern 'switches by SGR mouse' --timeout 30000`: **1 passed, 0 failed, 9 filtered**, including mouse switching and draft restoration. Generated three VT-cell PNGs with `bench/render-terminal-captures.py` and inspected the initial session screen. This is baseline evidence only, not qualification of the proposed redesign. Captures are temporary and must be regenerated by the executor.

## 3. Why the present UI misses the target

| Area | Current implementation | Required result |
| --- | --- | --- |
| Screen hierarchy | Tabs plus full-width transcript and accumulated bottom chrome | Main conversation column, anchored composer, restrained status, responsive right sidebar |
| Home | Small wordmark and centered runtime rows | Intentional Harvest wordmark, centered prompt, a few useful shortcuts, unobtrusive project/status footer |
| Surfaces | Most colors belong to messages/tools/status; no explicit screen/panel/composer/modal surfaces | Coherent background, panel, input, selected, and error hierarchy |
| Composer | Correct rail primitive exists, but labels/status are inherited from old shapes | Filled prompt with mode/model/effort metadata and separate contextual hints |
| Tools | Framed read/edit/bash/eval/search output; different MCP/custom-tool treatment | Compact successful activity, purposeful code/diff/output panels, consistent failures and expansion |
| Metrics | Repeated per-request usage and dense status segments | Quiet context/cost sidebar; detailed metrics available on demand |
| Dialogs | Several replacement-editor and fullscreen flows with varying presentation | Consistent bounded dialogs where appropriate; preserve true fullscreen explorers |
| Interaction | Good existing bindings/tabs, no general command palette or sidebar control | Discoverable actions with remap-aware hints; no stolen existing shortcuts |
| Restoration | Live/rebuild/parked viewers independently assemble blocks | Same presentation and ordering after resume, navigation, and agent focus |
| Verification | Existing VT and gallery infrastructure covers useful parts | Add missing fullscreen layout/state coverage and inspect rendered artifacts |

## 4. Design decisions the executor should use

### 4.1 Screen composition

Keep the existing fullscreen workspace as the default. Preserve the inline mode selected by `tui.fullscreen=false`, including native scrollback history/replay. Do not switch modes automatically based on width.

Use these conceptual layouts; diagrams indicate placement, not literal box borders:

```text
HOME
  [multiple-session tabs only when useful]

                     HARVEST wordmark
                [filled prompt + accent rail]
                [mode · model · effort]
                [send / newline / commands hints]

  project path                                 relevant connection/status

SESSION, WIDE
  [compact session tabs when multiple sessions exist]
  conversation / reading viewport            | session title
  user panel                                  | Context
  assistant prose                             | MCP / LSP
  inline tool rows and output panels          | Todo / Agents
  ...                                         | Workspace Changes
  [queued input / required attention]          | relevant extension status
  [filled composer + mode/model/effort]        | Harvest version
  [busy/error or key hints, project path]      |

SESSION, NARROW
  [tabs when multiple] [compact title/context summary]
  conversation viewport
  [required attention / queued input]
  [filled composer + mode/model/effort]
  [busy/error or hints]
  sidebar available as an overlay
```

- Main column has two columns of horizontal outer padding at normal sizes. Sidebar is 42 columns with two columns of inner padding. Prefer surface boundaries over heavy boxes.
- Automatically dock the sidebar at widths `>120`, provided the remaining main content width remains usable. Hide it automatically on narrower screens. Add persisted `tui.sidebar: auto | show | hide`, default `auto`; `show` on narrow screens requests an overlay instead of crushing the transcript.
- Home prompt maximum width is 75 columns, clamped to available width. Conversation prompt fills the main column. Main transcript is not capped to 75 columns; it uses the space left by the sidebar.
- Display the session title in the wide sidebar. When sidebar is hidden, show it in a compact top context row. Keep multi-session tabs but suppress a redundant tab strip for a single session. Preserve session-picker/new-session access through the palette and existing commands/bindings.
- A sidebar overlay has width `min(42, columns - 2)` at normal narrow sizes; the smallest screens use a full-width bounded panel. Escape closes it and restores the prior focus. Closing must not change the user's stored automatic/docked preference accidentally.
- Keep persisted visibility separate from ephemeral overlay-open/focus state. The layout must not reopen a dismissed narrow overlay on every render; entering narrow `show` requests it once, and subsequent reopening requires an explicit toggle/focus action. In narrow mode the shortcut toggles the temporary overlay; in wide mode it toggles docked visibility through the persisted preference.
- Define all dimensions, breakpoints, and spacing once as named layout constants. Do not spread arithmetic and magic numbers through components.

### 4.2 Height pressure and attention priority

Compute a single layout from `{columns, rows}` and measured content. It must return exactly bounded viewport rows and rectangles used by both painting and hit testing.

Priority under height pressure: **focused required input/approval/question**, editable composer/cursor and autocomplete, essential error/interrupt state, conversation rows, optional header/tabs, sidebar details, decorative home content and secondary hints. No blind `after.slice(-rows)` that removes the active input.

- Reuse `Editor.setMaxHeight()`. At ordinary sizes, grow multiline input to at most roughly one-third of the screen; reserve room for metadata and hints. At very short sizes, reduce padding/metadata/sidebar before reducing editable rows.
- Preserve the existing `24×4` usability contract. Also qualify widths/heights down to `20×4`; render a minimal editable surface without negative widths or a cursor outside the screen. A sidebar toggle at these sizes must still be closable.
- Small optional widgets may be summarized when space is tight. Focused extension custom UI must keep its existing mount/focus behavior. Do not silently discard arbitrary extension widgets or pending approvals to make screenshots cleaner.
- Home/session state must be explicit, based on the displayed conversation/session state. Configuration warnings and command notices are independent attention content, not conversation-state detectors.

### 4.3 Theme, identity, and symbols

Add a built-in Harvest palette family with OpenCode's restrained visual values, while retaining Harvest identity. Proposed defaults: `harvest` for dark and `harvest-light` for light. Preserve explicitly selected user themes and all existing composer shapes.

Target dark palette: screen `#0a0a0a`, panel `#141414`, input/raised surface `#1e1e1e`, main text `#eeeeee`, muted text `#808080`, neutral border `#484848`, primary accent `#fab283`, secondary `#5c9cf5`, reasoning/accent `#9d7cd8`, success `#7fd88f`, error `#e06c75`. Light variants use the reference's separate light values, not inverted dark colors. These values originate in the pinned reference palette above; resolve them centrally through theme JSON.

- Add optional semantic backgrounds such as `screenBg`, `panelBg`, `raisedBg`, `composerBg`, `modalBg`, and any required semantic foregrounds. Extend `ThemeBg`/`ThemeColor`, runtime schema, JSON schema, loader, and adapters together.
- Existing custom themes that omit new tokens must load through central fallbacks to existing colors/terminal defaults. Do not make new fields required or hand-edit every legacy palette.
- Use `theme.bgFill()` and `theme.fgOnBg()` for padded surfaces and nested ANSI content. Preserve 256-color fallback, colorblind diffs, theme changes, and custom terminal backgrounds.
- Use an original Harvest wordmark, not OpenCode's logo. Prefer static text/vector-like terminal glyphs over bitmap assets. Keep home modest; no new splash delays or animations.
- Default interface must work with ordinary monospace fonts. Use existing Unicode/ASCII symbol presets and textual state labels; Nerd glyphs remain optional. Rasterizer font gaps must be distinguished from VT/layout failures.
- The current rail/field primitives contain hard-coded block rail/cap glyphs (`▎`, `▐`, `▌`). Resolve these through the existing symbol presets or provide width-safe ASCII rails/caps. Qualify both runtime and composer preview in ASCII; merely setting `symbolPreset` does not prove these styles obey it.

### 4.4 Composer and status hierarchy

Reuse the existing `rail` composer style and `CustomEditor`. Extend the style's metadata attachment only if necessary. Do not reimplement text editing, cursor movement, undo, paste, attachment placeholders, spelling, autocomplete, external editor, or IME.

- Filled input surface and accent rail; one clear active-mode label.
- Metadata below the editable text: mode, active model, provider when useful, thinking effort. Show shell/Python/voice and agent-focus states accurately.
- One outside hint/status row: active remapped send/newline/commands shortcuts while idle; activity plus interrupt action while running; actionable failure plus retry action after errors.
- Move context/cost/integration detail to the sidebar. Narrow view gets a compact context/cost summary only if it fits. Unknown usage stays unknown, not `0%` or a fabricated amount.
- Avoid displaying the same title/model/path/run state in several neighboring rows. Keep explicit `statusLine.preset`/custom segment preferences functional; present their extra details without forcing the old dense default onto the new shell.
- Keep critical statuses, approvals, collab host/guest ownership, loop/goal state, and extension statuses discoverable. Laya-off and other healthy inactive states need not consume the default hint row.
- Same editor instance and draft survive startup adoption, layout changes, modal close, tab switch, and hot theme changes.

### 4.5 Transcript and tool grammar

Define one visual grammar shared across all transcript construction paths:

| Content | Default presentation | Expanded/detail presentation |
| --- | --- | --- |
| User prompt | Quiet panel with accent rail, readable text, attachment chips | Existing full text/images and links |
| Assistant prose | Open Markdown on screen background; consistent inset and spacing | Existing content, copy and links |
| Reasoning | Compact Thinking/Thought state and elapsed time when available | Existing reasoning trace; respect visibility settings |
| Read/glob/grep/AST search | Muted inline activity with target/count; retain meaningful grouping | Bounded source/search preview and file/line links |
| Write/edit/apply_patch/AST edit | File summary, change counts if actually known, intentional diff panel | Full existing diff/preview/detail behavior |
| Bash/eval | Command/code with bounded live output on a quiet panel | Full permitted output/artifact information |
| MCP/custom/unknown tools | Human label, primary arguments, accurate state; consistent fallback | Existing renderer output or generic structured detail |
| Task/hub/subagent activity | Name/task/state and concise progress | Existing nested details and Agent Hub |
| Error/denial/interruption/unknown outcome | Distinguishable state and useful explanation when activity is shown; required active failure/approval attention stays visible | Bounded full diagnostic details and available actions |
| Completion metadata | Small mode/model/elapsed endcap once per displayed completion | Existing detailed usage available on demand |

- Successful simple tools should normally occupy one or two activity rows, without a rounded frame per operation. Use output/diff/code panels when their content matters. Do not force every specialized renderer into identical rows.
- Reuse and extend `tui/output-block.ts`, `tui/code-cell.ts`, `tools/render-utils.ts`, and existing width helpers. Add a rail/panel variant to the central block abstraction; preserve default behavior for unrelated callers until deliberately migrated.
- Surface result semantics, not transport internals. Move token timings, timeout/artifact metadata, provider/auth/search diagnostics to expanded details where they do not affect an immediate decision. Preserve source URLs, citations, errors, output truncation warnings, and ambiguous-effect warnings.
- Preserve web search's existing complete answer in its collapsed default; simplify its chrome/metadata without silently truncating the answer. Historical tool cards/errors respect tool-activity visibility preferences and remain recoverable by showing activity; required current approval/failure attention uses the independent attention slot.
- Preserve `read.toolResultPreview`, tool visibility/expansion preferences, and per-tool custom renderers. A same-name extension tool must not accidentally receive the built-in tool's summary. Keep generic structured details as fallback.
- Keep reasoning visibility separate from actual model thinking configuration. Avoid model/provider name matching in UI policy; structured facts/catalog policy remain authoritative.
- Preserve chronological order, optimistic submit exactly once, tool-result ownership, mixed assistant text around tools, late result updates, hidden images, attachment links, and protocol blocks.
- Completion endcaps must not make an unfinished block eligible for history retirement, introduce a duplicate final answer, or become stale after an asynchronous tool update.
- Completion ownership is the existing deferred usage/turn boundary, **not each `AssistantMessageComponent` constructor**. `splitAssistantMessageToolTimeline()` clones messages/usage and stamps text segments as stopped, so constructor-based endcaps would repeat after every segment. Aggregate once after tools/post-tool prose; use known `completedAt`/`turnElapsedMs`, omit unavailable values, and do not append a second completion when a late result arrives.
- Newly styled identity/reasoning/cache-marker rows must not mutate above an already published/accepted stable prefix. Preserve `isTranscriptBlockFinalized()` / `getTranscriptBlockVersion()`, immutable render arrays, and frozen task time (`frozen`/`nowMs`) across settlement, backscroll, and history acceptance.

### 4.6 Sidebar data and truthfulness

Use a typed, read-only snapshot of existing state. Do not parse formatted ANSI, read private members through casts, duplicate session state, or query the filesystem/network inside `render()`.

Sidebar order: session title; context/cost; MCP; LSP; active Todo; Agents when present; Workspace Changes when present; relevant extension/runtime statuses; small Harvest/version footer. Empty optional sections are omitted.

- Obtain title from the active `SessionManager`; context, cost, run/usage and cached repository data from a typed snapshot exposed by the status component; MCP/LSP/todo/agent state from their existing controllers/registries.
- Extract shared state acquisition only where needed. One owner for VCS watchers, usage refreshes, and service polling; components consume immutable snapshots and request coalesced renders on change.
- The checkout now uses `@harvest/pi-natives/vcs`, `VcsRepo`, and `utils/active-repo-context.ts`. The `utils/git.ts`/`utils/jj.ts` paths mentioned by older instructions do not exist in this checkout. Extend the actual central/native API if necessary; never hand-spawn Git/JJ from sidebar code.
- Native `VcsRepo` already exposes `changedFiles`, `numstat`, `statusPorcelain`, `statusSummary`, and `diffText`. Verify option types/working-copy semantics in `packages/natives/native/index.d.ts` and existing consumers. Merge staged/unstaged/renamed/untracked states accurately, with capped lists and timeouts.
- Label the list **Workspace Changes**. It represents repository changes, including pre-existing user work; do not claim all changes were made by this session. Binary/untracked files must not get invented line counts. Non-repository, JJ, disconnected and error states remain usable.
- Stale asynchronous results cannot overwrite the newly focused session/worktree. Cancel or generation-check pending refreshes; dispose timers/watchers/subscriptions when ownership ends.
- Use one sidebar scroll viewport independent of the transcript, with collapsible sections. Separate scroll areas for every section are unnecessary. All information/actions remain accessible by keyboard; mouse reporting must not be the only way to inspect a long sidebar.
- Add a palette **Focus sidebar** action: arrows/PageUp/PageDown navigate/scroll, section controls collapse/expand, and Escape restores active input. Reuse `ScrollView`/mouse routing helpers where suitable; translate zero-based `parseSgrMouse()` screen coordinates through final rectangles. Sidebar wheel must not move the transcript, and tabs no longer assume row zero after new header content.
- Use `viewSession`/`focusedAgentId` for focused-agent accounting, `getTodoPhases()` for Todo, and `runDiagnostics.snapshot(sessionId)` for run state. Do not accidentally show the parent session's cost while viewing a child.
- MCP state must update even when `startup.quiet=true`: `#handleMcpConnectionStatusEvent()` currently returns before updating its display sets in quiet mode. Separate state mutation from notification gating or seed from `MCPManager.getAllServerNames()`, `getConnectedServers()`, and `getConnectionStatus(name)`. Retain failure detail from events, and unsubscribe `addConnectionStatusListener()` when disposed. LSP snapshots must likewise update independently of welcome visibility.
- `getCachedContextBreakdown()` alone cannot establish known usage: it currently substitutes zero for unavailable tokens. Carry an explicit known/unknown flag from session accounting with cached totals. Native `vcs.watch()` observes repository-head changes; working-file lists also need mutation/completion invalidation and a bounded shared refresh/manual refresh seam for external edits, rather than assuming head events see every edit.

### 4.7 Dialogs, palette, and navigation

Build common dialog presentation by extending **`modes/components/overlay-box.ts` / `OverlayPanel`** and existing `TUI.showOverlay()`/`OverlayHandle`, preserving settlement and cancellation behavior. Use 60/88/116-column caps where appropriate and clamp to screen dimensions. Opaque styled surfaces are acceptable; do not emulate transparency with unsupported terminal control tricks.

- Model/theme/session/settings/action selectors should share title, search, selected row, body padding, and key-help conventions. Genuine explorers such as Agent Hub/transcript rewind may stay fullscreen.
- Approval/question/plan-review dialogs retain existing policy and ownership. Escape must continue to mean the existing cancellation/close action; redesign must not silently approve or force-kill anything.
- Add a general palette action `app.commands.open` with default **Alt+K**, plus `/commands`; add `app.sidebar.toggle` with default **Alt+Shift+B**, plus `/sidebar`. These have no current default collision in the app/core definitions inspected. **Alt+B is already word-left in the editor and must remain so.** Verify registries and user-remap precedence before wiring new actions; reserve command names through the existing builtin-resolution rules.
- Populate the palette from the **TUI** state assembled by `buildTuiBuiltinSlashCommands()` and `InteractiveMode.refreshSlashCommandState()`, including extensions, custom/MCP prompts, enabled skills, file commands, and templates. `buildAvailableSlashCommands()` in `available-commands.ts` is ACP-filtered and omits TUI-only commands; it cannot be the sole palette source. Do not duplicate a manual list or execute strings through the shell. Actions with arguments should prepare the draft or open the existing selector, not execute an incomplete command.
- Preserve **Ctrl+P** model cycling, **Ctrl+T** reasoning visibility, **Shift+Tab** mode cycling, **Ctrl+L** live voice, **Ctrl+Q/Ctrl+Enter** follow-up, **F5/Alt+R** retry, session-tab bindings, and user remaps. Do not copy OpenCode chords wholesale.
- Derive displayed key hints from the active `KeybindingsManager`. Modal closure restores the correct previous focus and draft. Invisible or clipped tabs/items must have no mouse targets.
- Preserve `SelectorController.focusActiveEditorArea()` behavior: an approval/question may occupy `editorContainer` even when `ui.hasOverlay()` is false. Palette/sidebar actions must not dispatch behind that surface, and closure must focus the currently mounted input rather than a stale base editor. Keep nested extension asks and collaboration first-answer-wins settlement intact.
- Remove height-overflow assumptions in Settings/ModelHub/AgentsHub (current minimum heights/content rows can exceed a short terminal). Supply the actual viewport budget, preserve selected-row visibility, and collapse internal sidebars before clipping required controls.

## 5. Architecture and implementation boundaries

### Existing owners to retain

| Owner | Responsibility after redesign |
| --- | --- |
| `modes/composer.ts` | Startup terminal ownership, home/session layout, bounded frame composition, editor continuity |
| `modes/interactive-mode.ts` | Session-aware wiring, UI lifecycle, adoption, state/command integration |
| `modes/types.ts` | Explicit context/contracts shared by controllers |
| `modes/components/transcript-container.ts` | Semantic block ledger, finality/versioning, bounded tail/history window |
| `modes/controllers/event-controller.ts` | Live message/tool state and chronological display updates |
| `modes/utils/ui-helpers.ts` | Main transcript reconstruction and queued input presentation |
| `modes/components/chat-transcript-builder.ts` | Parked/advisor/collab transcript construction |
| `modes/components/status-line/*` | Shared cached usage/VCS/runtime state plus compatibility status rendering |
| `modes/controllers/selector-controller.ts`, `extension-ui-controller.ts` | Existing selector/custom-UI lifecycle and promise settlement |
| `packages/tui/src/tui.ts`, `terminal.ts` | Terminal writes, protocol/input/focus/overlay/cursor/restoration |

### Minimal new product abstractions

Proposed **new** modules: `modes/workspace-layout.ts` for geometry and clipping; `modes/components/workspace-sidebar.ts` for sidebar rendering/navigation; `modes/components/command-palette.ts` for palette presentation. Extend `overlay-box.ts` for dialog chrome; do not add a parallel border system. Keep theme values and layout constants centralized.

Replace the fullscreen layout's inference from anonymous runtime children with a typed named-slot contract: transcript, tabs, attention/notices, above/below-editor widgets, composer, metadata/hints, sidebar, focused replacement UI. Retain compatible existing mounts for inline mode and extension APIs. `Container` is vertical concatenation, not a split-pane engine.

Render pane row arrays at their **actual allocated widths**, pad/fill using ANSI-aware helpers, and combine them into one `TerminalFramePlan.viewport`. Pass graphics/image constraints into components before rendering; never blindly slice raw image protocol rows. Preserve cursor markers until the terminal engine extracts them. The same geometry computes screen-space mouse routing.

Keep fullscreen and inline history contracts distinct. Fullscreen publishes bounded viewport rows without native history batches or ED3; inline keeps monotonic acknowledged history IDs and replay/resize behavior. Do not alter session persistence/export to store rendered rows.

Scrollback needs a stable reading anchor: when streaming while scrolled up, preserve the viewed semantic block and its intra-block position instead of letting a bottom-relative offset drift with appended rows. Recompute that anchor on rewrap/sidebar resize; returning to tail resets follow mode. Store/restores it per displayed session through existing view-state mechanisms. Do not rebuild the entire transcript on every animation frame.

## 6. Ordered implementation milestones

### M0 — Preserve baseline and establish reproducible fixtures

1. Read root `AGENTS.md`, relevant docs, and the file map below. Record `git status --short` and existing overlapping changes before editing; preserve them.
2. Inspect current settings without changing the user's persistent config. Verify Bun/native dependencies; run focused baseline UI tests and record existing failures separately.
3. Reuse `cli/gallery-fixtures/*`, `gallery-cli.ts`, VT tests, and the capture helper. Fix clock/theme/model/symbol/VCS values in isolated fixtures. Do not use the user's real session as a screenshot fixture.
4. Capture home, normal session, streaming, tools, dialogs, and width/height extremes before changes. Store evidence in a task output/temp directory with a manifest recording viewport/theme/font/state.

**Exit:** baseline findings, fixtures, and captures exist; failures are classified. Do not treat a baseline failure as permission to delete coverage or skip final verification.

### M1 — Theme surfaces and one geometry contract

1. Extend theme schema/loader/unions/adapters with optional surface tokens and fallbacks. Add the two Harvest default palettes and wire unset defaults through settings, synchronous startup theme loading, and the theme selector.
2. Implement product layout rectangles/constants, row budgets, clipping, and shared hit-test geometry. Preserve glyph/ANSI/grapheme correctness.
3. Add contract tests for pane containment, width/height pressure, background reset handling, and old custom-theme loading. Do not test literal default values just because they were added.

**Exit:** old themes/shapes still load; new surfaces have correct foreground contrast; geometry is safe at boundary sizes.

### M2 — Composer/home/session shell and startup consistency

1. Wire named mounts through `Composer` and `InteractiveMode`. Keep the speculative first paint lightweight; do not add expensive session/provider/VCS imports to the startup module graph.
2. Replace the minimal fullscreen home with the planned home composition; preserve quiet startup behavior and deferred keystroke replay.
3. Add responsive session composition, single/multi-session title placement, clamped composer growth, metadata, hints, and required attention placement.
4. Render configuration warnings/changelog/notices intentionally in fullscreen. Preserve their quiet-startup/settings semantics; warnings must not vanish just because the view is home.
5. Centralize effective composer-shape resolution for first paint, adoption, previews, runtime changes, and cache serialization. Version obsolete cached chrome and accept missing new preference fields safely.
6. Keep startup caches limited to valid UI preferences/placeholders; do not persist sidebar accounting, active run status, or transient errors. Preserve setup splash skip/disposal and one-time `ComposerLease` adoption.

**Exit:** first paint and session-adopted view match; no draft loss/flicker/remount; notices are reachable; short-screen input remains usable; inline history tests remain green.

### M3 — Shared snapshots and sidebar

1. Expose a typed status snapshot from existing cached state; supply session/MCP/LSP/todo/agent/extension snapshots through existing owners.
   Refactor quiet-startup MCP state acquisition before using it as sidebar input, and qualify quiet mode explicitly.
2. Implement docked and overlay sidebar, collapsible sections, independent scrolling, keyboard navigation, and context/cost truthfulness.
3. Add Workspace Changes using the native VCS boundary and existing lifecycle patterns; cap work/output, reject stale generations, dispose resources.
4. Wire `tui.sidebar` settings and startup cache compatibility. Narrow overlay closure must restore focus.

**Exit:** current session data is accurate; no duplicate polling; old repository results cannot leak into a switched tab; unavailable services/repos do not freeze input.

### M4 — Transcript, tools, and completion hierarchy

1. Introduce central inline activity and rail/panel variants by extending existing render helpers. Preserve frame markers/inline flags so custom outputs do not get double backgrounds/padding.
2. Migrate user/assistant/reasoning/completion metadata and usage-row presentation, preserving images/chips/copy/OSC133 zones.
3. Review grouped reads and specialized tool families in batches: filesystem/search; shell/eval; edit/diffs; MCP/custom/generic; task/hub; LSP/debug; web/GitHub/memory/todo/ask/resolve/other renderers. Prefer shared wrapper/panel changes; edit a specialized renderer only where its actual output conflicts with the grammar. Do not mechanically rewrite every tool or erase useful specialized presentation.
4. For each batch verify pending partial args, executing, success, error, denied/interrupted, collapsed/expanded, and narrow rendering through the existing gallery fixtures and real component paths.
5. Apply the same display decisions to `EventController`, `UiHelpers`, and `ChatTranscriptBuilder`. Preserve the full semantic transcript used by exports/`harvest render`.
6. Recompute affected geometry budgets centrally: `outputBlockContentWidth`, edit header fitting, Eval preview windows, and task assignment frame inset. Keep edit-preview spinners on the mutable trailing rows and task elapsed values frozen after settlement/history acceptance.
7. Migrate manual `!`/`!!` Bash and Eval executions through `execution-shared.ts`, including PTY viewport insets. Migrate custom/hook/skill/collab/summary/diagnostic message surfaces outside `ToolExecutionComponent`; preserve custom-renderer priority, author attribution, visibility, and expansion.

**Exit:** compact tools and purposeful output panels are consistent; partial streamed arguments remain visible; live/restored/parked paths match semantically and visually; no output/order/ownership regression.

### M5 — Dialog presentation and discoverable actions

1. Unify compatible selector/approval/question/plan-review presentation on existing overlay/list contracts; preserve genuine fullscreen explorers.
2. Implement the registry-backed palette and sidebar commands/bindings. Route execution through existing actions/commands, preserving prompt argument entry and restrictions.
3. Update autocomplete placement to actual composer pane geometry; translate mouse input through final clipped rectangles.
4. Update setup/theme/composer previews to render the same real components/styles as runtime. Do not build independent visual mocks.

**Exit:** every visible action works by keyboard; existing shortcuts/remaps still work; closing/canceling nested UI settles once and restores input/draft.

### M6 — Scroll, resize, cursor, and performance qualification

1. Complete per-session reading anchors and sticky-follow behavior while streaming, switching tabs, resizing, and showing/hiding sidebar.
2. Validate fullscreen alternate-screen entry/exit, overlay composition, image clipping/budget, and hardware/software cursor locations. If cursor handling changes, include its update inside synchronized output; avoid a separate unsynchronized write.
3. Preserve render-array caching, finalized prefixes, block versions, incremental Markdown/reveal, adaptive rendering/backpressure, and input priority. No synchronous discovery or VCS work in render loops.
4. Benchmark the actual fullscreen windowed path with a live tail and long history. The existing `transcript-compose.bench.ts` does not commit history merely because its comments say so; adjust/add the relevant scenario without unrelated benchmark refactors.

**Exit:** reading position stays stable, tail return works, cursor/hit maps track geometry, no terminal corruption, and repeated rendering stays bounded by displayed work.

### M7 — Final visual review, documentation, and handoff

1. Run final checks below; repair regressions caused by this work. Do not broaden into unrelated local backend fixes.
2. Regenerate all required captures and inspect actual PNGs at matching sizes/theme/font. Compare against the fixed reference and this spec for spacing, surfaces, density, prompt/sidebar/title placement, and clipping. Iterate until all required screens agree with the target.
3. Update user docs and per-package `[Unreleased]` changelog for affected packages. Correct stale fullscreen/theme documentation only in areas touched by this change.
4. Deliver changed-file list, behavior changes, command/scenario results, before/after captures, performance evidence, and any genuinely unverified terminal/platform constraints. No unsupported claim of zero bugs or complete cross-platform parity.

**Exit:** section 9 is fully satisfied. All required milestones are part of the same assignment.

## 7. File map and audit coverage

This map records the UI paths reviewed during planning. Large tool/controller modules were inspected through their UI/rendering branches and contracts; unrelated execution/provider/math internals are dependencies, not redesign targets. Recheck current files before implementation because the working tree is active. This is not a claim that every backend file or every test in the repository was read.

### Product shell and UI lifecycle

- Under `packages/coding-agent/`: `src/main.ts`, `src/cli.ts`, `src/modes/composer.ts`, `src/modes/startup-composer.ts`, `src/modes/composer-cache.ts`, `src/modes/interactive-mode.ts`, `src/modes/types.ts`, `src/modes/shared.ts`, `src/modes/agent-mode.ts`, `src/modes/run-diagnostics.ts`, `src/modes/queue-input.ts`.
- `src/modes/controllers/input-controller.ts`, `selector-controller.ts`, `extension-ui-controller.ts`, event/args/reveal controllers; session-tab methods and the live-session/view-state owners referenced by these controllers.
- `src/modes/components/custom-editor.ts`, `welcome.ts`, `footer.ts`, `overlay-box.ts`, `session-tab-strip.ts`, `keybinding-hints.ts`, `editor-top-gap.ts`, model/theme/settings/session/tree/copy/history/hook selectors, ask and plan-review dialogs, editor replacements, queued-message/status/HUD components, Agent Hub/dashboard/transcript viewers. `footer.ts` is legacy and is not the live mounted status owner.
- `src/modes/components/status-line/`: `component.ts`, `types.ts`, `presets.ts`, `segments.ts`, `separators.ts`, `git-utils.ts`, and barrel.
- Setup splash (`src/modes/setup-wizard/startup-splash.ts`) and composer/theme scene integration (`src/modes/setup-wizard/scenes/composer.ts`, `scenes/theme.ts`); `src/modes/components/composer-shape-preview.ts` and `composer-shape-registry.ts`. Same-style previews must stay in sync.
- `src/config/settings-schema.ts` UI groups and `src/config/keybindings.ts`; `src/slash-commands/types.ts`, `builtin-registry.ts`, `available-commands.ts`, and UI command/action integration branches.
- `src/session/session-tabs.ts`, `session-view-state.ts`, `live-session-registry.ts`, `session-manager.ts` public title/state APIs, and `src/utils/active-repo-context.ts`.

### Transcript and specialized rendering

- `src/modes/components/user-message.ts`, `assistant-message.ts`, `tool-execution.ts`, `read-tool-group.ts`, `usage-row.ts`, `transcript-container.ts`, `chat-transcript-builder.ts`, `transcript-outline.ts`, `visual-truncate.ts`, and relevant bash/eval/custom/artifact message components.
- Other independent transcript surfaces under `src/modes/components/`: `custom-message.ts`, `hook-message.ts`, `message-frame.ts`, `skill-message.ts`, `compaction-summary-message.ts`, `collab-prompt-message.ts`, `late-diagnostics-message.ts`, `background-tan-message.ts`, `advisor-message.ts`, `execution-shared.ts`, `bash-execution.ts`, `eval-execution.ts`. These must not retain conflicting chrome merely because they bypass the agent-tool renderer.
- `src/modes/controllers/event-controller.ts`, `tool-args-reveal.ts`, `streaming-reveal.ts`; `src/modes/utils/ui-helpers.ts`, `transcript-render-helpers.ts`, `interactive-context-helpers.ts`.
- `src/tools/renderers.ts`, `render-utils.ts`, `default-renderer.ts`, `read-renderer.ts`, `eval-render.ts`, `gh-renderer.ts`, `memory-render.ts`, and renderer branches in bash/write/glob/grep/AST/todo/ask/resolve/hub/vibe/think/other registered tools.
- `src/edit/renderer.ts`, `src/task/render.ts`, `renderer.ts`, `src/mcp/render.ts`, `src/lsp/render.ts`, `src/web/search/render.ts`, and debug/execution renderer helpers referenced by the registry.

### Core TUI and themes

- `packages/tui/src/tui.ts`, `terminal.ts`, `utils.ts`, `stdin-buffer.ts`, `keys.ts`, `keybindings.ts`, `mouse.ts`, `bracketed-paste.ts`, `editor-component.ts`, `symbols.ts`, `index.ts`.
- `packages/tui/src/components/composer/*`; editor/input/Markdown/image/box/text/truncated-text/spacer/loader/cancellable-loader/scroll-view/select-list/settings-list/tab-bar public contracts and rendering paths.
- Supporting TUI capability/image/scheduling/restoration/multiplexer modules and their exported contracts. Math parser internals are outside redesign scope.
- `packages/coding-agent/src/tui/*`: types, utilities, width-aware-text, hyperlinks, output-block, code-cell, status-line, file-list, tree-list, barrel.
- `packages/coding-agent/src/modes/theme/`: `schema.ts`, `theme-schema.json`, `color.ts`, `loader.ts`, `theme-class.ts`, `theme.ts`, `tui-adapters.ts`, symbols/shimmer/Mermaid helpers, and default palette JSONs. The palette directory was read/parsed for compatibility, not selected for bulk edits.

### Existing verification and previews

- `src/cli/gallery-cli.ts`, `gallery-screenshot.ts`, `gallery-fixtures/*`, `src/commands/gallery.ts`, `src/cli/render-cli.ts`, `src/commands/render.ts`.
- `test/session-terminal-navigation.test.ts`, `interactive-terminal-e2e.test.ts`, `modes/workspace-scroll.test.ts`, startup/composer/status/theme/editor/selector/transcript/tool-renderer regression families listed below.
- `packages/tui/test/virtual-terminal.ts`, virtual/stress render schedulers, process-terminal harness, history-frame/overlay/input/editor/theme/image/width tests.
- `bench/render-terminal-captures.py`, `rendering.ts`, `transcript-compose.bench.ts`, streaming throughput and TUI Markdown/layout benchmarks; package scripts and `scripts/ci-test-ts.ts`.
- Docs: `packages/coding-agent/DEVELOPMENT.md`; `docs/tui.md`, `tui-core-renderer.md`, `tui-runtime-internals.md`, `theme.md`, `settings.md`, `keybindings.md`, `agent-hub.md`, session-switching/navigation docs, tool/custom-renderer/extension/approval contracts, and `docs/product-decisions.md`.

Do not assume prose is up to date: executable source already contradicts older descriptions of inline-only rendering, theme schema location, and some keybindings.

## 8. Verification checklist and commands

### Required observable contracts

| Scenario | Must be demonstrated |
| --- | --- |
| Empty startup → adopted session | Same draft/editor, correct palette/shape, prompt usable during startup, notices visible |
| Submit before first model token | Exactly one visible user prompt, immediate screen update |
| Prose → tool A → prose → tool B → final | Chronological content retained in live, rebuilt, and parked transcripts |
| Partial Bash/edit arguments | Previews reveal latest streamed content; approval still waits for authoritative args/preview |
| Tool success/error/denied/interrupted | Accurate state; no success styling for unknown/failed effects; useful detail remains accessible |
| Scroll up during streaming → return to bottom | Reading anchor stable; tail return shows newest output |
| Resize / sidebar show-hide | No duplicated blocks, stale targets, broken wrapping, displaced cursor, or draft loss |
| Session switch, including failed/rapid loads | Per-session draft/scroll/state retained; old async refresh cannot repaint new session |
| Model/theme/settings/palette/approval/ask/plan UI | Search/navigation work; cancel/close settles once and restores correct focus/draft |
| Approval arrives while palette/sidebar owns focus | Close temporary UI and answer the currently mounted approval; no dispatch into a stale editor |
| Multiline/large paste + completion menu | Editable text/cursor/menu stay inside available space; attachment expansion survives submission |
| Extensions/MCP/custom renderer | Existing replacement/overlay/widget/status contracts and same-name ownership preserved |
| CJK/combining text/tabs/ANSI/long paths | Width-safe lines, sanitized content, shortened home paths, valid hyperlinks |
| Images with sidebar and expansion | Graphics constrained to pane/viewport, budget honored, text fallback survives replay |
| Base fullscreen → overlay → close → exit | Alternate buffer/protocol/cursor restored correctly; no fullscreen native-history output |
| `tui.fullscreen=false` | Existing scrollback retirement, resize replay, shell return behavior still passes |
| Dirty Git/JJ/non-repository/offline services | Truthful bounded sidebar; UI stays interactive |
| Quiet startup with MCP/LSP transitions | Connection/failure state updates without welcome/status chatter |

Use this representative screenshot matrix, **not a Cartesian product** of every size/state/theme. Add cases only when a distinct branch or discovered failure needs evidence. Use fixtures rather than paid provider calls.

| Viewport / appearance | Required capture coverage |
| --- | --- |
| `100×30`, dark/Unicode | Home, idle session, streaming reasoning/tools, expanded diff/shell, palette/model/settings, question/approval, multiple sessions, error/retry |
| `160×45`, dark/Unicode | Docked sidebar with populated context/integration/Todo/changes and a long transcript |
| `121×32` and `120×32`, dark/Unicode | Same session across dock/hidden breakpoint; no content/cursor/hit-target corruption |
| `80×24`, dark/Unicode | Narrow transcript, sidebar overlay open and closed |
| `60×16`, dark/Unicode | Multiline composer/autocomplete and a short selector with visible active controls |
| `24×4`, dark/ASCII | Minimal editable composer and required-attention flow; also test `20×4` bounds without requiring another redundant screenshot |
| `100×30`, light/Unicode | Session with tools/sidebar and a dialog, proving surface contrast |
| `80×24`, dark/ASCII | Home/composer and inline/block tool output, proving rail/cap fallback |
| `100×30`, older custom theme | Session and dialog proving missing-surface-token fallback |

### Existing tests to extend or preserve

- Shell/navigation: `startup-composer.test.ts`, `composer-cache.test.ts`, `session-terminal-navigation.test.ts`, `interactive-terminal-e2e.test.ts`, `modes/workspace-scroll.test.ts`, session-tab-strip and session-tab controller tests.
- Input/state: input-controller keybindings/focused-submit/large-paste/image/compaction/escape/suspend families; interactive mode editor/status/plan-review/mode/model/working-accent tests.
- Transcript: `event-controller-mixed-assistant-render.test.ts`; `modes/controllers/event-controller-read-grouping.test.ts`, `event-controller-args-reveal.test.ts`; `modes/utils/render-initial-messages.test.ts`; transcript-container/version/late-tool/rebuild/ordering regressions.
- Tools: `modes/components/tool-execution.test.ts`; memoization/args/preview/custom/write repaint tests; `tools/edit-renderer.test.ts`, `tools/read-renderer.test.ts`, `mcp-render-status.test.ts`, task nested-live/render-call and specialized renderer contracts.
- Streaming/late settlement: `streaming-preview-height.test.ts`, `tool-execution-write-repaint.test.ts`, `modes/components/tool-execution-background-task.test.ts`. Qualify final VT output for long-preview-to-result repaint, absence of preview spray, and background completion while a tool is historical/offscreen.
- Task/default-density changes: `task/render-call.test.ts` currently requires the full brief even collapsed. If adopting a concise default, replace that visual expectation with proof that the complete assignment remains accessible expanded and that call/progress/result ordering and single-card result suppression still hold. Preserve Ask timeout auto-choice disclosure, Resolve apply/reject/failure distinctions, and nested-task cycle/depth bounds.
- Theme/status: old custom-theme parsing, live epoch/contrast/256-color, symbol presets, overflow, usage/cache/disposal/VCS-generation tests.
- Core: `packages/tui/test/history-frame-plan.test.ts` already checks base fullscreen entry/exit and changed-row painting; preserve overlay focus, restoration, editor/IME, input priority, image clipping, tight layout, width and scheduler/backpressure tests.

Add tests only for a named observable failure mode. Update intentional visual expectations while retaining ordering/output/state/ownership contracts. No `mock.module()`, file-wide global mutation, implementation-source grep, tests of static boilerplate, or duplicated mock-only coverage.

### Commands

Run focused changed-area files first, then appropriate buckets. From repository root:

```powershell
bun --cwd=packages/coding-agent run check
bun --cwd=packages/tui run check
bun test packages/coding-agent/test/session-terminal-navigation.test.ts
bun test packages/coding-agent/test/interactive-terminal-e2e.test.ts
bun test packages/coding-agent/test/modes/workspace-scroll.test.ts
bun test packages/tui/test/history-frame-plan.test.ts
bun scripts/ci-test-ts.ts coding-agent-ui
```

Transcript/tool changes also require the focused renderer tests and the relevant `coding-agent-runtime` / `coding-agent-native` buckets where their tests are partitioned. A UI bucket alone does not cover every rendering regression. If shared pi-tui contracts change, run its package test suite and affected consumer checks; broaden to `bun run check:ts` when the shared boundary changes warrant it. Do not invoke `tsc`, bare root `bun test`, or direct `cargo test`.

Existing gallery commands (presentation inspection; not pass/fail gates):

```powershell
bun packages/coding-agent/src/cli.ts gallery --surface tool --width 100 --plain
bun packages/coding-agent/src/cli.ts gallery --surface composer --width 80 --plain
bun packages/coding-agent/src/cli.ts gallery --tool bash --state streaming --state success --width 80
bun packages/coding-agent/src/cli.ts gallery --tool edit --expanded --width 100
```

The gallery catches renderer errors and can print `render failed` while completing the command. Inspect returned sections/output; exit code alone proves nothing. `gallery --screenshot` uses VHS and an installed font, and may be unavailable on Windows. Reuse real VT-cell captures for screen composition; these test actual painted cells and already rasterize on this host. Resolve the bundled Python/Pillow executable through `load_workspace_dependencies` when available, or verify a suitable local Python/Pillow installation; the `python` example below stands for that verified executable.

Example baseline capture workflow; replace directory with a task-owned path and restore any pre-existing environment value in a long-lived shell:

```powershell
$env:HARVEST_TERMINAL_CAPTURE_DIR = Join-Path $env:TEMP 'harvest-ui-review'
bun test packages/coding-agent/test/session-terminal-navigation.test.ts
python packages/coding-agent/bench/render-terminal-captures.py $env:HARVEST_TERMINAL_CAPTURE_DIR
```

Extend that capture seam for new home/sidebar/dialog/streaming scenarios. Inspect PNGs with an image viewer/tool; merely generating them is not review. For ordinary-font qualification, use ASCII/Unicode fixtures or a font supporting the requested glyphs. A VT emulator cannot prove IME/clipboard/font behavior on every host; perform a Windows Terminal keyboard/paste/resize/exit smoke on this host and report any unavailable platform checks honestly.

Performance: run `bun packages/coding-agent/bench/rendering.ts` and a meaningful fullscreen scenario added to/reused from `transcript-compose.bench.ts`; use `harvest render <isolated fixture> --timing --repaint 5 --quiet` for replay/paint timing. `harvest render --plain` emits transcript rows, not the complete workspace, so it is not a shell/sidebar screenshot test. Record same-machine median/p95 and emitted bytes before/after. Investigate a sustained regression greater than 10%; that threshold is a proposed review budget, not an existing CI limit. The long-history scenario must demonstrate bounded visible-window work and responsive input without weakening backpressure.

## 9. Definition of done

- [ ] Home, session, composer, sidebar, transcript, tool output, and common dialogs follow one coherent OpenCode-like design.
- [ ] Default fullscreen shows the redesigned shell; inline mode and explicit themes/shapes remain functional.
- [ ] Wide/narrow/short layouts use shared geometry and preserve the active input, cursor, notices, and valid hit targets.
- [ ] Startup cached/prepaint/adopted appearance agrees and no session draft is lost.
- [ ] Sidebar data is accurate, bounded, session-owned, disposable, and generation-safe; no render-time discovery/polling.
- [ ] Tool lifecycle/expanded content, custom ownership, thinking preferences, images, and error/outcome semantics survive.
- [ ] Live/rebuilt/parked transcripts retain ordering and consistent presentation without duplicate blocks.
- [ ] Palette and sidebar actions work through current registries; hints respect remaps; existing chords still function.
- [ ] Required automated checks pass, or unrelated pre-existing failures are separately reproduced and reported. Failures caused by the redesign are fixed before delivery.
- [ ] Actual before/after captures have been inspected for all required screen families and responsive boundaries.
- [ ] Measured rendering/input behavior has no unexplained material regression.
- [ ] Updated docs and `[Unreleased]` entries describe visible behavior. No unrelated dirty work was overwritten; no commit/release was made.
- [ ] Final handoff links the screenshots and reports scenarios, results, and remaining verified limits. Do not claim complete visual parity from type checks or a single screenshot.

## 10. Non-negotiable regression guards

Follow root `AGENTS.md`: use central utilities, no `console.*` on TUI/worker/RPC paths, sanitization on every success/error/streaming path, `PREVIEW_LIMITS`/`TRUNCATE_LENGTHS` for content limits, no provider/model policy in TypeScript, no edits to generated catalog files, and no weakening of approval/jail/protocol security.

Do not change Laya, provider routing, model policy, RPC/ACP schemas, session persistence, cancellation semantics, or sharing/trust policy to complete this UI work. The open product decisions in `docs/product-decisions.md` are context; this redesign may surface existing diagnostics, but must not invent force-stop, budget enforcement, trust, or privacy behavior.

When implementation encounters source drift, keep the intended visible contract, locate the current central owner, and adapt the file map. When a dependency or platform blocks verification, preserve the work and report the exact missing check. Neither a screenshot mismatch nor a failing contract is a reason to declare the milestone complete.

## 11. Expanded coverage addendum (2026-10-04 execution assignment)

This section incorporates the expanded execution-assignment requirements. It does not replace §§1–10; it extends coverage from "redesign shell + transcript + palette" to **every user-facing feature flow**.

### 11.1 Shared presentation system (required)

- Typed UI actions with identity, category, availability reason, active keybinding, invocation intent. Actions distinguish: execute existing operation / open existing selector-inspector / collect arguments-subcommands / prepare draft without executing. Use existing dispatchers/controllers; never run command strings through the shell.
- Immutable session/run-scoped snapshots from existing authoritative owners. Rendering performs no discovery, VCS work, network, or inference. Reuse cached state/subscriptions; dispose/retarget on ownership change.
- Named workspace slots: transcript, tabs, required attention, optional widgets, focused input, metadata, hints, sidebar.
- Shared responsive dialog/list/form/inspector/notice/activity/output/artifact components. `OverlayPanel` + 60/88/116 caps; fullscreen explorers (Agent Hub, transcript rewind) preserved.
- Visual rules (§4 still authoritative): home wordmark + 75-col centered prompt; anchored composer + main column; 42-col sidebar docked >120 with usable main width; ephemeral narrow overlay independent of persisted `tui.sidebar`; actual inner-pane widths shared by render/cursor/mouse; focused input + required attention survive height pressure; semantic surface tokens incl. ANSI-reset; effective `harvest`/`harvest-light` defaults through startup/adoption/previews/runtime; plain prose, distinct user panel, restrained reasoning; compact success + purposeful code/diff/structured panels; endcap once per turn with detail on demand; remap-aware hints; registry-backed symbols.

### 11.2 Complete feature coverage (per-feature entries in `docs/agent-ui-feature-coverage.md`)

Enumerate current registries (prior evidence ~84 commands / 30 tools is stale; recon found ~90+ builtin specs, 28+3 tools — re-enumerate, don't hardcode). Every commands/subcommands/actions, tool operations, selectors, message types, settings flows, widgets, dynamic extension contracts gets an entry: feature + authoritative owner; discovery/invocation; component family + detail view; lifecycle states; errors + valid recovery; responsive/capability fallback; behavioral test + visual evidence.

Cover: startup/setup/warnings/changelog/init; sessions/tabs/resume/tree/branch/fork/rewind/handoff/worktrees/dirs; composer editing/history/autocomplete/attachments/paste/clipboard/external-editor/queues/steering/follow-up; models/roles/effort/providers/auth-accounts/quotas/fast/prewalk/xctx; plan/approval/review/goals/loops/pause-retry/forced-tools; files/search/AST/diffs/diagnostics/binary-large/artifacts; bash/manual/PTY/Eval/kernels/cells/SSH/debug; task/hub/vibe/background/advisor/watchdog/agents/live-parked; todo phases + all statuses; MCP/plugins/marketplaces/extensions/hooks/skills/templates/custom-UI; browser/computer-use/web/citations/GitHub/workspace-changes; memory/learning/maintenance/provenance/corrections; context/compaction/rules/Laya/snapcompact/tiny/diagnostics; security/findings/validation/disposition/approvals/questions/uncertain; collab/guests/permissions/sharing-export/redaction/transport; STT/voice/TTS/images/media/device-failures; settings/usage-stats-trace/help-hotkeys-tools/updates/restart/exit; BTW/TAN/OMFG/Cleanse + custom/hook/skill/advisor/summary/diagnostic/launch messages.

Deliberate semantic presentation required for: checkpoint, rewind, security_scan, memory_edit, learn, manage_skill, search_code, yield. Shared component may serve multiple features; generic JSON dump or bare label is not a complete flow. Dynamic fallbacks preserve args/output/artifacts/lifecycle/bounded diagnostics; renderer failure degrades visibly without corruption or canonical-output loss. Runtime coverage checks against registries + extension fixtures; no source-grep tests.

### 11.3 Required fallbacks (qualify each)

Unicode/Nerd/ASCII; true/limited/no color; keyboard-only, optional mouse, remapped keys, paste, Unicode input, IME; narrow/short, resizing, long/wide text, tabs, ANSI, hyperlinks, image rows; unsupported image/audio protocols; missing clipboard/editor/browser/native/service; offline/loading/degraded/interrupted/denied/uncertain; fullscreen alt-screen + inline scrollback. Truthful explanations + existing alternatives; never silently switch execution ownership or fabricate success. Preserve fail-open + approval fail-closed.

### 11.4 Review defects A–I (verified 2026-10-04; fix first)

A palette-refresh crash (unawaited, unbound) · B late-result drop on rebuild · C short-height hidden input (mostly fixed; home-branch reserve missing) · D sidebar flag/focus/nav (fixed; `app.sidebar.focus` unwired, `hide`+narrow silent no-op) · E palette windowing/paste/discovery/metadata (all confirmed) · F output geometry (unbroken overflow fixed; quiet-allocation + query-arg header + usage-row width remnants) · G anchors vs offsets (both exist, split ownership; width-resync missing) · H defaults/tokens/symbols/hints (runtime harvest/* fixed; schema defaults stale; 2 hardcoded symbol sites) · I sidebar projection gaps (agents/todo-states/extensions/VCS-retarget/worktrees). Each fix gets an observable regression test through real components/terminal input.

### 11.5 Execution order + ownership

Baseline/coverage/ownership/fixtures → regression repairs + shared interfaces → geometry/surfaces/defaults/shell/composer/nav → transcript/tool families + parity → dialogs/inspectors/integrations/fallbacks → integration/independent-review/perf/docs/acceptance. One editing owner per file at a time; cross-file needs return as patch requests to the integration owner. `docs/agent-ui-execution-state.md` (owner-only) + `docs/agent-ui-feature-coverage.md` stay current.
