# Harvest codebase audit handoff — 2026-09-30

## Scope and how to use this file

This is a **source-based, risk-focused audit**, centered on `packages/coding-agent/`, its `packages/agent` loop, and `decision-sidecar/`. I mapped the package boundaries through `packages/coding-agent/DEVELOPMENT.md`, traced production call sites, compared the Laya documentation with executable code, inspected selected cross-package paths, and ran focused checks. The repository is too large to claim that every branch of every feature was exercised. Treat each finding below as an investigation or implementation task; reproduce its consumer-visible effect before changing policy.

**Current verification:** `bun --cwd=packages/coding-agent run check:types` passed. Four focused Laya test files passed (19 tests). The live Laya test skipped its daemon probe because no sidecar was running. No fresh-host setup, GPU/CPU inference benchmark, packaged binary run, full TypeScript/Rust suite, or interactive multi-session test was performed. No implementation code was changed for this audit.

**Important existing work:** Laya auth token rotation, sidecar spawn singleflight/PID/early-exit handling, subagent abort propagation, workspace-jail use in main edit/write/LSP paths, async-job cancelled-slot retention, and auth-gateway open-mode CORS have already been addressed in current source. Older `CODE_REVIEW*.md` findings about those paths should not be copied forward without rechecking.

### Integration coverage map

| Surface | Main packages | What this pass checked | Remaining dedicated validation |
| --- | --- | --- | --- |
| CLI, SDK, agent loop | `coding-agent`, `agent` | boot map, context transform, tool preflight, completion classifier | full interactive and RPC turn matrix |
| Laya decisions | `coding-agent/core/harvest`, `decision-sidecar` | client, gating, pruning, routing, subagent selection, setup, calibration, Python inference | real model on CPU/GPU and packaged installs |
| Session UI | `coding-agent/session`, `modes` | live registry versus production tab switching | concurrent turns, approvals, cwd isolation |
| Tool and approval integrations | `coding-agent/tools`, `extensibility`, `mcp`, `lsp` | Laya interception and selected approval/jail call sites | each built-in, custom, MCP, remote, and device mutation path |
| Providers and catalog | `ai`, `catalog` | package boundaries and selected runtime logging | provider-by-provider wire and model-policy conformance |
| Memory, collaboration, natives, benchmark harness | `mnemopi`, `collab-web`, `natives`, `metaharness` | package inventory and prior-review freshness checks | separate contract/security/performance audits |

The last column is an audit boundary, not a claim that those features are broken or absent.

## Priority 0 — correctness or safety before tuning thresholds

### P0-1. Laya's tool authorization has no coding-specific calibration

**Evidence:** `decision-sidecar/calibration_params.json` contains the shipped 150-sample/site synthetic temperatures; `decision-sidecar/calibration.py:104-169` generates those records. `decision-sidecar/server.py:609-615` applies the values to confidence. `packages/coding-agent/src/core/harvest/laya-gating.ts:117-160` uses a hard-coded `noul >= 0.35` and confidence `< 0.75` rule to decide whether an otherwise high-risk call can avoid an extra approval. The normal TS client does not send `ground_truth` (`laya-client.ts:214-225`); the server logs `null` labels by default (`server.py:622-637`).

**Consequence:** The shipped temperature and thresholds do not demonstrate a measured false-clear rate on coding tools. This is especially important for shell, write, edit, and patch operations. Existing approval policy still applies; the issue is treating a Laya clear as a trustworthy additional safety signal.

**Action:** Keep conservative approval behavior. Build a labeled coding-tool dataset from real, consented traces with distinct read-only, reversible write, destructive, network/publish, and permission-change classes. Measure false clears, false escalations, timeout rate, and decision latency by tool class and hardware. Calibrate on a held-out set and version the calibration provenance. Require a measured acceptance threshold before enabling autonomous clear decisions.

### P0-2. Sidecar inference can continue after timeout while its concurrency slot is released

