# Harvest codebase audit handoff — 2026-09-30

## Scope and how to use this file

This is a **source-based, risk-focused audit**, centered on `packages/coding-agent/`, its `packages/agent` loop, and `decision-sidecar/`. I mapped the package boundaries through `packages/coding-agent/DEVELOPMENT.md`, traced production call sites, compared the Laya documentation with executable code, inspected selected cross-package paths, and ran focused checks. The repository is too large to claim that every branch of every feature was exercised. Treat each finding below as an investigation or implementation task; reproduce its consumer-visible effect before changing policy.

**Second-pass snapshot:** HEAD `9324b505ba702d00a675b7e85d71c11eb4f300ac`, including concurrent uncommitted live-session, Markdown-brain, gating, and pruning work visible during review. These files were changing independently of this audit. References below are repository-relative and one-based; recheck them against the implementing agent's checkout. Only this Markdown report was edited by the reviewer.

**Verification:** The first-pass type check passed. In this second pass, `bun --cwd=packages/coding-agent run check:types` failed at `test/markdown-brain.test.ts:73` because a `toolResult` fixture omits required `isError`. Three focused runs produced **183 passing and two failing tests across 13 files**. The first nine files passed 110 tests: MCP reconnect, collab read-only, RPC prompt-result/input-frame, concurrent AgentSession prompts, Laya gating-abort/pruning/client-auth, and Markdown-brain. Additional Anthropic/OpenAI replay, catalog conformance, and memory-tool tests produced 73 passes and two Mnemopi recall/reflection failures, traced to an unavailable native kernel's null fallback (U14). The live Laya probe skipped because no daemon was running. Isolated probes also reproduced MCP duplicate effects, collab transcript scope escape, session rewrite data loss, delayed RPC abort, RPC local-only wait timeout, accepted oversized RPC input, an unsettled guest UI request, and the native MMR failure in a fresh process. No real remote write, provider inference, network-exposed server, or implementation modification was used for these probes. No fresh-host setup, GPU/CPU inference benchmark, packaged binary run, full TypeScript/Rust suite, or interactive multi-session test was performed.

**Evidence labels:** “Reproduced” means the observable failure was exercised using the actual component with controlled dependencies; it does not imply a live external service was tested. “Source-confirmed” means the relevant implementation was traced. “Proposed feature” means a product/architecture improvement, rather than proof an existing contract is broken. P0/P1/P2 indicate implementation priority within this report, not an assertion of a demonstrated production exploit. It is impossible to enumerate every human behavior; the scenario matrix below covers major user intents, mistakes, interruptions, and environmental failures without claiming exhaustive coverage.

**Important existing work:** Laya auth token rotation, sidecar spawn singleflight/PID/early-exit handling, subagent abort propagation, workspace-jail use in main edit/write/LSP paths, async-job cancelled-slot retention, and auth-gateway open-mode CORS have already been addressed in current source. Older `CODE_REVIEW*.md` findings about those paths should not be copied forward without rechecking.

### Third-pass completion status — 2026-10-03 (HEAD `db336df`, clean tree)

Source-based recheck of every finding against the current checkout (audit snapshot was `9324b50`). **26/38 P0+P1+P2+U findings fixed; 0/10 F proposals implemented** (5 partial). Detail sections below are historical as-written; this table supersedes their `Status:` dispositions. Full `check:types` gate was not re-run (U13 fixture confirmed fixed in file only).

| ID | Status | Delta evidence |
| --- | --- | --- |
| P0-1 | Partial | Provenance/holdout infra added (`calibration.py`); shipped params still synthetic, thresholds still hard-coded (`laya-gating.ts:47`) |
| P0-2 | Partial | Bounded queue/deadline/metrics added (`inference_scheduler.py:18`); worker still `to_thread`, no killable process |
| P0-3 | Partial | Keying/bounding fixed (`laya-pruning.ts:158`); sole production caller is dispose (`agent-session.ts:4765`), no rewind/compact invalidation |
| P1-1 | Fixed | Routing documented as not wired; no production callers |
| P1-2 | Fixed | Tab path tries live resume first (`selector-controller.ts:2072-2087`); legacy switch kept as fallback |
| P1-3 | Partial | Explicit-vs-derived resolver exists; `/laya status` (`laya-cli.ts:126-154`) still misreports defaults as overrides |
| P1-4 | Fixed | Shadow dispatches immediately, classifies in background (`structured-subagent.ts:317-331`) |
| P1-5 | Fixed | Production path uses active goal (`laya-pruning.ts:391`), legacy first-message helper retained only |
| P1-6 | Fixed | Pre-score budget bypass (`laya-pruning.ts:544-554`) skips no-op scoring |
| P1-7 | Fixed | Client chunks to 64-question cap (`laya-pruning.ts:46-47`) |
| P1-8 | Fixed | Singleton resolved per call, rebuilt on drift (`laya-client.ts:415-440`) |
| P1-9 | Fixed | Tier-first gating covers MCP/extension (`laya-gating.ts:52-66`) |
| P1-10 | Fixed | Abort signal + `task_context` + narrowed checks (`laya-completion.ts:43-44,89-94`) |
| P1-11 | Fixed | Reuse requires model identity + authenticated decide probe (`laya-self-healing.ts:553-597`) |
| P2-1 | Fixed | Circuit breaker + p50/p95 (`laya-circuit.ts:35-38`); p99 gap remains |
| P2-2 | Fixed | Pre-parse 8 MB body cap middleware (`server.py:402-467`) |
| P2-3 | Partial | Writes moved to user-data + gitignored, but `decisions.jsonl` still tracked in git |
| P2-4 | Partial | Range bounded (`<0.4.0`), startup probe exists; equivalence test opt-in only |
| P2-5 | Partial | Production prompts in `.md`; subagent/calibration questions still inline |
| P2-6 | Fixed | Central validator (`laya-client.ts:72-121`) + caller re-checks kept |
| P2-7 | Fixed | `groundTruth` separated from `callerBaseline`; threshold still provisional pending held-out validation |
| P2-8 | Fixed | README corrected to 0–3 + contract test (`laya-pruning.test.ts:54-83`) |
| P2-9 | Fixed | Telemetry fallback via `logger.warn` (`telemetry.ts:632-644`), no `console.*` |
| P2-10 | Partial | 8-target binary + sidecar smoke in `release.yml`; no source-vs-npm/macOS-sidecar/offline legs |
| U1 | Fixed | Shared-root predicate gates snapshot/bus/transcript/control; advisors excluded (`host.ts:581-589,592-627,691-706`) |
| U2 | Partial | Replay limited to pre-dispatch `connect` stage + `outcomeUnknown` flag; no idempotency keys, no read/write split |
| U3 | Fixed | Cross-process file lock + lease/claim on rewrite (`file-lock.ts:1-55`, `session-manager.ts:3048-3058,486-528`) |
| U4 | Fixed | Request-correlated completion incl local-only/idle/disconnect (`rpc-client.ts:1041-1071,1082-1180`) |
| U5 | Partial | `abort*` on immediate lane + queue bound 64; no run/generation binding |
| U6 | Fixed | Non-loopback bind refused before serve without token (`stats/server.ts:462-472`) |
| U7 | Partial | 1 MiB line cap + drain + chunk admission; no stdin backpressure |
| U8 | Fixed | Unknown-command errors preserve validated request ID (`rpc-mode.ts:1592-1599`) |
| U9 | Fixed | 4 MiB bounded body/SSE reads (`http.ts:39,47,61-109`) |
| U10 | Fixed | Pending UI settles `unavailable` when no writable peer remains (`host.ts:177-178,532-544`) |
| U11 | Fixed | Progress to stderr in JSON mode, single doc on stdout (`stats/index.ts:109-113,172-200`) |
| U12 | Partial | Explicit rerank controls + result cache; no shared prune+rerank budget, no prompt-rev in key |
| U13 | Fixed (fixture; gate not re-run) | Fixture now includes `isError: false` (`markdown-brain.test.ts:228`) |
| U14 | Fixed | Capability-checked native path with real TS fallback (`mmr.ts:25-61,82-96`) |
| F1 | Absent | `isProjectTrusted: () => true` stub unchanged; no trust gate |
| F2 | Partial | Still uniform `approval="write"`; no per-server/tool policy |
| F3 | Absent | No user monetary ceiling found |
| F4 | Partial | Share redaction exists; no per-channel preview |
| F5 | Partial | `outcomeUnknown` flag only; no outcome ledger |
| F6 | Absent | Fragments only, no unified view |
| F7 | Partial | Store-side APIs exist; no provenance UI |
| F8 | Absent | No single pipeline diagnostic |
| F9 | Partial | Binary matrix only; listed portability legs missing |
| F10 | Absent | No owner/phase/force-stop UI contract |

