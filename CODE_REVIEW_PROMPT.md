# Comprehensive Code Review & Debugging Prompt: Harvest & Laya Ecosystem

> **Instructions for the Reviewing Agent:**
> Copy and execute the review instructions below. You are tasked with performing an exhaustive, adversarial, and deeply technical architectural review and debugging session of this codebase. Pay particular attention to all **execution harnesses** and thoroughly **debug the integration with the Laya local decision layer**.

---

```markdown
# MISSION BRIEFING: Senior Principal Systems & Security Code Review

You are acting as an elite Principal Systems Architect, Compiler Engineer, and Security Researcher. Your objective is to perform a thorough, rigorous, adversarial code review across the entire **Harvest 2.0 (`pi-mono`)** repository.

This project is a high-performance coding agent, multi-harness orchestration environment, and local AI runtime built with Bun, TypeScript, Rust native extensions (via N-API), and Python machine learning microservices.

Your review must hold this codebase to the highest engineering standards: zero-tolerance for silent data corruption, protocol violations, symlink escapes, unauthenticated network controls, unbounded memory consumption, uncaught type errors, or broken fallback contracts.

---

## 1. REPOSITORY LANDSCAPE & CONTEXT

### Package Architecture
| Package / Path | Responsibility & Criticality |
|---|---|
| `packages/coding-agent/` | **Primary focus.** Main CLI application, interactive TUI, tool executors, subagent dispatch, LSP client, evaluation, and autoresearch. |
| `packages/agent/` | Core agent runtime: state machine, execution loop (`agent-loop.ts`), turn persistence, append-only context, and compaction. |
| `packages/metaharness/` | Benchmark orchestration harness (SWE-bench runner, Harbor run storage, REST/SSE dashboard). |
| `packages/tui/` | Terminal UI library with differential rendering, Kitty keyboard protocol, SGR mouse routing, and headless process terminal render harnesses. |
| `packages/ai/` | Multi-provider LLM transport layer, auth-gateway, auth-broker, and streaming EventStream/SSE parsers. |
| `packages/catalog/` | Model catalog, provider descriptors, and KDL-based compatibility rule cascades. |
| `packages/omptype/` | ArkType-compatible schema validation with a lazy JIT runtime. |
| `packages/mnemopi/` | Persistent memory SQLite database, beam storage, and MCP server. |
| `packages/utils/` | Shared utilities, virtual terminal (`vterm`), process trees, and streaming primitives. |
| `crates/*` | Rust native crates (`pi-natives`, `pi-edit`, `pi-vcs`, `pi-shell`, `pi-iso`, `pi-walker`, `pi-voice`). |
| `python/robomp/` | GitHub issue/PR lifecycle automation and lifecycle cards verification harness. |
| `decision-sidecar/` | Laya local decision microservice: ModernBERT (`convaiinnovations/laya-typed-decisions`), FastAPI, hardware probing, and temperature calibration. |

### Strict Repository Invariants (`AGENTS.md`)
You MUST enforce these rules when reviewing:
1. **Bun Over Node**: Spawning shell commands for operations with native Bun or `node:fs/promises` APIs is prohibited (e.g. no `spawnSync(["mkdir", "-p"])`, use `node:fs/promises`).
2. **Model/Provider Policy Lives in KDL**: Never hardcode model or provider conditionals in TypeScript (no `id.includes("claude")`, no regex matching on model names). All policy lives in `packages/catalog/src/compat/rules/*.kdl`.
3. **No Direct Edits to Generated Files**: `models.json` and `rules.json` are auto-generated.
4. **Logging & CLI Output**: Code active during TUI, RPC, SDK, or worker runtime MUST use `logger` from `@harvest/pi-utils`, NEVER `console.log`/`error`/`warn` (which corrupts screen rendering and wire protocols).
5. **TUI Sanitization**: All content rendered to the terminal MUST pass through sanitization helpers (`replaceTabs()`, `truncateToWidth()`, `shortenPath()`, `PREVIEW_LIMITS`).
6. **Worker Re-entry Contract**: Workers must re-enter the CLI host entrypoint via `workerHostEntry()` from `@harvest/pi-utils`.
7. **Testing Rules**: Never use `mock.module()` (mutates global module registry). Tests must prove concrete external contracts, not static echoes or tautologies.

---

## 2. DEEP-DIVE: DEBUGGING THE LAYA DECISION LAYER INTEGRATION

The Laya integration embeds a local, 421M parameter ModernBERT classification model (`convaiinnovations/laya-typed-decisions`) running over loopback (`127.0.0.1:8177`) to handle fast sub-agent selection, context pruning, tool gating, and step completion without cloud LLM round-trips.

**You must meticulously debug both sides of this integration: the Python Decision Sidecar and the TypeScript Client/Services.**

### A. Decision Sidecar (`decision-sidecar/`)
1. **Model Loading & Checkpoint Pinning (`server.py`)**:
   - Verify that `laya.load(MODEL_ID, device=_device)` loads only the single checkpoint `convaiinnovations/laya-typed-decisions`.
   - Ensure the `Router` construct is NEVER imported or initialized, preventing unprompted background downloads of secondary checkpoints (like `laya-multilingual`).
   - Audit the English language gate (`laya.is_english(state)`); verify that non-English characters immediately bypass local inference and signal fallback.
2. **Request Payload & Batch Size Limits (`server.py`)**:
   - Inspect `/v1/decide` and `DecideRequest`. Audit whether the request enforces limits on `state` string length, question dictionary size, or total token count.
   - Trace memory allocation during batch collation (`collate_items`, `build_sequence`). Can a client submit an unbounded batch causing an OOM crash or CPU starvation?
3. **CORS & Network Security Boundaries (`server.py`)**:
   - Inspect the CORS middleware configuration. Why are default HTTP localhost ports (`http://localhost`, `http://127.0.0.1`) allowed without authentication?
   - Verify whether unauthenticated web browsers can trigger local inference or poison the decision audit log (`decisions.jsonl`).
4. **Calibration Engine Disconnect (`server.py` vs `calibration.py`)**:
   - Inspect `server.py`: Does it ever import `CalibrationManager` or construct an instance? Does it load `calibration_params.json` at startup as claimed in `README.md`?
   - Audit the confidence scoring in `/v1/decide`: Does it actually apply temperature-scaled calibration (`get_calibrated_confidence`), or does it return raw checkpoint confidence?
   - In `calibration.py`: Investigate the synthetic data generation logic. Does it append 450 synthetic records when < 50 real records exist? Does this pollute real production parameters with synthetic artifacts?
   - Check CLI argument parsing in `calibration.py`: Does `argparse` support `--decisions` and `--out` as advertised in `README.md`, or only `--synthetic`?
5. **Hardware Detection & Platform Compatibility (`hardware.py` & `test_hardware.py`)**:
   - Debug why `test_hardware.py` fails on non-macOS systems (e.g. Windows). Check `platform.mac_ver()` mocking: does unmocked `platform.mac_ver()` cause macOS detection to fail with `macOS is below required macOS 14+`?
   - Verify Apple Silicon MPS fallback to CPU when tensor allocation fails (`mock_torch.zeros.side_effect = RuntimeError(...)`).
6. **Length Bucketing Optimization (`test_bucketing.py`)**:
   - Audit `bucket_items_by_length()`. Verify numerical equivalence with unbucketed collation and check edge cases (empty list, single item, extreme length ratios).

### B. TypeScript Coding-Agent Integration (`packages/coding-agent/src/core/harvest/`)
1. **Compilation & Type Check Failures (`bun run check:types`)**:
   - Run and debug TypeScript type check errors in:
     - `packages/coding-agent/test/laya-pruning.test.ts`
     - `packages/coding-agent/test/laya-subagent-selection.test.ts`
   - Investigate why test mocks fail with `Property 'success' is missing in type ... but required in type 'DecisionResult<...>'`.
   - Fix invalid type references: `AgentSource` using `'bundle'` instead of `'bundled'`.
   - Fix missing properties: `AgentDefinition` missing `description`.
   - Fix non-existent methods being called on `LayaClient` (e.g. `batchDecide`, `isAvailable`).
2. **Context Pruning Protocol Corruption (`laya-pruning.ts`)**:
   - **Critical Protocol Bug**: Trace `pruneContextWithLaya()` when evaluating older assistant messages. If an assistant message contains both large text and `toolCall` blocks, what happens when it is pruned?
   - Does replacing the content with a pruned placeholder strip the `toolCall` while leaving corresponding `toolResult` messages orphaned in history?
   - What happens when an upstream provider (Anthropic, OpenAI) receives a `toolResult` without a preceding `toolCall`? (It throws a 400 Bad Request error).
   - Trace how `LayaPruningOptions` handles cancellation. Does `pruneContextWithLaya()` declare and accept an `AbortSignal`? Does `LayaClient.decide()` propagate the signal to the fetch request, or does it only create an internal timeout controller?
   - Trace `laya.pruningTokenBudget` from `settings-schema.ts`. Is this setting ever read by the SDK or pruning engine? Why is it defaulting to 40% candidate tokens instead of the configured budget?
3. **Subagent Selection Policy Bypass (`laya-subagent-selection.ts`)**:
   - Trace `resolveEffectiveSubagentPolicy()` and `selectSubagentWithLaya()`.
   - What happens if Laya selects an agent that is disallowed by the session's `spawns` allowlist or listed in `task.disabledAgents`? Does it throw a fatal error or gracefully fall back to the default agent?
   - Audit case-sensitivity in agent resolution: `buildSubagentCriteria()` lowercases agent names and returns lowercased picks, but `getAgent()` performs case-sensitive lookups. Can mixed-case agent names (e.g., `CodeGen`) fail resolution?
4. **Daemon Lifecycle & Client Resilience (`laya-service.ts` & `laya-client.ts`)**:
   - Audit daemon startup, PID tracking, and port probing on `127.0.0.1:8177`.
   - Verify timeout and retry contracts: if the daemon is unreachable or unresponsive (>300ms), do all callers adhere to their fallback policies (Tool Gating: Fail CLOSED; Routing: Fail OPEN; Pruning: Fail OPEN)?

---

## 3. AUDITING ALL EXECUTION & ORCHESTRATION HARNESSES

You must systematically inspect every harness in the repository against the technical criteria below.

### Harness 1: Agent Runtime Execution Loop (`packages/agent/src/agent-loop.ts`, `agent.ts`)
- **State Machine Transitions**: Audit the step lifecycle (`start` -> `toolExecution` -> `messageTransform` -> `turnComplete`). Can an unhandled promise rejection in a tool stream leave the agent loop permanently deadlocked?
- **Cancellation & Abort Signals**: Audit turn cancellation. Does aborting an agent turn cleanly interrupt active tool execution, background streams, and child processes? Are tool results paired with abort errors in message history?
- **Context Synchronization & Compaction**: Inspect `append-only-context.ts` and `compaction.ts`. Does compaction properly invalidate message caches without desynchronizing message digests?

### Harness 2: Subagent Orchestration & Dispatch Harness (`packages/coding-agent/src/task/`)
- **Depth & Recursion Bounding**: Inspect `structured-subagent.ts`, `executor.ts`, and `spawn-policy.ts`. Are subagent spawn depths strictly bounded to prevent infinite recursive self-delegation?
- **Spawn Permissions & Isolation**: How are permissions, read-only modes, and allowed toolsets inherited by child tasks? Can a subagent bypass restrictions imposed on the parent?
- **IPC & Result Streaming**: Audit `workpool.ts` and `worktree.ts`. How are stdout/stderr and structured results captured and yielded back to the parent task? Trace error attribution when a subagent crashes.

### Harness 3: Benchmark & Evaluation Harnesses (`packages/metaharness/`, `packages/coding-agent/src/eval/`, `packages/coding-agent/src/autoresearch/`)
- **Metaharness Benchmark Runner (`packages/metaharness/src/server.ts`, `runner.ts`)**:
  - Audit network interfaces: Does `Bun.serve()` bind to `127.0.0.1` or default to `0.0.0.0`?
  - Audit API security: Are benchmark launch, cancel, and delete endpoints authenticated? Can a local or LAN client trigger arbitrary benchmark execution?
- **Eval Persistent Kernel Bridge (`packages/coding-agent/src/eval/`)**:
  - Inspect `agent-bridge.ts`, `kernel-base.ts`, and `py/kernel.ts`.
  - The eval harness lets Python and JS kernels call back into agent tools (`tool.read`, `tool.search`, `tool.task`) over a loopback bridge. Audit authorization and path validation on these callbacks.
  - Review `runner-cache.ts`: How is `stageRunnerScript()` isolating temporary script files? Can a local user pre-create files in `os.tmpdir()` to hijack kernel execution?
- **Autoresearch Benchmark Harness (`packages/coding-agent/src/autoresearch/tools/run-experiment.ts`)**:
  - Inspect benchmark run logging. Does the experiment runner stream unbounded benchmark stdout/stderr into `benchmark.log`? Does it read the entire file into memory with `fs.promises.readFile()`, risking out-of-memory errors on high-verbosity benchmarks?

### Harness 4: Tool Execution & Sandbox Containment Harness (`packages/coding-agent/src/tools/`, `crates/pi-edit/`)
- **Workspace Jail & Symlink Traversal (`security.ts`, `assertPathJailed`)**:
  - Trace `assertPathJailed()` across `write.ts`, `edit/index.ts`, and `lsp/writethrough.ts`.
  - Can writing to a nonexistent leaf under an existing out-of-tree symlink pass lexical checks and overwrite external files?
  - Does `op === "delete"` in `EditTool` bypass `assertPathJailed()`?
  - Can LSP `applyWorkspaceEdit` target arbitrary absolute file URIs outside the workspace root?
  - Audit the plan-mode `local://` sandbox in `crates/pi-edit/src/path_policy.rs`: Can an in-sandbox symlink point to external files and be edited via `EditSession::apply`?
- **PTY & Virtual Terminal Buffer Bounds (`packages/utils/src/vterm/terminal.ts`)**:
  - Inspect the CSI escape parser. What happens when a child process emits an unterminated `ESC [` sequence followed by millions of parameter bytes? Does `#sequence` grow indefinitely in memory?
- **Web Search & Scraper Harness (`packages/coding-agent/src/web/search/providers/browser-page.ts`)**:
  - Audit `fetchHtmlPage()` and the headless browser fallback. Are fetched HTML response bodies capped by size, or can a multi-gigabyte payload exhaust memory?

### Harness 5: TUI Render & Terminal Protocol Harness (`packages/tui/`, `packages/utils/src/vterm/`)
- **Process Terminal Render Harness (`packages/tui/test/process-terminal-render-harness.ts`)**:
  - Audit terminal emulation correctness: Kitty keyboard protocol DA1 response ordering, SGR mouse tracking, and differential screen updates.
- **TUI Debug Socket Vulnerabilities (`packages/tui/src/debug-server.ts`)**:
  - Inspect `OMP_TUI_DEBUG` socket initialization: Does `start()` blindly call `unlinkSync()` on any existing file at that path?
  - Audit socket line buffering: Are client request lines bounded in length, or can a local client send an unterminated stream and exhaust memory?
- **Terminal Screen Pollution**:
  - Ensure background servers (like `packages/stats/src/server.ts`) do not call `console.log` / `console.error` during active TUI rendering.

### Harness 6: Python Lifecycle & Workflow Automation Harness (`python/robomp/`)
- **Robomp Verification Harness & Dashboard (`python/robomp/src/server.py`, `dashboard.py`)**:
  - Audit Docker Compose bindings: Is port `6543` exposed on `0.0.0.0`?
  - Does the unauthenticated `/` HTML dashboard leak privileged credentials (`ROBOMP_REPLAY_TOKEN`) into script tags?
  - Are operational APIs (`/api/status`, task replay endpoints) accessible without authentication?

### Harness 7: Rust Native Acceleration Harness (`crates/*`, `packages/natives/`)
- **VCS Patch Application (`crates/pi-vcs/src/git/patch.rs`)**:
  - Does `write_worktree_entry()` follow symlinked parent directories outside the repository root when applying patches?
- **Isolation Diffs (`crates/pi-iso/src/diff.rs`)**:
  - In non-Git workspace diffs, does `plain_change()` follow symlinks and disclose the contents of external files in the returned unified diff?
- **Native Audio Buffering (`crates/pi-voice/src/audio.rs`, `crates/pi-natives/src/audio.rs`)**:
  - Does `PlaybackWriter::write()` or microphone capture allocate unbounded `Vec`s without backpressure or queue capacity limits?
- **Workspace Entry Traversal (`crates/pi-natives/src/workspace.rs`)**:
  - Does `list_workspace()` collect and sort the entire filesystem tree into memory before truncating to `MAX_ENTRIES`?

### Harness 8: Background Async Job & Worker Harness (`packages/coding-agent/src/async/job-manager.ts`, `workerHostEntry`)
- **Capacity & Concurrency Accounting**:
  - When a job is cancelled via `cancel()` or `cancelAll()`, is its concurrency slot freed before its async `run()` promise settles?
  - Can repeated cancellations of unresponsive jobs cause concurrent execution to exceed configured limits?
- **Result Delivery Queues**:
  - Does `#deliverDelivery()` await sink callbacks without deadlines or concurrency bounds? Can stalled sinks cause memory leaks in `#inFlightDeliveries`?
- **Worker Host Contract**:
  - Verify all spawned worker threads use the unified re-entry pattern via `workerHostEntry()` from `@harvest/pi-utils`.

---

## 4. REVIEW METHODOLOGY & VERIFICATION PROCEDURES

Your review must not rely solely on static speculation. Run concrete verification tools and checks:

1. **Type Safety & Compiler Diagnostics**:
   ```sh
   # Coding-agent TypeScript type check
   bun --cwd=packages/coding-agent run check:types
   # Full workspace type check
   bun run check:ts
   ```
2. **Laya Test Suite Execution**:
   ```sh
   # Coding-agent Laya unit & integration tests
   bun test packages/coding-agent/test/laya-*.test.ts
   # Decision sidecar Python tests
   python -m unittest discover -s decision-sidecar -p "test_*.py"
   ```
3. **Repository Linting & Static Analysis**:
   ```sh
   oxlint .
   bun run lint:py
   ```
4. **Rust Native Test Suite** (if Cargo is present):
   ```sh
   bun run test:rs
   ```

---

## 5. OUTPUT DELIVERABLE FORMAT

Structure your findings in a clear, actionable format matching the structure of `CODE_REVIEW.md`.

For each issue identified, format as follows:

```markdown
### [P<0-3>] <Concise, High-Impact Finding Title>

**Locations:** [`<file_path>`](<file_path>#L<start>-L<end>)

**Description:**
<Detailed technical breakdown explaining the mechanism of failure, race condition, missing check, or design flaw.>

**Impact:**
<Concrete consequences: security escalation, data loss, OOM crash, protocol error, typecheck failure, or bypassed policy.>

**Reproduction / Code Proof:**
<A minimal code snippet, test case, or sequence of calls demonstrating the bug.>

**Recommendation & Fix:**
<Clear, drop-in replacement code or architectural refactoring to permanently resolve the issue according to repository conventions.>
```

### Severity Guidelines:
- **[P0] Blocker / Critical**: Remote code execution, arbitrary host file overwrites, credential exfiltration, active workspace escape.
- **[P1] High**: Unauthenticated network control endpoints, wire protocol corruption (e.g. orphan tool results), broken fallbacks, static type check / compiler failures.
- **[P2] Medium**: Memory leaks, unbounded buffers, ignored configuration settings, concurrency slot leaks, synthetic calibration contamination.
- **[P3] Low**: Documentation drift, CLI argument discrepancies, UI screen pollution, boundary contract edge cases.

Begin your review now, covering all harnesses with extreme technical rigor and giving top priority to debugging the Laya decision layer integration.
```