**Evidence:** `decision-sidecar/server.py:584-596` runs `_predict_sync` in `asyncio.to_thread()` inside `asyncio.wait_for()` and an `async with semaphore`. Python cancellation of the await does not stop a running worker thread. On timeout or client disconnect, the semaphore can be released while PyTorch work continues. The default server timeout is 120 seconds (`server.py:98`), while the TS client's default decision timeout is 300 ms (`laya-client.ts:55-56, 202-209`).

**Consequence:** Repeated short client timeouts can leave expensive inference running and admit more than the intended two active forwards, worsening latency or memory pressure. The reported 504 does not mean compute stopped.

**Action:** Keep capacity assigned until the actual inference future settles, or move inference to a cancellable/killable worker process with an explicit queue and deadline. Record queue wait, inference duration, client cancellation, and in-flight count. Test many 300 ms client aborts against slow inference and assert active model calls never exceed capacity.

### P0-3. Laya context pruning can replay a stale decision onto different history

**Evidence:** `laya-pruning.ts:93-110` stores locked decisions in a process-global map keyed by session ID plus `turnIndex`, `messageIndex`, and tool name (`:331-335, :362-366`). A search of production `packages/coding-agent/src` found no call to `resetLockedPruningDecisions` outside its definition. Session trees can rewind, compact, or switch branches while retaining a session identity. The map is also unbounded by session count.

**Consequence:** If message positions are reused after a branch/history rewrite, a prior keep/drop choice can be applied to different content. Long-lived processes retain decisions after sessions close.

**Action:** Key by stable message/content identity and branch generation, invalidate on every history rewrite/rewind/compaction, and bound retention. Add a regression test that forks or rewinds to a different message at the same index, then verifies the new content is scored afresh.

## Priority 1 — missing integrations and user-visible behavior

### P1-1. The documented Laya model router is not wired into production model selection

**Evidence:** `docs/decision-layer.md:34-43` describes routing at turn transitions, and `laya-routing.ts:33-121, 123-...` implements model/specialist classifiers. Production search found exports and tests, but no call to `routeModelWithLaya` or `routeSpecialistRoleWithLaya` from `sdk.ts`, the session loop, or model resolver.

**Consequence:** The settings and documentation promise a decision point that does not affect user turns. The bundled `model_routing` calibration is not exercising a live production path.

**Action:** Decide whether routing is a supported feature. If yes, integrate it at a single model-selection boundary after honoring explicit user model choices, role availability, budget, and provider credentials; emit a routing reason and fallback metric. If no, remove the claim and unused implementation. Do not silently switch the user's explicitly chosen model.

### P1-2. Live session registry exists but normal tab navigation still aborts/reloads

**Evidence:** `session/live-session-registry.ts` implements live runtime ownership, but production search found no importer of `LiveSessionRegistry`. `modes/controllers/selector-controller.ts:2069-2125` still calls `this.ctx.session.switchSession(...)`, clears UI state, and rerenders history. `REVIEW_RECENT_CHANGES_2026-09-28.md` documents the unfinished binding.

**Consequence:** Switching tabs interrupts active work and incurs reload latency; background completion and approvals cannot remain correctly attached to their source tab.

**Action:** Wire session-keyed runtimes through `/new`, `/sessions`, tab selection, prompt dispatch, event subscriptions, approvals, close, and shutdown. Preserve per-session cwd/settings/extension UI context. Use `releaseIdleRuntimes()` only after UI detachment and protect busy sessions. Verify two concurrent model runs, one approval on an inactive tab, background completion visibility, and failed-switch rollback in an interactive test.

### P1-3. Hardware-derived Laya settings are masked by schema defaults

**Evidence:** `settings-schema.ts:578-587` gives `laya.pruning` default `true`, and `:652-660` gives `laya.subagentSelectionTimeoutMs` default `300`. `laya-pruning.ts:250-263` uses the calibration recommendation only when the setting is `undefined`; a default `true` makes a slow-hardware recommendation advisory only. `laya-subagent-selection.ts:223-227` similarly reads the default 300 ms before the measured timeout.

**Consequence:** Calibration may say pruning is too slow or derive a longer/shorter timeout, but the normal effective values do not change. The UI text says the timeout is derived when unset, yet schema defaults make it set.