### Start here

| Finding | Immediate concern | Evidence level |
| --- | --- | --- |
| U1 | Collaboration reads unrelated sessions and advisor transcripts | Reproduced |
| U2 | MCP repeats a committed external action after response loss | Reproduced with typed transport failure |
| U3 | Competing session owners lose persisted history during stale rewrite | Reproduced with file storage |
| U6 | Explicit remote stats bind has no authentication | Source-confirmed |
| U14 | Native fallback crashes multi-result memory recall/reflection | Existing test failures and fresh-process repro |
| P0-2 / P0-3 | Laya timeout capacity and stale pruning identity | Source-confirmed risks requiring focused regression repro |
| U4 / U5 / U7 | RPC waits, cancellation, and admission | Reproduced at component boundaries |

The report contains 24 first-pass items rechecked where relevant, 14 second-pass findings, 10 proposed product controls, and a broad human/user scenario matrix. These counts include validation gaps and proposals; they are not a count of proven production bugs.

### Integration coverage map

| Surface | Main packages | What this pass checked | Remaining dedicated validation |
| --- | --- | --- | --- |
| CLI, SDK, agent loop | `coding-agent`, `agent` | boot map, context transform, tool preflight, completion classifier, RPC completion/cancel/admission | full interactive and real-host RPC turn matrix |
| Laya decisions | `coding-agent/core/harvest`, `decision-sidecar` | client, gating, pruning, routing, subagent selection, setup, calibration, Python inference | real model on CPU/GPU and packaged installs |
| Session UI | `coding-agent/session`, `modes` | live registry versus production tab switching | concurrent turns, approvals, cwd isolation |
| Tool and approval integrations | `coding-agent/tools`, `extensibility`, `mcp`, `lsp` | Laya interception, selected approval/jail paths, uncertain MCP retry, HTTP limits, project executable trust | each built-in, custom, MCP, remote, and device mutation path |
| Providers and catalog | `ai`, `catalog` | package/policy boundaries, runtime logging, selected Anthropic/OpenAI replay and KDL conformance tests | all provider wire/auth combinations and live model transitions |
| Memory/retrieval | `mnemopi`, coding-agent memory backends/brain | scope/edit/forget/lifecycle source and memory-tool tests; native MMR fallback failure | large-store performance, extraction services, deletion/privacy across every backend |
| Collaboration/browser | `coding-agent/collab`, `collab-web`, `browser-relay` | collab resource authorization and UI lifecycle; relay loopback/origin/token/payload guards | real web/TUI peers, browser disconnects, live multi-session ownership |
| Stats/harness | `stats`, `metaharness` | stats bind/auth and JSON paths; comparison with harness non-loopback guard | live authenticated dashboard, benchmark/job lifecycle and load tests |
| Native/TUI/ancillary packages | `natives`, Rust crates, `tui`, remaining packages and Python | selected native fallback/vector and Rust path-policy source; TUI ownership/backlog structure; subsystem inventory | complete Rust/platform/native/TUI audit and dedicated ancillary-package contracts |

The last column is an audit boundary, not a claim that those features are broken or absent.

## Priority 0 — correctness or safety before tuning thresholds

### P0-1. Shipped calibration does not establish coding-tool authorization safety

**Evidence:** `decision-sidecar/calibration_params.json` contains the shipped 150-sample/site synthetic temperatures; `decision-sidecar/calibration.py:104-169` generates those records. `decision-sidecar/server.py:609-615` applies the values to confidence. `packages/coding-agent/src/core/harvest/laya-gating.ts:117-160` uses a hard-coded `noul >= 0.35` and confidence `< 0.75` rule to decide whether an otherwise high-risk call can avoid an extra approval. The normal TS client does not send `ground_truth` (`laya-client.ts:214-225`); the server logs `null` labels by default (`server.py:622-637`).

**Consequence:** The shipped temperature and thresholds do not demonstrate a measured false-clear rate on coding tools. This is especially important for shell, write, edit, and patch operations. Existing approval policy still applies; the issue is treating a Laya clear as a trustworthy additional safety signal.

**Action:** Keep conservative approval behavior. Build a labeled coding-tool dataset from real, consented traces with distinct read-only, reversible write, destructive, network/publish, and permission-change classes. Measure false clears, false escalations, timeout rate, and decision latency by tool class and hardware. Calibrate on a held-out set and version the calibration provenance. Require a measured acceptance threshold before enabling autonomous clear decisions.

### P0-2. Sidecar inference can continue after timeout while its concurrency slot is released

**Evidence:** `decision-sidecar/server.py:584-596` runs `_predict_sync` in `asyncio.to_thread()` inside `asyncio.wait_for()` and an `async with semaphore`. Cancellation of that await does not stop an already-running worker thread. On the **server's inference timeout**, or cancellation of the route task, the semaphore can be released while PyTorch work continues. The default server timeout is 120 seconds (`server.py:98`), while the TS client's default decision timeout is 300 ms. A client disconnect does not necessarily cancel a FastAPI route; do not equate the 300 ms client deadline with release of server capacity. Semaphore queue wait also occurs before the inference timeout starts.

**Consequence:** Abandoned client requests can continue consuming capacity or queueing. When server deadlines expire while forwards still run, new admissions can exceed the intended active-forward limit. The reported 504 does not mean compute stopped. This is source-confirmed; the real model's timeout/overlap behavior was not benchmarked.

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