**Action:** Distinguish an explicit user override from a default (`auto`/unset plus effective-value resolver). Surface configured, derived, and effective values in `/laya status`. Test both explicit override and calibrated auto mode.

### P1-4. Default shadow subagent selection blocks dispatch

**Evidence:** `settings-schema.ts:630-639` enables shadow mode by default and describes it as background. `task/structured-subagent.ts:289-300` awaits `selectSubagentWithLaya` before agent dispatch. `laya-subagent-selection.ts:229-234, 278-314` performs the HTTP decision, then returns the caller/default agent in shadow mode.

**Consequence:** Even though shadow mode cannot change the selected agent, every generic handoff can wait for sidecar inference or the 300 ms timeout. This adds serial latency to the agent loop for telemetry alone.

**Action:** Dispatch immediately using the caller/default agent; run shadow classification asynchronously with bounded concurrency and cancellation, then join its audit outcome later. Measure handoff p50/p95 before and after. Ensure background work is dropped or capped when the sidecar is overloaded.

### P1-5. Pruning uses the first user request as the permanent task goal

**Evidence:** `laya-pruning.ts:219-229` returns the first user message as `extractTaskGoal`; `:296, 451-465` uses it for every relevance question. A test currently asserts this behavior as the “current task goal.”

**Consequence:** In a long chat with a new request, Laya scores history against an old task. Relevant context for the current request may be dropped, and irrelevant old material may be retained.

**Action:** Derive the active goal from the latest user request or explicit goal state, while preserving durable constraints separately. Include task/branch generation in locked-decision identity. Test a chat that changes task after several turns.

### P1-6. Pruning spends inference when its budget cannot drop anything

**Evidence:** `laya-pruning.ts:446-465` scores all candidates first. Only afterward does `:508-539` read the 32,000-token default pruning budget (`settings-schema.ts:609-617`) and decide what fits. There is no pre-score shortcut when all candidate tokens fit under budget.

**Consequence:** Short and medium sessions pay a local inference round trip for zero token savings. With a 300 ms client deadline, this can increase turn latency while often falling back.

**Action:** Compute the effective budget before the call; bypass Laya when all eligible candidate tokens fit, accounting for already locked drops and safety floors. Instrument “scored/no drop,” tokens saved, and added wall time. Only enable pruning when measured downstream model savings exceed local decision cost.

### P1-7. Long-session pruning can exceed the sidecar's batch limit and fail open wholesale

**Evidence:** `laya-pruning.ts:451-465` puts every newly eligible candidate into one request. The server rejects more than 64 questions (`server.py:81, 401-405`). The client then returns a fallback, and pruning sends full context (`laya-pruning.ts:468-489`).

**Consequence:** Exactly when a session grows large enough to need pruning, a batch cap can disable it for that turn. Large payloads also increase inference latency even below the cap.

**Action:** Bound work per turn and chunk requests to the server contract. Prefer deterministic eligibility filtering before Laya, score only newly aged-out chunks, and maintain a small queue. Define partial-failure behavior that keeps uncertain content, without sending an oversized request.

### P1-8. Laya client singleton does not follow runtime URL changes

**Evidence:** `laya-client.ts:279-293` caches a default `LayaClient` constructed from the URL setting once. When `getLayaClient(baseUrl)` sees a different URL, it returns a new client without replacing the default. Setup can resolve a port conflict and persist another `laya.url` (`laya-service.ts:879-892, 1199-...`); `laya-cli.ts:528-533` can instantiate the singleton before setup.

**Consequence:** In the same Harvest process, subsequent no-argument callers may continue using the old port and fall back or fail closed until restart.

**Action:** Resolve the effective URL at call time or invalidate/rebuild the singleton when settings change. Test setup moving from 8177 to 8178 followed immediately by normal gating/pruning through `getLayaClient()`.

### P1-9. Tool gating only classifies a fixed name list, leaving integration gaps

**Evidence:** `laya-gating.ts:22-34, 77-96` gates only named shell/write/edit aliases. MCP and extension tools have their own approval declarations (`docs/approval-mode.md`), but their mutation-capable tool names are absent from this set.