**Evidence:** The first-pass “no production importer” claim is superseded: `interactive-mode.ts:122, 601, 1260` now imports, stores, and initializes `LiveSessionRegistry`; a cold-open factory and `SessionFocusController.selectMainSession` also exist. However, the inspected `selector-controller.ts:2069-2136` resume/tab path still calls `this.ctx.session.switchSession(...)` and only tracks the current runtime afterward. This finding is **partial integration under active development**, and must be rechecked when the concurrent tab work lands.

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

The client still casts parsed JSON directly to `LayaDecideResponse` without a central schema/type/model-identity check. **Concurrent fixes now reject non-finite/out-of-range gating probability and confidence, and missing/non-finite/out-of-range pruning scores**; the second-pass tests for these fixes passed. Do not report those particular bypasses as unfixed. Completion still uses caller-specific neutral defaults, and answer discriminants plus several other response fields remain unchecked centrally. `laya-pruning.ts`, `laya-calibration.ts`, and `laya-self-healing.ts` still contain loose `any` boundaries. Finish a central response validator while preserving the new conservative caller checks; validate expected question IDs/types and finite ranges. Use typed protocol-block guards and cover malformed, partial, and wrong-type responses at the consumer boundary.

### P2-7. Shadow data and threshold evaluation need outcome quality, not just caller agreement

`laya-subagent-selection.ts:291-303` writes `groundTruth: defaultAgent` in shadow mode, which records the caller's pick as if it were a correctness label. A caller choice is a baseline, not ground truth. Keep caller-vs-Laya disagreement data, but label actual task outcomes separately and blind-review a representative sample. The `0.01` default confidence threshold (`settings-schema.ts:641-649`) should not be described as safe until verified on held-out real assignments.

### P2-8. Configuration and documentation disagree with effective behavior

`docs/decision-layer.md` and `decision-sidecar/README.md` present model routing and general completion as active interception points, while current production wiring is narrower. `decision-sidecar/README.md:80-83` describes `score` as 0–10, while pruning normalizes a 0–3 four-option score (`laya-pruning.ts:43-48, 493-499`). Update docs from tested behavior and add a small contract test for output shape and supported settings. Do not use docs alone as feature-completeness evidence.

### P2-9. Shared runtime telemetry can write to the terminal

`packages/agent/src/telemetry.ts:634-641` falls back to `console.warn` when no warning hook is installed or a hook throws. `AGENTS.md` forbids `console.*` on TUI/RPC/worker paths. Route this through the centralized logger or a supplied warning sink; verify JSONL RPC stdout and interactive rendering stay clean when telemetry callbacks fail.

### P2-10. Installation and packaging coverage remains incomplete

`REVIEW_RECENT_CHANGES_2026-09-28.md` records that clean Windows/macOS/Linux setup, restricted/offline installs, actual CPU/GPU wheel selection, and standalone binary Laya inference were not exercised. The current focused tests do not close those gaps. Add a CI matrix or manual release gate for source, npm bundle, and compiled binary; verify one real `/v1/decide` call, token rotation, port fallback, and teardown on each supported platform. Keep the single-checkpoint invariant.

## Measurement plan

Before claiming “faster,” record a baseline on at least one fast GPU and one ordinary CPU machine: startup-to-ready; prompt-to-first-token; tool-call overhead; subagent handoff; context-transform time; Laya queue/inference/client time; timeout/fallback percentage; prompt tokens before/after pruning; provider time and cost; completion accuracy; and p50/p95/p99 per task class. Repeat with Laya disabled, enabled with gating only, shadow selection, pruning, and each proposed optimization. Report end-to-end turn time and correctness together; an isolated local classifier benchmark cannot prove faster agent turns.

## Second pass: user journeys and integration defects

### U1 — P1: Collaboration exposes agents and transcripts outside the shared session

**Status: reproduced.** `packages/coding-agent/src/collab/host.ts:564-581` snapshots the entire process-global `AgentRegistry`, excluding only advisors. `:593-637` authorizes agent chat/kill/revive by write-token membership and advisor kind, without checking that the requested agent belongs to the shared root. `:641-679` fetches any registry transcript by ID, without a root/descendant check or an advisor exclusion. `registry/agent-registry.ts:117-124` defines that process-global registry; entries carry `parentId`.

**User scenario:** A user shares project A while project B or an advisor is registered in the same process. A view-link guest learns B's ID from the welcome snapshot and reads its transcript. A known advisor ID can also be fetched, despite the explicit code comment that advisors must never be mirrored. A write-link guest can reach control operations for unrelated non-advisor IDs. Room encryption and write-token checks are present; the defect is resource scope after joining a valid room.

**Probe result:** A real `CollabHost`/`CollabSocket` over the existing encrypted in-memory relay returned `visibleAgents: ["other-project"]` and the unrelated transcript to a read-only guest. Fetching a registered advisor ID returned that transcript too. No live network sharing was started. Unrelated write control was source-confirmed, not executed.

**Action:** Capture the shared root's stable identity and generation when the room starts. Use one authorization predicate for snapshots, bus events, transcript reads, and controls, permitting only that root and its owned descendants. Exclude advisors everywhere. Require established peer membership for transcript requests as well. Define room behavior on tab/root switches rather than silently broadening scope.

**Acceptance:** Register A, B, children of both, and an advisor. A's read-only and writable guests cannot discover/read/control B, B's children, or the advisor, including direct guessed-ID requests. A's permitted child transcript remains readable; existing view-link mutation denial stays intact.

### U2 — P1: MCP reconnect can repeat an already-completed action

**Status: reproduced.** `mcp/tool-bridge.ts:55-71, 583-641` retries calls after selected connection failures. `DeferredMCPTool` has the same post-call retry pattern around `:745-767`. `_toolCallId` is not used to deduplicate remote execution. The bridge does not condition retries on tool idempotency. The stdio reader marks receive-side EOF retryable at `mcp/transports/stdio.ts:644-653`; `mcp/errors.ts:192-235` also marks resets retryable. HTTP 502/503 responses are retryable even though an upstream operation may have committed. The HTTP accepted-SSE failure path already prevents replay; keep that existing protection.

**User scenario:** Send a message, create a ticket, perform a deployment, or charge an external resource through an MCP tool. The server commits, then exits or loses the response. Harvest reconnects and sends the same operation again. The user approved one action but receives two effects.

**Probe result:** A real `MCPTool` with a controlled transport committed a counter increment, then threw a typed stdio receive-EOF error matching production metadata. Reconnect succeeded and the same call produced **two effects**, even with `idempotentHint: false`. A generic reset probe also produced two effects.

**Action:** Separate safe reconnect-before-send from retry-after-uncertain-delivery. For uncertain writes, report “outcome unknown” and reconcile remote state. Retry only when delivery is proven absent, or when a trusted idempotency contract/server key makes replay safe. JSON-RPC request IDs alone do not provide business-level idempotency. Do not treat untrusted annotation hints as sufficient authorization.

**Acceptance:** A commit-then-disconnect fixture is invoked once for a non-idempotent action. A safe read or server-enforced idempotent write can recover without duplicate effects. Cover eager/deferred tools, stdio, HTTP reset, 502/503, cancellation, and OAuth challenge recovery separately.

### U3 — P1: Two writable owners of one session file can silently lose history

**Status: reproduced with real file storage.** `SessionManager.open` around `session/session-manager.ts:2912-2938` loads a file and creates another writable manager. The inspected implementation has per-manager persistence serialization and epoch guards, but no cross-owner lease. `rewriteEntries` at `:2463` writes the manager's in-memory entries through the atomic rewrite path at `:909-951`. The comment on `peekSessionInit` at `:2942-2946` claims `open` acquires a single-writer lock; the observed behavior does not enforce that claim. Production turn-recovery and session-maintenance paths call `rewriteEntries`.

**User scenario:** Resume the same session in two terminals, or embed two SDK runtimes against the same file. Manager B appends a message. Manager A performs a repair/maintenance rewrite using its older loaded history. B's message disappears. Atomic replacement protects publication, but does not resolve competing owners.

**Probe result:** Two `SessionManager.open` calls accepted the same temporary JSONL file. After B flushed `second-writer-message`, A's `rewriteEntries()` changed `beforeContainsSecond: true` to `afterContainsSecond: false`. A memory-storage probe showed the same result. A normal title update and compaction append did **not** reproduce loss; the finding specifically concerns whole-file stale rewrites.

**Action:** Enforce one writable owner per canonical session path across processes, with an ownership generation and deliberate stale-owner recovery. Offer read-only attach or fork when another writer exists. If multiple writers are intended, design version/CAS reconciliation instead of silently overwriting. Release ownership on close, switch, and failed initialization; resolve aliases/symlinks consistently.

**Acceptance:** A second writer is rejected or made read-only before mutation; another process cannot overwrite the first owner's history. Crash recovery can reclaim a demonstrably stale lease. Per-instance rewrite-race tests must continue passing.

### U4 — P1: RPC convenience waits do not complete for local-only prompts or an already-idle agent

**Status: reproduced at the client helper boundary; server behavior source-confirmed.** `modes/rpc/rpc-client.ts:997-1050` makes `waitForIdle`, `collectEvents`, and `promptAndWait` depend on a future `agent_end`. `promptAndWait` does not use a local-only prompt acknowledgment. The server emits `agentInvoked: false` or `prompt_result` for commands that complete without running the agent; the client dispatch around `:1057-1120` does not turn this into helper completion.

**User scenario:** An integration sends a local slash/extension command, or calls `waitForIdle()` after a fast turn has already ended. Work is complete, but the helper waits until its timeout. A rejected prompt can also leave the separately started collection promise pending.

**Probe result:** With `RpcClient.prompt` controlled to acknowledge a local-only operation, `promptAndWait('/status', ..., 30)` rejected with `Timeout collecting events`. This isolates the helper; it was not a full CLI `/status` run.

**Action:** Use request-correlated completion that can end through agent completion, local-only result, failure, or disconnect. Subscribe before sending, then account for current state without a check/subscribe race. Cancel/unsubscribe the collector if sending fails. Avoid resolving a wait from an unrelated concurrent request.

**Acceptance:** A local-only command resolves promptly with its result; already-idle waits resolve; ordinary streaming waits until the matching run ends; prompt rejection and process exit clean up pending collectors without later unhandled rejections.

### U5 — P1: RPC cancellation waits behind slow ordinary commands

**Status: reproduced.** `modes/rpc/rpc-mode.ts:352-375` reserves immediate dispatch for UI and host-tool/URI replies. `RpcInputDispatcher:428-477` queues other commands on one promise chain. Bash launches separately, but `abort`, `abort_retry`, and `abort_bash` are not immediate control frames. A slow compaction/login/session operation can therefore delay cancellation.

**User scenario:** The user presses Stop while a control command is blocked on a service or operation. The UI sends an abort promptly, but it stays queued. This makes the user repeat Stop, close the terminal, or kill the process and risk losing useful state.

**Probe result:** A real dispatcher received `compact`, then `abort`. With the compact handler deliberately blocked, the observed dispatched commands were only `["compact"]`; abort ran after the gate was released.

**Action:** Give cancellation a validated immediate lane bound to the target run/generation, while preserving serialization for state mutations. Each long control operation should accept cancellation and an end-to-end deadline. Define bounded shutdown for commands accepted before EOF, rather than draining indefinitely.

**Acceptance:** Abort is observed within a specified local responsiveness target while compaction/login/host work is stalled. A late abort cannot cancel a subsequent run. Ordinary command ordering and side-channel deadlock avoidance remain correct.

### U6 — P1: Stats allows unauthenticated non-loopback exposure

**Status: source-confirmed; exposure deliberately not started.** `packages/stats/src/index.ts:108-127, 149, 194` accepts `--host`, explicitly advertises `0.0.0.0`, and passes it to `startServer`. `stats/src/server.ts:353-400, 418-443` binds that hostname without a token guard. API handlers include session listing, traces, and individual entries at `:286-320`. The repository security invariant requires authentication for any non-loopback stats bind. The default remains loopback, and removing permissive CORS does not authenticate direct network clients.

**User scenario:** A user follows the LAN exposure example to inspect stats from another device. Other reachable network users can request session data without credentials.

**Action:** Reject non-loopback bind without a token, or remove remote binding until an authenticated design exists. Protect sensitive reads as well as writes; browser same-origin policy is insufficient. Keep identity/reuse probes minimally informative, and make any advertised URL work with the chosen authentication flow.

**Acceptance:** `0.0.0.0`, `::`, and other non-loopback addresses fail before bind without credentials. Authorized requests succeed; unauthenticated session/entry reads fail. Loopback defaults and conflict recovery continue working. Compare with the existing guard in `metaharness/src/server.ts:265-270`.

### U7 — P2: RPC framing and command admission are unbounded

**Status: oversized physical frame reproduced; queue bound source-confirmed absent.** `modes/rpc/rpc-input.ts:47-67` calls `readLines` and parses the whole decoded line without a byte cap. `packages/utils/src/stream.ts` grows its concatenation sink while waiting for newline. `RpcInputDispatcher` retains queued tasks without a count/byte limit. Outbound protocol chunking does not enforce these inbound limits.

**Probe result:** A 2,097,199-byte physical prompt frame was accepted with `accepted: 1, errors: 0`, exceeding the advertised 1 MiB physical framing size. No memory-exhaustion payload was attempted.

**User scenario:** A pasted/generated large payload, host bug, or burst of requests causes memory growth and cancellation lag; a stream without newline never reaches the JSON parser.

**Action:** Bound line bytes before parsing and drain rejected lines to their newline so framing recovers. If large inputs are supported, specify bounded inbound chunk reassembly. Add command count/byte admission limits and explicit overload errors/backpressure. Bound event collectors by bytes or provide a streaming callback instead of retaining every full message snapshot.

**Acceptance:** Over-limit lines and never-terminated streams remain bounded; the next valid frame is processed; floods receive a useful overload response; Stop still works. Exercise legacy and negotiated protocol versions.

### U8 — P2: Unknown RPC commands lose response correlation