**Consequence:** Laya's extra risk signal does not apply consistently across tool integrations. This does **not** mean MCP/custom tools bypass the existing approval engine; it means the Laya integration does not cover them.

**Action:** Base eligibility on structured tool approval tier/capability plus explicit exceptions, not wire names. Ensure revised extension input is classified once at the final execution boundary. Test a custom write tool and MCP write tool with the same risk contract as built-ins.

### P1-10. Completion check is narrower than advertised and is not cancellable

**Evidence:** `laya-completion.ts:33-39` accepts no `AbortSignal`; `:67-90` omits `taskContext` from the payload and always sends both success and unexpected-stop questions. The sole production use is `session/unexpected-stop-classifier.ts:66-80`, which only reads `isPrematureStop` from assistant text. No production step-result evaluator calls it.

**Consequence:** Documentation describes general step/task completion, but users get only unexpected-stop classification. The unused success question adds work, and a cancelled turn may wait for the decision timeout.

**Action:** Either wire step-result evaluation to the actual tool outcome/exit code and task context, or narrow the feature and send only the unexpected-stop question. Propagate the turn signal. Benchmark false continuation and false completion against the existing classifier before replacement.

### P1-11. Sidecar reuse trusts an unauthenticated, weak health response

**Evidence:** `laya-service.ts:353-365` accepts any HTTP listener returning `ready: true` or `status: "ok"`; `:872-888` can reuse that listener. Health does not prove model identity or that `/v1/decide` accepts the managed token.

**Consequence:** A foreign listener at the port can be treated as a healthy sidecar, while all real decisions fail later. This can create a confusing fail-closed approval storm.

**Action:** Require expected model/version identity and an authenticated, bounded decide probe before reuse. Never kill a foreign listener. Add a fake-health-server test.

## Priority 2 — performance, integration hygiene, and validation

### P2-1. No end-to-end decision-layer latency budget is enforced

The client uses 300 ms by default (`laya-client.ts:55-56`), while calibration explicitly permits multi-second single-choice inference and a 15-second subagent ceiling (`laya-calibration.ts:210-229`). Interactive autostart runs in the background (`main.ts:1882-1886`), so early calls may race model loading. Define a per-decision-point latency budget and a circuit breaker: after repeated timeout/overload, skip optional Laya decisions for a cooldown. Keep tool-gating fallback approval. Capture p50/p95/p99 for token read, queue wait, inference, total HTTP, fallback, and downstream tokens saved; use the existing diagnostics/performance tools rather than a second telemetry stack.

### P2-2. Sidecar request caps occur after framework body parsing

`server.py:301-318` lets FastAPI/Pydantic build the request object before `_validate_request` checks question count and state size (`:396-448`). The documented 500k-character limit is therefore an inference/tokenization bound, not a transport memory bound. Add ASGI/body-byte limits before JSON parsing and test an oversized unauthenticated request on loopback. Keep the authenticated inference endpoint loopback-only.

### P2-3. Runtime decision logs are tracked in the repository

`git ls-files` reports `decision-sidecar/decisions.jsonl` as tracked; the current file is about 240 KB. `server.py:617-639` writes state snippets, instructions, session IDs, answers, and timestamps there. Move operational logs under a private user-data directory, ignore them in Git, and provide an explicit sanitized export for calibration. Review existing tracked content before repository history changes; do not assume snippets contain no secrets.

### P2-4. Python inference depends on private Laya internals with a floating dependency

`server.py:457-556` imports `laya.agent` internals and calls `agent._to_internal`, accesses `agent.cfg`, tokenizer, model, and temperature maps. `requirements.txt` allows `laya>=0.3.5`. A later upstream release can change those internals without a dependency error. Pin a tested version/range, add a startup compatibility probe and a numerical-equivalence test against the public single-question `agent.predict` path, and make upgrades deliberate.

### P2-5. Laya prompt assets violate the repo prompt convention

Decision instructions and criteria are inline strings in `laya-gating.ts:107-113`, `laya-routing.ts:26-31, 70-79`, `laya-completion.ts:73-85`, and `laya-pruning.ts:451-458`. `AGENTS.md` requires prompts in static `.md` files with Handlebars for dynamic content. Consolidate these into versioned prompt assets so calibration data can be tied to an exact prompt revision. This also removes duplicate wording from tests and docs.