**Status: source-confirmed.** The default branch at `modes/rpc/rpc-mode.ts:1561-1563` calls `error(undefined, ...)`, dropping an available command ID. `rpc-client.ts` resolves pending requests by ID. This behavior is mentioned in RPC docs, but still creates an integration failure during version skew.

**User scenario:** A newer host sends a command an older agent does not support. An error is emitted, but the request remains pending until timeout instead of surfacing the incompatibility immediately.

**Action/acceptance:** Preserve a validated request ID on unknown-command errors. A client sending `{id: 'req', type: 'unsupported'}` must receive a correlated rejection immediately. Malformed frames without a trustworthy ID may still use an uncorrelated parse error. Add capability/version negotiation for optional commands.

### U9 — P2: MCP HTTP responses can be fully buffered before limits apply

**Status: source-confirmed.** `mcp/transports/http.ts:471, 501, 787` reads entire error bodies or JSON responses using `response.text()`/`response.json()`; the inspected transport has no response-byte cap. Later diagnostic sanitization and provider context/output truncation cannot prevent the earlier allocation.

**User scenario:** One buggy or compromised MCP server returns a very large body. Other integrations and the TUI share the same process and may become unresponsive or run out of memory.

**Action:** Use the central bounded stream utilities, enforce a real byte limit while reading rather than trusting Content-Length, and cap decompressed bytes, SSE line/event accumulation, and structured content. Distinguish oversized responses from recoverable connection failures; do not replay a write because its response was too large.

**Acceptance:** Oversized success/error/chunked/compressed responses terminate with a bounded, attributable error, preserving another server's active work and cancellation responsiveness.

### U10 — P2: A guest UI request does not settle when the last writable guest leaves

**Status: reproduced.** `collab/host.ts:175-194` stores a pending UI promise until answer, signal abort, or host teardown. `:521-526` removes departed peers but does not settle pending requests when no writable peer remains.

**Probe result:** After the sole writable guest disconnected, an existing request remained unsettled; explicitly aborting it cleaned up. The short observation checks an event-driven missing settlement, not a production latency estimate.

**User scenario:** The remote participant leaves while answering an ask. The remote branch stays pending until the local participant answers, the turn is cancelled, or sharing ends. The extension UI controller races a local presentation in several paths, so this is **not proof every agent turn deadlocks**.

**Action/acceptance:** Resolve the remote branch as `unavailable` when no eligible recipient remains, keeping local fallback available. Define reconnect grace deliberately if replay is intended. Do not confuse transport unavailability with an explicit guest cancellation. Test departure, role change, reconnect, local winner, and multiple guests.

### U11 — P2: Standalone stats JSON output contains non-JSON stdout

**Status: source-confirmed.** `stats/src/index.ts:180` prints `Synced ...` to stdout before the `values.json` branch emits JSON at `:184-187`.

**User scenario:** Pipe `omp-stats --json` into a JSON parser or dashboard collector; parsing fails despite a successful command. This finding concerns the standalone stats entry point, not every `harvest stats` adapter.

**Action/acceptance:** Send progress/summary to stderr in JSON mode, with exactly one parseable JSON document on stdout. Verify empty and non-empty databases, sync failure, and non-TTY invocation using the executable entry point.

### U12 — P2: New Markdown-brain reranking adds another serial Laya call to the model path

**Status: source-confirmed, concurrent feature in progress.** `sdk.ts:3419-3447` awaits pruning, then awaits `brain.transform`. `core/harvest/brain.ts:174-230` reranks whenever multiple pages match and Laya is enabled; the metadata/index refresh cache is not a reranking-result cache. No separate brain enable/rerank setting is passed from the SDK. Repeated model calls for the same user request can rescore unchanged pages after already spending time on pruning.

**User scenario:** A multi-tool task retrieves the same skill/knowledge pages each iteration. On a CPU or an overloaded sidecar, optional retrieval ranking adds another timeout/round trip before first token. A global Laya enable switch now activates this additional decision point.

**Action:** Give brain retrieval and reranking explicit effective controls and the shared decision budget. Cache ranking by query, content signatures, prompt/model/calibration revision, and scope; invalidate on edits and task changes. Preserve cheap deterministic retrieval when reranking misses its budget. Apply prompt-cache-aware context identity and measure whether improved retrieval outweighs added time. Add controls for project/user retrieval scope so users can deliberately exclude cross-project knowledge.

**Acceptance:** Unchanged query/pages reuse bounded cached ranking; edited/deleted pages invalidate it; turning reranking off preserves lexical retrieval; cancellation/offline mode remains prompt. Existing tests already cover latest-user retrieval, protocol preservation, scope separation, graph expansion, and malformed-score fallback—preserve those contracts.

### U13 — P2: The current type-check gate is broken by a new test fixture

**Status: reproduced during this review.** `bun --cwd=packages/coding-agent run check:types` failed because `test/markdown-brain.test.ts:73` supplies a `toolResult` without required `isError`. Runtime tests pass because execution does not enforce that TypeScript contract. This is concurrent work, not a reviewer edit.

**Action/acceptance:** Make the fixture conform to `ToolResultMessage` and rerun the mandated package gate. Do not weaken the production type to accommodate a malformed fixture. Recheck current source first because its author may already have fixed it.

### U14 — P1: Native fallback breaks multi-result memory recall/reflection

**Status: reproduced through existing tests and in a fresh process; fallback cause source-confirmed.** `packages/mnemopi/src/core/mmr.ts:41-63` always enters its native batch path for ordinary well-formed text and the default similarity function, then iterates the returned indices. `packages/natives/native/loader-state.js:894` intentionally falls back when an addon cannot load; its final proxy around `:2477-2492` supplies `() => null` for unimplemented exports. There is no concrete `mmrRerankIndices` fallback. `native/index.js:97` exports that callable as if it supplied the declared `Uint32Array` contract. A function-existence check would therefore not establish availability.

**User scenario:** Harvest boots on a host without a usable precompiled addon. A user stores several memories, then recalls or reflects on them. The supposedly optional native optimization throws instead of using the existing TS implementation. Single-result recall can work, making the failure appear intermittent.

**Evidence:** The existing `memory-tools.test.ts` cases “stores multiple memories and returns correct count” and “returns a synthesized text block based on recalled memories when data exists” failed at `mmr.ts:59` with `null is not an object (evaluating 'index of picked')`. In a separate Bun process, the exported native function returned `null` for three ordinary strings/scores; `mmrRerank` threw the same error. Forcing the existing JS path via a custom similarity callback returned the three results. The Rust source at `crates/pi-natives/src/vectors.rs:265` specifies a typed-array result; this does not prove that a correctly built native kernel is faulty.

**Action:** Expose trustworthy native capability availability and preserve a real TS fallback for absent/invalid kernels, or implement a contract-correct kernel fallback in the loader. Do not treat a proxy function or `null` result as successful native support. Audit other vector exports and generic dummy-class/function fallbacks for similarly false capability signals. Make the startup diagnostic identify degraded native features and the reason an addon was rejected.

**Acceptance:** Ordinary multi-result recall/reflection succeeds with a valid addon and with the addon deliberately unavailable. Results match the existing JS selection contract; unsupported native features produce useful capability errors rather than fabricated empty/null success. Run native parity and packaging/fallback tests without weakening them to accept null. Building/replacing native artifacts was not performed during this read-only review.

## Missing product controls and integration work to evaluate

These are **proposed features or explicit contract decisions**, not an assertion that all associated workflows are absent. Each is grounded in a current boundary and needs a product decision before implementation.

| ID | Proposal and user need | Existing capability / evidence | Concrete delivery criterion |
| --- | --- | --- | --- |
| F1 | Project trust before executable discovery | Extension context returns `isProjectTrusted: () => true` (`extensions/runner.ts:1180`); ambient loaders import discovered modules; MCP project config defaults on (`mcp/config.ts:103`). `docs/mcp-config.md:234-240` explicitly treats committed definitions as trusted and recommends separate profiles. | On opening an unfamiliar checkout, show executable sources/commands before importing or starting them; user trust is scoped to canonical root and reviewed changes. Headless mode has an explicit safe behavior. Denying trust does not prevent ordinary code reading. This changes the existing trust model deliberately. |
| F2 | Trust-aware MCP capability/approval classification | Every eager MCP tool declares `approval = 'write'` (`tool-bridge.ts:546`); approval docs define arbitrary execution as `exec`, and mode `write` auto-allows write. MCP tools can implement shells/browser/network actions. | Unknown MCP capabilities default conservatively, with user-configurable per-server/tool policy and audited allow overrides. A remote shell cannot inherit workspace-write permission merely because it is MCP. Treat read/idempotency annotations as hints from a trusted source, not proof. |
| F3 | One session/task budget including delegated work | Goal runtime has token-budget machinery and stats reports spend; those exist. No general user-configured monetary ceiling was found in the settings review. | Reserve/account spend across the main agent, subagents, retries, advisor, memory extraction, and fallback providers; expose estimates and unavoidable in-flight overshoot. Stop starting work at the limit and ask before raising it. Do not claim exact dollar enforcement when provider usage/pricing is delayed or unknown. |
| F4 | Privacy preview for each outbound sharing channel | Provider secret obfuscation and share-snapshot redaction exist. Collab replication/transcript reads do not use that same redaction seam; text redaction cannot inspect secrets embedded in images. | Preview scope/content/recipients before sharing; choose redaction and image exclusion; report limitations. Apply channel-specific policy to snapshots, incremental entries, transcript fetches, exports, diagnostics, telemetry, and calibration data. Raw local archives can remain an explicit option. |
| F5 | Action outcome reconciliation | U2 demonstrates an uncertain external outcome; local checkpoints/rewind already exist. | Track intent, authorization, request identity, delivery uncertainty, and confirmed remote outcome; offer reconciliation or compensating action. Clearly distinguish workspace undo from undoing an external message/deployment/database action. |
| F6 | Unified integration health and capability view | MCP statuses, `/laya` diagnostics, backend statuses, provider discovery, and worker smoke probes exist separately. | One discoverable view shows configured vs usable vs connected, active profile/endpoint, effective settings, auth expiry, last bounded failure, capabilities, and recovery. A single failing integration does not hide healthy ones. Reuse existing status events. |
| F7 | Retrieval explainability and memory correction workflow | Mnemopi already supports scoped recall plus update/forget/invalidate (`mnemopi/state.ts:284-360`); Markdown-brain now adds cited retrieval and supersession metadata. | Show why a page/fact was injected, scope/source/age, conflicts, and how to correct or exclude it. Integrate existing edit/forget APIs rather than adding a second memory store. A user's latest correction overrides stale retrieved material without silently deleting unrelated data. |
| F8 | Explicit context pipeline controls and provenance | Extensions, steering, pruning, brain, memory, secrets, image conversion, and provider replay each transform context. | A diagnostic explains transformations, retained constraints, token/byte changes, fallback reasons, and timings without revealing secrets. Use one budget/ownership contract; preserve tool-call/result linkage and explicit user intent at every transform. |
| F9 | Release and portability qualification | Worker-host smoke, native bindings, packaging scripts, and cross-platform setup exist. Actual sidecar CPU/GPU and fresh-install matrices were not run here. | Release gates cover source/npm/binary, supported OS/architectures, offline/restricted networks, non-admin Windows, missing Python/native assets, and real single-checkpoint inference. Unsupported hardware degrades with a precise message. |
| F10 | Consistent cancellation and recovery UI | Many tools already propagate signals; U5 and U10 show remaining lifecycle boundaries. | Every long operation shows its owner and cancellable phase; Stop settles local UI promptly, reports ongoing uncancellable remote work, and avoids pretending a completed external effect was rolled back. A second Stop has a deliberate force behavior. |

## Human/user scenario matrix for the implementing agent

These are **required scenarios to evaluate**, not a list of 43 discovered bugs. “Verify” means the feature exists or no failure was established here. Build tests around observable contracts, not source text or boilerplate. Prioritize the rows linked to reproduced findings.