### P2-6. Unvalidated response shapes and loose types obscure fallback behavior

`laya-client.ts:235-267` casts JSON directly to `LayaDecideResponse`; it does not validate answer type, finite numeric range, missing question IDs, or model identity. Callers fill missing fields with different neutral values (`laya-gating.ts:143-145`, `laya-pruning.ts:493-505`, `laya-completion.ts:108-116`). `laya-pruning.ts` and `laya-calibration.ts` also use `any` for protocol blocks and payloads. Add one schema validator at the client boundary, fail to the decision point's defined fallback, and use typed content-block guards. Include malformed, NaN/out-of-range, and partial-answer tests.

### P2-7. Shadow data and threshold evaluation need outcome quality, not just caller agreement

`laya-subagent-selection.ts:291-303` writes `groundTruth: defaultAgent` in shadow mode, which records the caller's pick as if it were a correctness label. A caller choice is a baseline, not ground truth. Keep caller-vs-Laya disagreement data, but label actual task outcomes separately and blind-review a representative sample. The `0.01` default confidence threshold (`settings-schema.ts:641-649`) should not be described as safe until verified on held-out real assignments.

### P2-8. Configuration and documentation disagree with effective behavior

`docs/decision-layer.md` and `decision-sidecar/README.md` present model routing and general completion as active interception points, while current production wiring is narrower. `decision-sidecar/README.md:80-83` describes `score` as 0–10, while pruning normalizes a 0–3 four-option score (`laya-pruning.ts:43-48, 493-499`). Update docs from tested behavior and add a small contract test for output shape and supported settings. Do not use docs alone as feature-completeness evidence.

### P2-9. Shared runtime telemetry can write to the terminal

`packages/agent/src/telemetry.ts:634-641` falls back to `console.warn` when no warning hook is installed or a hook throws. `AGENTS.md` forbids `console.*` on TUI/RPC/worker paths. Route this through the centralized logger or a supplied warning sink; verify JSONL RPC stdout and interactive rendering stay clean when telemetry callbacks fail.

### P2-10. Installation and packaging coverage remains incomplete

`REVIEW_RECENT_CHANGES_2026-09-28.md` records that clean Windows/macOS/Linux setup, restricted/offline installs, actual CPU/GPU wheel selection, and standalone binary Laya inference were not exercised. The current focused tests do not close those gaps. Add a CI matrix or manual release gate for source, npm bundle, and compiled binary; verify one real `/v1/decide` call, token rotation, port fallback, and teardown on each supported platform. Keep the single-checkpoint invariant.

## Suggested implementation sequence for another agent

1. **Protect correctness:** fix timeout capacity, pruning lock identity/invalidation, and response validation; add focused regression tests for each observable failure.
2. **Make Laya policy evidence-based:** collect coding-specific labeled data, separate shadow telemetry from dispatch, and expose effective calibrated settings. Keep conservative tool approvals throughout.
3. **Remove wasted latency:** skip pruning below budget, bound scoring batches, adopt per-call deadlines/circuit breaker, and measure actual token savings against added decision time.
4. **Finish integration deliberately:** connect live sessions to the UI, decide whether model routing and step completion are supported features, then wire or remove the claims.
5. **Harden release paths:** validate authenticated sidecar identity, private logs, pinned dependency compatibility, and packaged cross-platform smoke tests.

## Measurement plan

Before claiming “faster,” record a baseline on at least one fast GPU and one ordinary CPU machine: startup-to-ready; prompt-to-first-token; tool-call overhead; subagent handoff; context-transform time; Laya queue/inference/client time; timeout/fallback percentage; prompt tokens before/after pruning; provider time and cost; completion accuracy; and p50/p95/p99 per task class. Repeat with Laya disabled, enabled with gating only, shadow selection, pruning, and each proposed optimization. Report end-to-end turn time and correctness together; an isolated local classifier benchmark cannot prove faster agent turns.