| Journey | Human behavior / environment | Required observable behavior | Review disposition |
| --- | --- | --- | --- |
| First run | No keys, wrong provider, expired login | Explain the exact missing credential and usable next step; no destructive partial initialization | Verify provider/auth flows |
| First run | Proxy, captive portal, offline, model download blocked | Bounded startup; useful diagnostic; Laya fallback matches the decision point | Verify release matrix; P2-10 |
| First run | CPU-only, low RAM/disk, unsupported accelerator | Hardware-derived effective settings; no repeated heavy setup every prompt | P1-3, P2-1 |
| First run | Port 8177 occupied by a different healthy-looking service | Reuse only authenticated compatible Laya; choose fallback port safely | P1-8, P1-11 |
| First run | Unfamiliar repository includes extensions/MCP commands | Executable trust decision before project code runs | F1 |
| Input | Empty prompt, whitespace, typo, accidental giant paste | Harmless handling; bounded parsing and usable recovery | U7; verify TUI paste |
| Input | Double Enter or two client retries for one intent | Visible queue/request identity; no accidental repeat of external effects | U2, F5; verify submit handling |
| Input | User asks a question while agent is editing | Answer briefly and preserve active work unless clearly redirected | Verify steer/follow-up semantics |
| Input | User changes task or says earlier assumption was wrong | Latest task used; durable constraints retained; stale decisions invalidated | P0-3, P1-5; brain latest-query test passes |
| Input | Ambiguous request or conflicting instructions | Ask the missing material question; continue only independent authorized work | Verify prompt/goal policy |
| Input | User says stop, pause, resume, or cancel one subagent | Distinct operations; target-specific cancellation; unrelated work continues | U5; concurrent AgentSession test passes |
| Input | Non-English, mixed-language code/comments, transliteration | No silent loss of capabilities; explain Laya language fallback; preserve original text | Verify Laya language detection and translations |
| Input | Image/file attachment, no-vision model, huge binary | Preserve attachment identity; use existing vision fallback or explain limitation; cap work | Verify existing multimodal conversion |
| Approvals | User declines or edits arguments before approving | Refusal respected; approval bound to final args, tool, workspace, run/generation | Verify final preflight and extension revisions |
| Approvals | Approval pending while switching tabs/projects | Answer applies only to originating action/session; background badge identifies owner | P1-2; dedicated interactive validation |
| Approvals | MCP/custom shell under workspace-write mode | Execution capability obeys user policy across tool families | F2 |
| Approvals | Laya down or malformed response | Gating requires approval; optional decisions fail open; no contradictory status | Gating/pruning fixes tested; P2-6 remains partial |
| Tools | Server commits but response is lost | Outcome uncertainty surfaced; no blind non-idempotent replay | U2 |
| Tools | Two MCP servers, same normalized tool name, reconnect reorder | Stable ownership and attributable result | Existing MCP collision test passes; verify live servers |
| Tools | One server hangs/crashes while another works | Independent deadlines, fair admission, other work and Stop remain responsive | U9; verify multi-server lifecycle |
| Tools | OAuth expires mid-call/profile switches | Correct profile binding; bounded refresh; no credential leakage into project config | Existing auth recovery present; verify real flow |
| Tools | Output very large, contains ANSI/control bytes/secrets | Memory bounded before decoding; rendering sanitized; artifacts retain deliberate access | U9; verify every renderer/error path |
| Workspace | Dirty tree, user edits files during agent work | Preserve external edits; conflict detection and reviewable mutations | Verify edit/checkpoint contracts |
| Workspace | Read-only files, removed directory, disk full, permission denied | Useful error; no false success; session stays recoverable | Verify persistence/jail errors |
| Workspace | Symlink/junction, nonexistent leaf, alias path | Resolved-path jail checks before mutation; no lexical reopen escape | Main paths previously hardened; audit remaining tool entries |
| Workspace | Windows drive/UNC/long/unicode path; case aliases | Consistent canonical identity, safe cwd, actionable platform errors | Verify native + TS path matrix |
| Session | Crash/kill during streamed tool or atomic rewrite | Recovery preserves valid durable entries and explains uncertain tool outcome | U3, F5; verify crash recovery |
| Session | Same session opened in two terminals | One writer or explicit reconciliation; no silent history loss | U3 |
| Session | Rewind/fork/compact after Laya locked a drop | Stable identity/generation; new branch content scored afresh | P0-3 |
| Session | Resume after project moved/deleted | Safe cwd fallback and explicit workspace association | Existing cwd fallback present; verify user-visible result |
| Session | Multiple projects run concurrently | Session-owned settings/cwd/auth/MCP/approvals/drafts; ambient globals do not retarget work | P1-2; core concurrent-turn tests pass |
| Sharing | Share A while B and advisor are registered | Only authorized root/descendants visible; advisors remain private | U1 |
| Sharing | View-only link, forged token, writable participant leaves | Mutation denied; pending asks settle unavailable; local fallback works | Existing token tests pass; U10 |
| Sharing | Export transcript with secrets or screenshot | Preview/redaction/exclusion matches explicit channel policy | F4 |
| Stats | User exposes dashboard on LAN | Authentication required for sensitive reads; explicit bind status | U6 |
| Automation | CLI consumed by script, no TTY, piped JSON | Clean stdout, bounded requests, stable machine errors | U4, U7, U8, U11 |
| Automation | Host/agent protocol versions differ | Correlated unsupported-command error; capability discovery | U8 |
| Automation | stdin EOF/disconnect during slow command | Bounded teardown; pending requests settle; avoid leaked work | U5; verify complete shutdown |
| Memory | Stale preference, contradiction, project vs global fact | Show provenance; honor correction; scope and forgetting work predictably | Existing Mnemopi APIs; F7 |
| Memory | Addon unavailable; recall matches several memories | Real fallback keeps recall/reflection functional; report degraded native capability | U14 |
| Memory | User selects memory off or excludes user knowledge | Explain distinct brain/backend controls; no unexpected retrieval or remote retention | F7/F8/U12; product decision |
| Accessibility | Keyboard-only, small terminal, resize, screen reader, no color | Input/actions discoverable; stable focus; information does not depend solely on color/animation | Dedicated TUI/manual validation pending |
| Long work | Cost limit, repeated failure, sidecar overload, no progress | Bound retries/work; show reason; stop launching optional work over budget | F3, P2-1; goal budgets already exist |
| Packaging | Binary worker or native module fails after install | Runtime smoke catches actual protocol/lifecycle failure; clear fallback | Existing worker-host invariant; F9 |

## Laya decision-loop tuning plan

### Preserve decision ownership

Laya should supply a typed recommendation at an explicit decision boundary. The agent's deterministic policy owns approvals, workspace scope, provider availability, user model choice, budgets, and tool protocol integrity. Learned confidence cannot override an explicit deny, user cancellation, or missing authorization. Routing must respect the existing KDL/provider policy and credential/model availability; keep model-name rules out of TypeScript.

Represent each invocation with session/root generation, active task version, question schema and prompt revision, calibration/model revision, deadline, and decision-point fallback. Return answer validity, confidence, abstention reason, and timing separately. Use the existing client/logging infrastructure rather than proliferating wrappers.

### Schedule only useful work

| Decision point | Current problem | Recommended scheduling | Failure behavior |
| --- | --- | --- | --- |
| Tool gating | Fixed tool names; unproven coding calibration | Run at final approved/executable args boundary; prioritize over optional scoring; reuse only identical intent/args/policy generations | Require human approval; an already-cancelled action is not executed |
| Pruning | Below-budget scoring; oversized batches; stale locks/goal | Estimate need first; preserve protocol and durable constraints; score bounded new candidates by stable identity | Keep uncertain content; budget-aware compaction remains a separate contract |
| Subagent shadow | Awaited telemetry | Dispatch immediately; bounded best-effort shadow queue; drop obsolete/cancelled requests | Use caller/default agent |
| Subagent auto-pick | Calibration/threshold provenance | Apply after roster/spawn/permission filters and explicit user selection; validate chosen candidate | Existing deterministic/default choice |
| Model routing | Helper currently unwired | Integrate once at an explicit selection boundary only if product-approved; avoid per-iteration model oscillation | Preserve valid user/configured model |
| Unexpected stop | Extra success question; missing task/signal | Ask only needed question; include task progress and real outcome; propagate cancellation | Existing configured classifier/heuristic |
| Brain reranking | Another serial call per unchanged request | Deterministic retrieval first; optional cached rerank under shared budget | Keep lexical/graph rank |

Add an end-to-end scheduler budget covering queue wait, token read, serialization, inference, decoding, and every optional decision in the model-call path. Avoid raising all timeouts as a CPU workaround: calibrate useful features, skip optional work, and retain conservative gating. A circuit breaker can suppress optional calls after repeated timeout/overload; it must not turn unavailable gating into autonomous permission. Bound queue length and expire obsolete tasks before inference.

### Calibrate for real coding behavior

1. Define observable labels by decision: irreversible-effect detection, relevant-context retention, dispatch task outcome, premature continuation/completion, and retrieval benefit. A default/caller agent pick is a baseline, not ground truth.
2. Sample consented traces across tool families, supported providers, CPU/GPU hardware, task lengths, error paths, misleading output, cancellations, changed goals, and mixed-language content. Sanitize before retaining traces; do not automatically commit them.
3. Split by session/project and time so near-duplicate histories do not leak into train/calibration/test. Compare with deterministic baselines and Laya-disabled runs. Review disagreements manually, especially confident false clears and required-context drops.
4. Calibrate each question family with versioned prompt/checkpoint/data provenance; evaluate abstention and selective coverage. Treat thresholds as empirical results with uncertainty. For example, zero observed false clears in a small sample is not proof of a zero false-clear rate.
5. Use shadow/canary rollout with a rollback switch and outcome metrics before auto-pick or pruning becomes default. Preserve explicit user overrides and expose configured/derived/effective settings.
6. Only consider actual model weight fine-tuning if the public Laya/checkpoint training interface and licensing support it, with a separate offline job and held-out evaluation. This review provides a tuning plan; it did **not** train or change a checkpoint. Preserve the single-checkpoint `laya.load(...)` invariant and never introduce `Router` downloads.

## Code quality and latency improvements across the project

| Area | Improvement | Why it helps / validation |
| --- | --- | --- |
| State ownership | Explicit session/run generations for global registries, pending approvals, pruning, callbacks, and shared-room authorization | Prevents cross-tab actions and stale completions; test racing navigation/cancel/revival with distinct observable effects |
| Async lifecycles | One deadline/cancel contract from entry to leaf; bounded admission and teardown; compute capacity tracks actual completion | Removes hidden queue waits and leaked tasks; measure cancel-to-idle and in-flight counts, not just request timeout |
| Protocol boundaries | Validate inbound/sidecar/provider/MCP data before using typed casts; preserve correlated errors | Type assertions do not validate bytes from another process; verify malformed/wrong-version/partial frames |
| Capability metadata | Shared trusted capability and side-effect model for tools, with user policy overrides | Aligns built-in, MCP, custom, browser, SSH, and LSP approvals; avoids wire-name risk lists |
| Context transforms | Single ordered pipeline with provenance and a shared latency/token budget | Makes pruning/memory/brain/images/extensions measurable; preserve linkage and instruction priority |
| Caching | Cache only with scope, identity, signatures, credentials/policy generation, and bounded lifetime | Avoids repeated scanning/ranking/discovery without stale routing or cross-project reuse; measure hits and invalidations |
| Hot paths | Profile startup imports, filesystem scans, transcript rebuild, token estimation, image conversion, memory recall, and provider serialization | Investigate measured bottlenecks; use lazy imports where appropriate and bounded parallel reads; never infer gains from code size |
| Persistence | Cross-process writer ownership plus append/rewrite contracts | Atomic writes alone cannot prevent U3; benchmark long-session rewrite and recovery |
| Native/worker packaging | Preserve worker-host entry/fallback; smoke real request/response and teardown in compiled artifacts | Avoids source-only success; native fast paths require a functional fallback and measured end-to-end benefit |
| Maintainability | Reuse domain/central utilities, static prompt assets, explicit types, structured logger, KDL model policy | Prevents divergence; enforce structural rules via lint/type tooling rather than source-grep tests |
| Observability | One trace connects prompt → transforms → provider → tools → decisions → persistence | Separates model time, local overhead, queue wait, and user approval time; redact all trace payloads |
| Test quality | Add commit-after-response-loss, scope escape, competing writer, and lifecycle transition contracts | Existing 110 passing tests do not cover these failures; prioritize behavioral reproductions over wiring/constant assertions |
| Documentation | Effective settings, executable trust, undo limits, local-only RPC completion, and partial feature wiring | Reduces surprising user behavior; derive claims from exercised contracts and keep unsupported paths explicit |

**Performance experiments, in order:** remove awaited shadow telemetry; skip no-op pruning; bound/circuit-break optional calls; cache unchanged brain ranking; profile startup discovery and session-render costs; measure memory and provider preprocessing; then consider batching/native acceleration only for measured bottlenecks. Parallelize independent reads or pure work, while preserving mutation, approval, tool-result, and context-order dependencies. Keep network/provider latency, cold startup, warm turn latency, user approval time, and CPU/GPU contention separate. No percentage speedup is justified by this audit alone.

## Updated implementation handoff order

1. Rebase findings onto the current checkout and preserve other agents' work. Fix the type-check fixture if still broken. Retain concurrent gating/pruning validation improvements and re-evaluate tab wiring after it lands.
2. Reproduce U1/U2/U3 with focused behavioral regression tests, then fix collab scope, uncertain MCP replay, and session writer ownership. Address non-loopback stats authentication (U6) before advertising remote access. Restore memory recall/reflection in native fallback mode (U14).
3. Fix request completion/cancellation/correlation and bounded admission in RPC (U4/U5/U7/U8); add HTTP body caps, guest-request settlement, and clean stats JSON (U9/U10/U11).
4. Resolve remaining Laya correctness/ownership issues before threshold tuning. Remove optional serial work, finish central validation, and provide effective settings plus measurement.
5. Decide F1–F10 deliberately, with explicit product contracts. Complete live-session and routing integration only with session-scoped approvals/settings/cwd and user model precedence proven.
6. Run focused package gates and behavioral scenarios for each change. Run broader provider/platform/packaging matrices where the changed contract requires them. Report failures and unverified real-service behavior accurately; do not declare the entire project correct from a passing type check or mocked tests.

### Second-pass validation ledger

| Check | Result | Limit |
| --- | --- | --- |
| Five existing integration/runtime test files | 78 passed, 0 failed | Controlled/local fixtures; no real external write |
| Four current Laya/brain test files | 32 passed, 0 failed | Live sidecar probe skipped; no inference accuracy benchmark |
| Four provider/catalog/memory test files | 73 passed, 2 failed | Both failures enter native MMR fallback from multi-result memory recall/reflection |
| Package type check | Failed: `markdown-brain.test.ts:73`, missing `isError` | Concurrent work may fix this after review |
| Typed MCP receive EOF after committed effect | Two effects for one call | Controlled transport; production retry metadata used |
| Collab read-only scope probe | Unrelated root disclosed; unrelated and advisor transcript returned | Real host/socket/crypto with in-memory relay |
| Competing session writers | Second writer's persisted entry disappeared after stale rewrite | Temporary real file and separate memory-storage reproduction |
| RPC blocked command followed by abort | Abort remained queued | Real dispatcher with controlled blocking handler |
| RPC acknowledged local-only prompt | Helper timed out | Controlled acknowledgment; actual wait helper |
| RPC 2 MiB physical input frame | Accepted, zero parse errors | No memory-exhaustion attack attempted |
| Last writable guest leaves during UI request | Promise remained pending until explicit abort | Existing local UI race can still resolve the user interaction |
| Fresh-process native MMR probe | Kernel export returned null; wrapper threw; existing JS path completed | Host uses native fallback; a working native binary was not rebuilt/tested |
| Full interactive/cross-platform/real-provider/real-model checks | Not performed | Must remain release/integration follow-up work |
