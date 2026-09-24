# Code Review Findings

Review date: 2026-09-24

## Findings

### [P2] JSON Schema `oneOf` accepts values that match multiple branches

**Location:** [`packages/omptype/src/from-json-schema.ts`](packages/omptype/src/from-json-schema.ts#L91-L94)

The importer selects `node.anyOf ?? node.oneOf` and lowers either keyword to the same union IR. A JSON Schema `oneOf` requires exactly one matching branch; union validation only requires at least one. For example, an input value matching both `{ type: "number", minimum: 0 }` and `{ type: "number", maximum: 10 }` passes the imported schema even though it violates `oneOf`.

**Impact:** Consumers that use `fromJsonSchema` for validation can accept documents the source schema rejects, weakening schema-based input validation.

**Recommendation:** Preserve `oneOf` as an exactly-one composition in the IR/runtime or reject it explicitly as unsupported instead of treating it as `anyOf`.

### [P2] JSON Schema `required` names without `properties` are ignored

**Location:** [`packages/omptype/src/from-json-schema.ts`](packages/omptype/src/from-json-schema.ts#L170-L182)

`#lowerObject` only creates `PropIR` entries by iterating `node.properties`. The `required` set changes optionality for those entries, but required names absent from `properties` never become checks. Thus `{ type: "object", required: ["token"] }` imports as an object with no required properties and accepts `{}`.

**Impact:** Imported schemas can accept objects missing required keys, allowing incomplete or invalid data through downstream validation.

**Recommendation:** Emit required-property presence checks for every name in `required`, including names without a corresponding `properties` schema, or reject this supported JSON Schema form explicitly.

### [P2] The local decision sidecar has no request or batch-size limits

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L103-L118), [`decision-sidecar/server.py`](decision-sidecar/server.py#L168-L207)

`DecideRequest` accepts an unbounded state string/object/list and an unbounded questions dictionary. The per-question path serializes/tokenizes each supplied state and creates one model item per question before building batch tensors; there is no payload-byte, question-count, or aggregate-token limit. The unauthenticated loopback endpoint also allows cross-origin browser requests from its configured localhost origins.

**Impact:** A malicious or compromised web application served from an allowed local origin can submit an oversized state or large question batch and consume enough CPU or memory to stall or terminate the sidecar, disrupting all local decision calls.

**Recommendation:** Enforce a request-body limit, cap question count and aggregate state size, and reject inputs that exceed the model's supported token budget before tokenization and batch allocation. Consider a bounded inference semaphore so concurrent requests cannot multiply the allocation cost.

### [P2] The sidecar never loads or applies its calibration parameters

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L38-L42), [`decision-sidecar/server.py`](decision-sidecar/server.py#L67-L103), [`decision-sidecar/calibration.py`](decision-sidecar/calibration.py#L171-L228), [`decision-sidecar/README.md`](decision-sidecar/README.md#L96-L109)

`CalibrationManager` can fit and save per-call-site temperatures, and the README says the sidecar loads `calibration_params.json` at startup. The server neither imports nor constructs that manager, and it does not read the generated file or call `get_calibrated_confidence` before returning answers. Both the custom batched branch and `_agent.predict` therefore return confidence based only on the checkpoint's own temperatures.

**Impact:** Running the documented calibration step does not change any decision confidence or downstream gating behavior, despite the README describing calibration as active.

**Recommendation:** Load and apply the call-site calibration parameters in the response path (with explicit handling for each answer type), or remove the claim that offline calibration affects live decisions.

### [P3] Decision-sidecar documentation advertises unsupported CLI and environment options

**Locations:** [`decision-sidecar/README.md`](decision-sidecar/README.md#L84-L101), [`decision-sidecar/server.py`](decision-sidecar/server.py#L38-L42), [`decision-sidecar/server.py`](decision-sidecar/server.py#L299-L311), [`decision-sidecar/calibration.py`](decision-sidecar/calibration.py#L231-L235)

The README says `LAYA_HOST` configures the bind host and `LAYA_LOG_DIR` selects a log directory, but the server reads neither variable; it only reads `LAYA_PORT` and a `LAYA_LOG_FILE` path. The documented calibration command passes `--decisions` and `--out`, but the parser defines only `--synthetic`, so the documented invocation exits with an argparse error.

**Impact:** Operators following the README cannot configure the advertised host/log location or run the documented calibration command; the service may continue binding to loopback and writing to its default log path.

**Recommendation:** Implement the advertised options or correct the README with the actual CLI arguments and environment-variable names.

### [P1] The roboomp dashboard exposes its control token and private operational data

**Locations:** [`python/robomp/docker-compose.yml`](python/robomp/docker-compose.yml#L110-L111), [`python/robomp/src/server.py`](python/robomp/src/server.py#L789-L800), [`python/robomp/src/dashboard.py`](python/robomp/src/dashboard.py#L115-L130)

The default Compose port mapping binds the dashboard to `0.0.0.0:6543`. The unauthenticated `/` handler embeds `ROBOMP_REPLAY_TOKEN` in the returned HTML, and `/api/status` plus other operational read routes are unauthenticated. A remote party who can reach the published port can retrieve the replay token and use token-gated replay/trigger endpoints; even when the token is unset, operational endpoints disclose issue and event details.

**Impact:** Network clients that can reach the Docker host can inspect private issue/event data and, when replay is enabled, obtain the credential intended to protect manual bot actions.

**Recommendation:** Require authentication for the dashboard and every operational API, and avoid returning the privileged token to unauthenticated clients. Bind the published port to loopback by default unless an authenticated reverse proxy is configured.

### [P2] Unauthenticated auth services can listen on non-loopback interfaces

**Locations:** [`packages/coding-agent/src/cli/auth-gateway-cli.ts`](packages/coding-agent/src/cli/auth-gateway-cli.ts#L178-L181), [`packages/ai/src/auth-gateway/server.ts`](packages/ai/src/auth-gateway/server.ts#L846-L854), [`packages/ai/src/auth-gateway/http.ts`](packages/ai/src/auth-gateway/http.ts#L84-L96), [`packages/ai/src/auth-broker/server.ts`](packages/ai/src/auth-broker/server.ts#L53-L59), [`packages/ai/src/auth-broker/server.ts`](packages/ai/src/auth-broker/server.ts#L100-L106), [`packages/ai/src/auth-broker/server.ts`](packages/ai/src/auth-broker/server.ts#L647-L682)

The auth-gateway CLI accepts both `--no-auth` and an arbitrary `--bind` value. It passes an empty bearer-token list to `startAuthGateway`, and `isAuthorized` treats an empty list as allowing every request. The exported `startAuthBroker` API likewise accepts an empty `bearerTokens` list and arbitrary `bind`; its option comment says empty auth is for loopback only, but the startup path does not enforce that restriction. Both servers bind to the requested hostname without checking that unauthenticated mode is restricted to loopback.

**Impact:** An unauthenticated gateway exposed on a wildcard or LAN bind lets reachable clients submit billable model requests using the host's provider accounts. An unauthenticated broker exposed the same way lets clients call credential-management APIs backed by the broker's auth store.

**Recommendation:** Reject empty-token configurations unless the resolved bind address is loopback, and enforce this in both `startAuthGateway` and `startAuthBroker` so embedded callers cannot bypass it.

### [P2] File mutation paths can bypass the workspace jail

**Locations:** [`packages/coding-agent/src/core/harvest/security.ts`](packages/coding-agent/src/core/harvest/security.ts#L132-L145), [`packages/coding-agent/src/tools/write.ts`](packages/coding-agent/src/tools/write.ts#L1288-L1301), [`packages/coding-agent/src/edit/index.ts`](packages/coding-agent/src/edit/index.ts#L577-L621), [`packages/coding-agent/src/lsp/writethrough.ts`](packages/coding-agent/src/lsp/writethrough.ts#L66-L76), [`packages/coding-agent/src/lsp/client.ts`](packages/coding-agent/src/lsp/client.ts#L547-L565), [`packages/coding-agent/src/lsp/edits.ts`](packages/coding-agent/src/lsp/edits.ts#L316-L346), [`crates/pi-edit/src/path_policy.rs`](crates/pi-edit/src/path_policy.rs#L46-L58), [`crates/pi-edit/src/path_policy.rs`](crates/pi-edit/src/path_policy.rs#L172-L196), [`crates/pi-edit/src/session.rs`](crates/pi-edit/src/session.rs#L234-L250)

There are multiple gaps in the mutation checks. First, `assertPathJailed` calls `realpath` on the complete target and, on any failure, compares the normalized lexical path instead. For a new file under a workspace symlink that points outside the workspace, `realpath` fails because the leaf does not exist; the lexical path appears in-workspace and is accepted. The write path then reaches `writeFileWithFallback`, which writes to that path and follows the parent symlink. Second, `EditTool.#write` handles `op === "delete"` before it invokes `assertPathJailed`, so delete requests skip the workspace check altogether. Third, the LSP `workspace/applyEdit` handler forwards server-supplied document and resource-operation URIs to `applyWorkspaceEdit`, which converts them to filesystem paths and writes, renames, or deletes them without workspace containment checks. An LSP server can therefore target an absolute file URI outside the session workspace. The native edit engine has a related containment gap for the plan-mode `local://` sandbox: `PathPolicy::resolve` only normalizes the URL path lexically, and `targets_local_sandbox` immediately accepts any lexical descendant. A symlink already inside the sandbox can point to an outside file or directory, and `EditSession::apply` passes the resulting path to the writer after this check, allowing a plan-mode edit to modify the symlink target outside the sandbox.

**Impact:** Agent tools and LSP servers can mutate files outside the session workspace through symlinked paths, unchecked delete operations, absolute LSP file URIs, or symlinked plan-mode `local://` paths.

**Recommendation:** Apply one shared workspace-containment check to every mutation entry point, including LSP text/resource operations and the native edit engine's `local://` sandbox. Validate every target before branching by operation. Resolve and validate the deepest existing ancestor for nonexistent write targets (or reject destinations whose real location cannot be established), and keep the check adjacent to the filesystem operation to limit symlink races. The repository already has path-resolution helpers that account for nonexistent leaves and symlinked ancestors; reuse an appropriate helper here.

### [P3] The decision sidecar trusts unauthenticated web pages on localhost's default port

**Location:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L96-L102)

In the current worktree, the prior wildcard CORS policy has been narrowed and credentials are disabled. The allowlist still trusts `http://localhost` and `http://127.0.0.1` (the default HTTP port) as cross-origins, while `/v1/decide` has no authentication. A user visiting a hostile application served from one of those origins can make browser requests to the loopback sidecar and read responses. Requests can submit state and questions for inference, and the endpoint writes request-derived information to the decision log. This no longer permits an arbitrary public website origin such as `https://example.com`.

**Impact:** A hostile or compromised web application available on localhost's default port can use the user's local model service, read its responses, and cause request state to be written to the local decision log.

**Recommendation:** Remove CORS if browser clients are not required. Otherwise allow only the exact UI origin that needs it and add a per-process secret or another local authorization check; do not treat loopback binding as authentication.

### [P2] Calibration silently mixes synthetic labels into real calibration data

**Location:** [`decision-sidecar/calibration.py`](decision-sidecar/calibration.py#L253-L259)

When fewer than 50 labeled records exist, the CLI always appends 450 synthetic records (150 per call site) to the real records before fitting and saving calibration parameters. If a real call site overlaps one of the synthetic call sites, the fitted temperature for that site combines observed and generated labels. The saved result does not distinguish this mixed dataset from real calibration.

**Impact:** Calibration values can be dominated by fabricated examples and then appear to be based on production outcomes. If these parameters are used for confidence thresholds or routing, decisions can be skewed while reporting apparently measured calibration.

**Recommendation:** Make synthetic calibration an explicit, separate mode that does not append to real records or persist production parameters. If synthetic data is retained as a bootstrap aid, label its provenance in output and keep its parameters separate from those fitted on observed outcomes.

### [P2] OAuth callback reflects provider error text into executable HTML

**Locations:** [`packages/ai/src/registry/oauth/callback-server.ts`](packages/ai/src/registry/oauth/callback-server.ts#L555-L565), [`packages/ai/src/registry/oauth/callback-server.ts`](packages/ai/src/registry/oauth/callback-server.ts#L592-L596), [`packages/ai/src/registry/oauth/oauth.html`](packages/ai/src/registry/oauth/oauth.html)

The callback reads `error_description` from the request URL and puts it into `resultState.error`. It then embeds `JSON.stringify(resultState)` directly into the HTML template's `<script type="application/json">` block. JSON stringification does not escape HTML script terminators, so a value containing `</script>` can end the data block and inject executable markup/script into the callback response. A hostile page can navigate a popup to a known loopback OAuth callback port with a crafted error description; the injected script then runs with that localhost origin. The reflected error is included even when the request's OAuth `state` is invalid (state only controls whether the pending login is rejected).

**Impact:** A crafted callback URL can execute script in the browser under the local OAuth callback origin, creating a localhost reflected-XSS path during a login flow.

**Recommendation:** Escape `<` (at minimum) when serializing JSON for an HTML script element, or move the response state into a safely encoded data attribute/text node and parse it without interpolating raw HTML. Keep callback state validation independent from safe response rendering.

### [P2] Session transcripts are created with default world-readable permissions on Unix

**Locations:** [`packages/coding-agent/src/session/session-paths.ts`](packages/coding-agent/src/session/session-paths.ts#L185-L196), [`packages/coding-agent/src/session/session-storage.ts`](packages/coding-agent/src/session/session-storage.ts#L109-L119), [`packages/coding-agent/src/session/session-storage.ts`](packages/coding-agent/src/session/session-storage.ts#L203-L220), [`packages/coding-agent/src/session/session-storage.ts`](packages/coding-agent/src/session/session-storage.ts#L294-L304)

The default session directory is created by `FileSessionStorage.ensureDirSync` with `mkdirSync(..., { recursive: true })`, and session files are created through `openSync` without a mode, `writeFileSync` without a mode, or `Bun.write`. On Unix, these use the process umask against permissive defaults; with the common `022` umask, newly created session directories are typically `0755` and transcript files `0644`. The code does not subsequently chmod session directories or files. Transcripts include user messages, tool output, and persisted conversation context, so other local accounts can read them on shared systems. In contrast, the nearby credential database explicitly applies `0700`/`0600` permissions.

**Impact:** On multi-user Unix hosts with a typical umask, local users other than the session owner can read conversation transcripts and tool output from the default session store.

**Recommendation:** Create and enforce private permissions for the session root and per-project directories, and create transcript/temp files with owner-only permissions. Apply the hardening to existing session directories/files as well as newly created ones.

### [P2] Laya pruning can remove tool calls while retaining their tool results

**Locations:** [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L269-L285), [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L425-L430)

Older assistant messages become pruning candidates when their text is large enough, without checking whether `content` also contains `toolCall` blocks. If Laya scores such a candidate for removal, the pruning branch replaces the entire assistant `content` array with a text placeholder. A mixed assistant response therefore loses its tool-call blocks, while the corresponding tool-result messages remain in context.

**Impact:** A later provider request can contain tool results with no matching assistant tool call, and the model loses a prior tool action that may be necessary to understand the conversation. Some provider APIs reject malformed tool histories.

**Recommendation:** Preserve non-text protocol blocks when pruning assistant text, or exclude assistant messages containing tool calls from pruning.

### [P1] Laya pruning SDK integration passes an unsupported cancellation option

**Locations:** [`packages/coding-agent/src/sdk.ts`](packages/coding-agent/src/sdk.ts#L3419-L3427), [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L15-L22), [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L55-L78), [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L297-L314)

The new module imports `AgentMessage` from `@harvest/pi-ai`, but that package exports `Message` and does not define `AgentMessage` (the latter is declared in `@harvest/pi-agent-core`). The SDK also calls `pruneContextWithLaya` with a `signal` property, but `LayaPruningOptions` does not declare it. These type errors prevent the coding-agent package from type-checking. The pruning helper does not pass cancellation to `LayaClient.decide`, and `LayaClient.decide` only creates its own timeout controller; an aborted turn therefore cannot interrupt the sidecar request.

**Impact:** The current SDK integration fails static type checking; cancellation handling would remain incomplete if the call were made to compile by dropping the option.

**Recommendation:** Add the abort signal to the pruning option contract and propagate it through `LayaClient.decide` to its fetch, returning the original unpruned context when the operation is cancelled.

**Reproduction / Code Proof:** After installing the locked workspace dependencies, `bun --cwd=packages/coding-agent run check:types` exits 1 with 31 TypeScript diagnostics. The production errors include the missing `AgentMessage` export, the undeclared `signal` option, an invalid `laya.pruningMinChunkTokens` setting path, and an implicit `any`; Laya tests additionally use stale `DecisionResult`, `AgentSource`, `AgentDefinition`, and `LayaClient` shapes. Bun's runtime transpiler still executes most of those tests, so the passing runtime assertions do not establish that the package builds.

### [P3] Apple-Silicon hardware tests do not mock the macOS version gate

**Locations:** [`decision-sidecar/test_hardware.py`](decision-sidecar/test_hardware.py#L39-L70), [`decision-sidecar/hardware.py`](decision-sidecar/hardware.py#L120-L147)

**Description:** Both Apple-Silicon tests patch `sys.platform`, `platform.machine()`, and `platform.processor()`, but leave `platform.mac_ver()` untouched. On a non-macOS runner it returns an empty version, which production code parses as major version zero and correctly takes the pre-macOS-14 CPU fallback before reaching either mocked MPS branch. The MPS-success test therefore observes `cpu`, and the allocation-failure test observes the version-gate reason instead of the expected MPS allocation reason.

**Impact:** `python3 -m unittest discover -s decision-sidecar -p "test_*.py"` fails on Linux and Windows even when the hardware detection behavior is correct. The intended MPS success and allocation-failure contracts are not tested portably.

**Reproduction / Code Proof:** On this Linux host the suite reports two failures: expected `apple_silicon_mps` but received `cpu`, and expected an MPS fallback reason but received `Apple Silicon CPU fallback: macOS  is below required macOS 14+`.

**Recommendation & Fix:** Patch `platform.mac_ver` to return a supported macOS version such as `("14.5", ("", "", ""), "")` in both tests. Keep the existing `torch.zeros.side_effect` assertion path so the second test reaches and verifies the MPS allocation fallback.

### [P2] The advertised bucketing test is neither discovered nor coupled to production code

**Locations:** [`decision-sidecar/test_bucketing.py`](decision-sidecar/test_bucketing.py#L1-L74), [`decision-sidecar/test_bucketing.py`](decision-sidecar/test_bucketing.py#L76-L169), [`decision-sidecar/server.py`](decision-sidecar/server.py#L67-L103)

**Description:** `test_bucketing.py` is named like a unittest module but defines only free functions plus a `main()` benchmark. `unittest discover` imports the module but does not execute `test_unit_bucketing()` or the numerical-equivalence benchmark. The file also copies `bucket_items_by_length()` instead of importing it from `server.py`, so even a direct execution can pass while the production implementation has diverged. Import-time dependencies on NumPy, PyTorch, and Laya cause discovery to error before any unrelated unit contract can run when the ML environment is absent.

**Impact:** The documented Python suite gives no automated coverage for empty/single/extreme buckets or bucketed-versus-unbucketed numerical equivalence. A regression in the live sidecar bucketing implementation can ship while this file continues to appear to be a passing test.

**Reproduction / Code Proof:** `python3 -m unittest discover -s decision-sidecar -p "test_*.py"` currently errors while importing this module because NumPy is absent. With the dependencies installed, unittest still finds no `unittest.TestCase` methods in this file; the assertions run only through `python test_bucketing.py`, which also downloads/loads the 421M checkpoint.

**Recommendation & Fix:** Move the lightweight bucketing helper into an import-safe module and test that exact production function with a real `unittest.TestCase`. Put model-backed numerical equivalence and latency measurement in a separately marked integration/benchmark suite with an explicit dependency and checkpoint opt-in.

### [P2] The Laya setup unit suite installs packages, downloads a model, and starts a daemon

**Locations:** [`packages/coding-agent/test/laya-setup-wizard.test.ts`](packages/coding-agent/test/laya-setup-wizard.test.ts#L133-L161), [`packages/coding-agent/src/core/harvest/laya-service.ts`](packages/coding-agent/src/core/harvest/laya-service.ts#L90-L180), [`packages/coding-agent/src/core/harvest/laya-service.ts`](packages/coding-agent/src/core/harvest/laya-service.ts#L186-L228)

**Description:** The setup-wizard test calls `configureLayaLocally()` without a seam or mock. On a clean host that function may run `pip install`, download/load the large Hugging Face checkpoint, spawn a detached uvicorn process, and poll a fixed port. The adjacent probe test also assumes a locally installed Python runtime. These are host-mutating integration actions embedded in the default `laya-*.test.ts` unit glob.

**Impact:** The test is network-, platform-, package-index-, disk-, memory-, and port-dependent; it can mutate a developer or CI machine and leave a daemon behind. On this clean host it was the sole runtime failure (63 passed, 1 failed), after the dependency-install path returned failure.

**Reproduction / Code Proof:** `bun test packages/coding-agent/test/laya-*.test.ts` reaches the assertion at line 154 with `result.success === false`. The call stack is the real setup implementation, not a fake service boundary.

**Recommendation & Fix:** Inject a typed process/service adapter and test the setup state transitions with per-test spies restored in `afterEach`. Move one real install/download/daemon scenario to an explicitly opted-in integration test that uses an isolated virtual environment, cache, temporary port, and guaranteed process cleanup.

### [P2] The Laya pruning token-budget setting is never read

**Locations:** [`packages/coding-agent/src/config/settings-schema.ts`](packages/coding-agent/src/config/settings-schema.ts#L607-L616), [`packages/coding-agent/src/sdk.ts`](packages/coding-agent/src/sdk.ts#L3422-L3426), [`packages/coding-agent/src/core/harvest/laya-pruning.ts`](packages/coding-agent/src/core/harvest/laya-pruning.ts#L356-L363)

The settings schema exposes `laya.pruningTokenBudget` with a 32,000-token default, but the SDK does not pass it to `pruneContextWithLaya`, and the pruning helper never reads the setting. Unless a caller manually supplies `prunableTokenBudget`, the helper instead uses 40% of candidate tokens (with a 1,500-token floor), so changing the documented setting has no effect.

**Impact:** Users cannot control the pruning retention budget through the exposed setting; actual context retention can differ substantially from the configured value.

**Recommendation:** Read `laya.pruningTokenBudget` from settings or pass it through the SDK call, and keep the configured unit/meaning aligned with the budget selection logic.

### [P2] Laya subagent selection can choose agents that execution policy rejects

**Locations:** [`packages/coding-agent/src/task/structured-subagent.ts`](packages/coding-agent/src/task/structured-subagent.ts#L265-L289), [`packages/coding-agent/src/task/structured-subagent.ts`](packages/coding-agent/src/task/structured-subagent.ts#L233-L256), [`packages/coding-agent/src/core/harvest/laya-subagent-selection.ts`](packages/coding-agent/src/core/harvest/laya-subagent-selection.ts#L150-L216), [`packages/coding-agent/src/task/discovery.ts`](packages/coding-agent/src/task/discovery.ts#L146-L149)

The selector receives every discovered agent, including agents excluded by the session's `spawns` allowlist or `task.disabledAgents`. When Laya returns a high-confidence choice, `resolveEffectiveSubagentPolicy()` switches to it and re-runs the spawn assertion; a disallowed choice throws instead of falling back to the permitted default. Disabled agents are checked later and likewise turn the automatic choice into a request failure. In addition, `buildSubagentCriteria()` lowercases agent names and the selected result is lowercased, while `getAgent()` performs an exact case-sensitive name lookup; a custom agent whose name contains uppercase characters can be selected as an unresolvable lowercase name.

**Impact:** Default subagent requests can fail depending on Laya's pick when the roster contains restricted or disabled agents, and custom agents with mixed-case names can become unresolvable through automatic selection.

**Recommendation:** Build the selection roster from agents that pass the same spawn/depth/disabled checks used at execution time, and map normalized model choices back to the original exact agent name. If a result becomes invalid, retain the already-validated default agent rather than failing the request.

### [P3] Stats server writes directly to the terminal while the TUI is active

**Locations:** [`packages/stats/src/server.ts`](packages/stats/src/server.ts#L169), [`packages/stats/src/server.ts`](packages/stats/src/server.ts#L392), [`packages/coding-agent/src/modes/controllers/command-controller.ts`](packages/coding-agent/src/modes/controllers/command-controller.ts#L166)

`startServer()` is called by the interactive `/trace` command. If it needs to rebuild the stats client, `ensureClientBuild()` writes a progress line with `console.log`; if an HTTP handler throws, the server's catch block writes the error with `console.error`. Both paths can run while the interactive terminal UI owns stdout/stderr, so they can corrupt the rendered screen. The package is also used by a slash-command path in `builtin-collaboration.ts`.

**Impact:** Opening the trace dashboard can print build progress into the TUI, and a request-time server error can write arbitrary error text over the interactive display.

**Recommendation:** Route these messages through the shared logger or a caller-provided output sink; reserve direct console output for standalone CLI commands.

### [P2] Starting the TUI debug socket deletes any existing path at its configured location

**Locations:** [`packages/tui/src/tui.ts`](packages/tui/src/tui.ts#L1096-L1099), [`packages/tui/src/debug-server.ts`](packages/tui/src/debug-server.ts#L221-L235)

When `OMP_TUI_DEBUG` is set, `TuiDebugServer.start()` unconditionally calls `unlinkSync()` on the configured path whenever anything already exists there. It does not check that the existing filesystem entry is a stale Unix socket. Setting the debug path to an existing regular file therefore deletes that file before the socket bind is attempted; a symlink or another process's socket is also unlinked without validating its type or ownership. A directory is not deleted, but the attempted unlink fails and the listener later cannot bind at that path.

**Impact:** Enabling TUI debugging can destroy unrelated user data at the configured path, and can disrupt another process that owns a socket there.

**Recommendation:** Refuse to replace non-socket entries. For an existing socket, verify that it is stale and owned by the current user before removing it, or choose a fresh path instead of unlinking blindly.

### [P2] TUI debug socket buffers unterminated requests without a size limit

**Location:** [`packages/tui/src/debug-server.ts`](packages/tui/src/debug-server.ts#L258-L268)

Each accepted client appends incoming data to a string buffer and processes requests only after a newline arrives. There is no per-line or per-client byte limit, and the socket remains open while the buffer grows. A client with access to the opt-in `OMP_TUI_DEBUG` socket can send an indefinitely long line (or many large requests) and exhaust the TUI process's memory; large complete lines also go through synchronous `JSON.parse` on the event loop.

**Impact:** A local debug-socket client can stall or terminate the interactive agent through unbounded memory and parsing work.

**Recommendation:** Enforce a maximum request-line size before appending or parsing, close clients that exceed it, and bound the number of queued requests processed per event-loop turn.

### [P1] Metaharness exposes unauthenticated run control on every network interface

**Locations:** [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L211-L221), [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L282-L340), [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L380-L420), [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L552-L585)

`ManagerServer.start()` calls `Bun.serve({ port, ... })` without a hostname; Bun's default is `0.0.0.0`. The API routes have no authentication or origin/CSRF guard, yet include launching and resuming benchmark processes, cancelling runs, and deleting run or experiment records and job directories. The README presents the service as a local dashboard, but its default bind makes those controls reachable to other machines on the host's network.

**Impact:** A network client can start benchmark workloads as the server user, consume configured model/API resources, interrupt active runs, and delete stored benchmark data.

**Recommendation:** Bind to `127.0.0.1` by default and require an explicit opt-in for a network bind; if remote access is supported, protect every mutating route with authentication and CSRF defenses.

### [P2] Mnemopi creates its persistent memory database with ambient file permissions

**Locations:** [`packages/mnemopi/src/db.ts`](packages/mnemopi/src/db.ts#L80-L84), [`packages/mnemopi/src/config.ts`](packages/mnemopi/src/config.ts#L19-L20), [`packages/mnemopi/src/core/beam/store.ts`](packages/mnemopi/src/core/beam/store.ts#L150-L165)

`openDatabase()` creates the parent directory with the platform's default `mkdir` mode and lets SQLite create the database without an explicit restrictive mode or a post-create permission check. Under a common `022` umask, a new data directory is typically `0755` and the database `0644`. The default location is `~/.hermes/mnemopi/data`, and the database stores raw working-memory content, so other local accounts can read persisted private memories on systems whose home path permits traversal.

**Impact:** Private memory text and associated metadata can be disclosed to other local users through the default on-disk database.

**Recommendation:** Create the data directory with owner-only permissions and ensure existing directories/database files are not more permissive than intended; use platform-appropriate ACL handling where Unix mode bits do not apply.

### [P2] Ollama Cloud accepts invalid context lengths from model metadata

**Locations:** [`packages/catalog/src/provider-models/ollama.ts`](packages/catalog/src/provider-models/ollama.ts#L82-L92), [`packages/catalog/src/provider-models/ollama.ts`](packages/catalog/src/provider-models/ollama.ts#L178-L206)

`getContextWindow()` accepts any JavaScript number from `/api/show`, including zero, negative values, `NaN`, and infinity. That value is then used as the discovered model's `contextWindow`; for most models it also feeds the `maxTokens` calculation. A malformed or unexpected upstream metadata value can therefore replace the safe fallback with an unusable limit, including `NaN` limits.

**Impact:** Ollama Cloud models can be advertised with invalid context/output limits, causing downstream context budgeting or request construction to fail or behave unpredictably.

**Recommendation:** Accept only finite positive integer context lengths and otherwise use the existing reference/fallback limit.

### [P2] Native audio playback buffers an unbounded amount of queued audio

**Locations:** [`crates/pi-voice/src/audio.rs`](crates/pi-voice/src/audio.rs#L116-L129), [`crates/pi-voice/src/audio.rs`](crates/pi-voice/src/audio.rs#L139-L145), [`crates/pi-natives/src/audio.rs`](crates/pi-natives/src/audio.rs#L56-L98)

`PlaybackStream::start()` creates an unbounded channel, and every `PlaybackWriter::write()` copies the entire caller-provided sample slice into a new `Vec` and enqueues it without a size or backlog limit. The public N-API `AudioPlayback.write(Float32Array)` exposes this path directly; the live WebRTC decoder also feeds it. If producers write faster than the output device consumes samples, or continue while the device is stalled, queued vectors grow without bound.

**Impact:** A caller can exhaust process memory by queueing large or sustained audio faster than the speaker drains it, potentially terminating the CLI or live session.

**Recommendation:** Bound queued audio by samples or duration and define an explicit backpressure/overflow policy. Validate per-call sample size as well, so a single write cannot exceed the intended queue budget.

### [P2] Native audio capture can queue microphone chunks without a bound

**Locations:** [`crates/pi-natives/src/audio.rs`](crates/pi-natives/src/audio.rs#L29-L40), [`crates/pi-voice/src/audio.rs`](crates/pi-voice/src/audio.rs#L295-L309)

The microphone callback copies every device chunk into a new `Vec` and submits it to the N-API `ThreadsafeFunction` using `NonBlocking` mode. The adapter sets no queue capacity and has no drop/backpressure policy. If the JavaScript event loop is blocked or the callback runs slower than capture, pending chunks accumulate in the N-API callback queue.

**Impact:** A stalled JavaScript consumer can cause microphone capture to grow process memory until the host is disrupted or terminated.

**Recommendation:** Give the callback queue a finite capacity and define overload behavior, such as dropping oldest/newest chunks with an observable counter or stopping capture; keep capture callbacks from allocating an unbounded backlog.

### [P2] Native VCS patch writes follow symlinked directories outside the repository

**Locations:** [`crates/pi-vcs/src/git/patch.rs`](crates/pi-vcs/src/git/patch.rs#L101-L122), [`crates/pi-vcs/src/git/patch.rs`](crates/pi-vcs/src/git/patch.rs#L1500-L1534), [`crates/pi-vcs/src/git/patch.rs`](crates/pi-vcs/src/git/patch.rs#L1558-L1567), [`crates/pi-natives/src/vcs.rs`](crates/pi-natives/src/vcs.rs#L1257-L1265)

`apply_patch()` validates patch paths against absolute paths and `..`, but `write_worktree_entry()` only repeats that lexical validation before joining the path to the repository root. It then creates parent directories and calls `fs::write`, both of which follow an existing symlink in a parent component. A patch that creates or updates `linked-dir/outside.txt` can therefore write through a workspace symlink `linked-dir` to a location outside the repository. The native VCS adapter exposes this operation to the TypeScript caller.

**Impact:** A patch supplied to the agent's VCS operation can modify files outside the repository when the worktree contains a symlinked parent directory.

**Recommendation:** Resolve and validate the target's existing ancestors against the repository root, and make the containment check robust against symlink replacement between validation and write (for example, use directory-relative no-follow operations).

### [P2] Search-page fetches buffer unbounded response bodies

**Locations:** [`packages/coding-agent/src/web/search/providers/browser-page.ts`](packages/coding-agent/src/web/search/providers/browser-page.ts#L44-L54), [`packages/coding-agent/src/web/search/providers/browser-page.ts`](packages/coding-agent/src/web/search/providers/browser-page.ts#L104-L120)

`fetchHtmlPage()` reads every response with `response.text()`, and the headless-browser fallback returns `page.content()` without a byte or character cap. These paths serve the HTML-backed Google, DuckDuckGo, Ecosia, Mojeek, and Startpage search providers. Their hard timeout limits how long a fetch can run, but it does not limit the amount of response data buffered during that time.

**Impact:** An unexpectedly large or hostile search response can consume substantial memory in the coding agent while a web-search request is processed.

**Recommendation:** Enforce a maximum response size in both the fetch and browser paths, stopping body reads once the limit is reached and returning a bounded/truncated result or a clear provider error.

### [P2] Autoresearch loads the entire benchmark log into memory after execution

**Locations:** [`packages/coding-agent/src/autoresearch/tools/run-experiment.ts`](packages/coding-agent/src/autoresearch/tools/run-experiment.ts#L300-L325), [`packages/coding-agent/src/autoresearch/tools/run-experiment.ts`](packages/coding-agent/src/autoresearch/tools/run-experiment.ts#L146-L151)

The experiment runner streams all benchmark output into `benchmark.log` without a file-size cap, then reads the complete file into a string with `fs.promises.readFile()`. The 4 KiB experiment limit is applied only afterward to the LLM-facing preview, so it does not bound the persisted log or peak memory. The command runs for up to ten minutes by default.

**Impact:** A benchmark that emits output continuously can grow the run log until it consumes available disk, then force the coding-agent to allocate the same large output again in memory after the process exits.

**Recommendation:** Enforce a maximum captured-log size while streaming and parse metrics incrementally or from a bounded tail; preserve an explicit indication that the full output was cut off.

### [P2] Cancelled background jobs stop counting toward the concurrency limit before they settle

**Locations:** [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L275-L283), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L390-L398), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L555-L564)

`cancel()` and `cancelAll()` change a job's status to `cancelled` immediately after sending its abort signal. Both capacity checks count only jobs whose status is `running`, so the slot is released before the job's `run()` promise has settled. The manager's callback contract accepts arbitrary async work and does not enforce abort cooperation; a callback that ignores the signal or is blocked in non-cancellable work can continue running after cancellation while new jobs are admitted.

**Impact:** Repeatedly cancelling unresponsive jobs can exceed the configured limit of concurrently executing background work, allowing runaway child processes or other work to accumulate.

**Recommendation:** Track execution settlement independently from user-visible job status, and keep unsettled cancelled jobs in the capacity count until their promise completes (with a separate explicit policy if shutdown intentionally abandons them).

### [P2] Async job result deliveries can accumulate without a concurrency bound

**Locations:** [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L1011-L1033), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L1048-L1088)

The delivery loop removes each ready item and starts `#deliverDelivery()` without awaiting it, so it proceeds to launch every queued delivery even while earlier sink calls remain pending. `#deliverDelivery()` awaits the sink with no deadline or cancellation, and the manager explicitly allows for a sink that never settles. The running-job limit does not cap deliveries after their jobs finish, and completed jobs can continue to be created while old sink calls hang.

**Impact:** A stalled delivery sink can leave an unbounded number of promises, delivery records, and result payloads retained in `#inFlightDeliveries` over time.

**Recommendation:** Bound concurrent sink calls and apply a delivery deadline or cancellation policy that releases retained delivery state when an owner cannot accept a result.

### [P2] Local memory roots collide for distinct project paths

**Locations:** [`packages/coding-agent/src/memories/index.ts`](packages/coding-agent/src/memories/index.ts#L1280-L1281), [`packages/coding-agent/src/memories/index.ts`](packages/coding-agent/src/memories/index.ts#L1422-L1424), [`packages/coding-agent/src/memories/index.ts`](packages/coding-agent/src/memories/index.ts#L299-L306)

`encodeProjectPath()` replaces every path separator and colon with `-` without escaping existing hyphens or adding an unambiguous encoding. For example, `/work/a-b/c` and `/work/a/b-c` both map to `--work-a-b-c--`. Every local project memory artifact uses this name, including `learned.md`, rollout summaries, and the project memory database; `clearMemoryData()` recursively deletes the same colliding root.

**Impact:** Projects with colliding paths can share learned context and summaries, and clearing memory for one project can delete the other project's local memory.

**Recommendation:** Use a collision-resistant encoding of the canonical project path (for example, an escaped path plus a stable hash) and keep a migration or compatibility strategy for existing memory directories.

### [P2] Cleanse checker output is buffered without a size limit

**Locations:** [`packages/coding-agent/src/cleanse/checkers.ts`](packages/coding-agent/src/cleanse/checkers.ts#L1213-L1266), [`packages/coding-agent/src/cleanse/checkers.ts`](packages/coding-agent/src/cleanse/checkers.ts#L1290-L1300)

`runChecker()` retains all stdout and stderr in strings until the child exits. For streaming diagnostics, its poller repeatedly passes the complete accumulated strings through `completeLines()`, which slices and copies the text again every flush interval. Multiple read-only checkers may run concurrently, and the failure-output cap is applied only when formatting a final failure diagnostic.

**Impact:** A noisy or stuck checker can grow memory without bound; periodic full-buffer copies can also amplify CPU and allocation costs on large output.

**Recommendation:** Cap captured output or parse and discard complete prefixes incrementally, retaining only the bounded context needed for partial diagnostics and failure messages.

### [P3] Activity queries with a zero limit return every matching row

**Locations:** [`packages/coding-agent/src/activity/index.ts`](packages/coding-agent/src/activity/index.ts#L277-L284), [`packages/coding-agent/src/activity/index.ts`](packages/coding-agent/src/activity/index.ts#L311-L314)

Both `recent()` and `query()` clamp a requested limit to zero and then call `slice(-limit)`. In JavaScript, `-0` is `0`, so `slice(-0)` is `slice(0)` and returns the entire array. A caller requesting no rows therefore gets all matching activity rows instead.

**Impact:** The activity index violates its limit contract for `limit: 0`, potentially returning significantly more records than the caller requested across all indexed agents.

**Recommendation:** Return an empty array when the clamped limit is zero before applying the negative slice.

### [P2] An unterminated CSI escape grows the virtual terminal parser buffer without limit

**Locations:** [`packages/utils/src/vterm/terminal.ts`](packages/utils/src/vterm/terminal.ts#L175-L184), [`packages/coding-agent/src/modes/components/bash-execution.ts`](packages/coding-agent/src/modes/components/bash-execution.ts#L168-L205)

When the parser is in `csi` state, every character that is not a final byte in the `@`–`~` range is appended to `#sequence`. There is no maximum sequence length or reset condition other than a final byte or a new escape. A child process can emit `ESC [` followed by an arbitrary stream of parameter/intermediate bytes without terminating the sequence. The interactive-bash PTY replay feeds child output directly into this terminal.

**Impact:** A process whose output is displayed through PTY replay can grow retained memory for the lifetime of the terminal, even though the surrounding PTY queue and rendered scrollback are bounded.

**Recommendation:** Cap CSI sequence length and discard/reset malformed sequences after the cap while preserving parser synchronization.

### [P2] Eval runner staging trusts a pre-existing file under a predictable shared-temp path

**Locations:** [`packages/coding-agent/src/eval/runner-cache.ts`](packages/coding-agent/src/eval/runner-cache.ts#L29-L38), [`packages/coding-agent/src/eval/py/kernel.ts`](packages/coding-agent/src/eval/py/kernel.ts#L282-L285)

`stageRunnerScript()` creates a fixed directory directly under `os.tmpdir()`, then treats any existing `runner-<hash>.py` path as trusted. `existsSync()` follows symlinks, and the code neither verifies ownership/content nor creates the cache directory with a private, verified permission boundary. The Python kernel then executes that returned path. A local user who pre-creates the predictable directory and runner filename can plant a symlink to a script they control before the first eval invocation.

**Impact:** On shared-temp systems, starting the Python eval kernel can execute another local user's planted script with the Harvest user's privileges.

**Recommendation:** Stage under an owner-only cache directory and validate its ownership and permissions; write to a fresh file using exclusive/no-follow semantics and verify the staged contents before execution.

### [P2] Morphing schemas treat an own `__proto__` property as a prototype mutation

**Locations:** [`packages/omptype/src/compile.ts`](packages/omptype/src/compile.ts#L850-L885), [`packages/omptype/src/interp.ts`](packages/omptype/src/interp.ts#L394-L441)

When an object schema morphs and removes extra properties, both validator paths create an ordinary `{}` and assign declared output properties with `out[key] = value`. For a schema that declares `__proto__` and an input with that own property, the assignment invokes the legacy prototype setter instead of defining an own property. The returned object therefore loses the declared field and inherits from the input value; this can also affect subsequent property lookups through the changed prototype.

**Impact:** Validated/morphed data does not preserve the schema’s declared shape, and downstream consumers may observe attacker-controlled inherited properties.

**Recommendation:** Build fresh outputs with a null prototype or define properties with `Object.defineProperty`/an equivalent own-property operation, consistently in both interpreter and JIT paths.

### [P3] Snapcompact shape validation accepts inherited object property names

**Locations:** [`packages/snapcompact/src/snapcompact.ts`](packages/snapcompact/src/snapcompact.ts#L201-L202), [`packages/snapcompact/src/snapcompact.ts`](packages/snapcompact/src/snapcompact.ts#L400-L410), [`packages/coding-agent/src/modes/components/snapcompact-shape-preview.ts`](packages/coding-agent/src/modes/components/snapcompact-shape-preview.ts#L69-L77)

`isShapeVariantName()` checks `value in SHAPE_VARIANTS`, but `SHAPE_VARIANTS` is an ordinary object. Names inherited from `Object.prototype`, such as `toString` and `constructor`, therefore pass the type guard. `resolveShape()` then indexes the table with that value and passes the inherited function to geometry/billing code instead of a shape record.

**Impact:** Invalid persisted or UI-provided shape names can be accepted as valid and break snapcompact preview/rendering.

**Recommendation:** Use `Object.hasOwn(SHAPE_VARIANTS, value)` for the runtime guard.

### [P2] Devin Connect gzip frames have no decompressed-size limit

**Locations:** [`packages/ai/src/providers/devin.ts`](packages/ai/src/providers/devin.ts#L84-L85), [`packages/ai/src/providers/devin.ts`](packages/ai/src/providers/devin.ts#L246-L258), [`packages/ai/src/providers/devin.ts`](packages/ai/src/providers/devin.ts#L322-L326)

The Connect parser rejects frames larger than 16 MiB, but that check applies to the compressed wire payload. Both normal message frames and end-stream trailers pass compressed payloads directly to `gunzipSync()` without a decompressed output cap. A highly compressible response can therefore expand far beyond the frame limit before protobuf parsing or trailer handling.

**Impact:** A malformed or compromised Devin endpoint can exhaust the client process’s memory while it handles a single bounded-size stream frame.

**Recommendation:** Decompress with a strict maximum output length and reject frames that exceed the uncompressed message budget.

### [P2] Bedrock event-stream buffering trusts an unbounded peer frame length

**Locations:** [`packages/ai/src/providers/aws-eventstream.ts`](packages/ai/src/providers/aws-eventstream.ts#L157-L166)

`decodeEventStream()` appends each response chunk to `buf`, reads the peer-provided 32-bit frame length, and waits until that many bytes arrive. It rejects lengths below the minimum but has no upper bound on a declared frame. A response that advertises a very large frame and streams data without completing it keeps growing the concatenated buffer.

**Impact:** A malformed or compromised Bedrock endpoint/proxy can cause unbounded memory growth during streaming inference.

**Recommendation:** Reject total lengths above an explicit event-stream frame cap before buffering toward the declared length; also bound residual bytes when the stream ends or is malformed.

### [P1] Non-Git isolation diffs follow changed symlinks and read outside files

**Locations:** [`crates/pi-iso/src/diff.rs`](crates/pi-iso/src/diff.rs#L326-L340), [`crates/pi-iso/src/diff.rs`](crates/pi-iso/src/diff.rs#L348-L380), [`crates/pi-natives/src/iso.rs`](crates/pi-natives/src/iso.rs#L159-L174)

The non-Git tree walker records symlink entries, but `plain_change()` later calls `std::fs::read(side.join(rel))`, which follows the symlink. For an added or modified symlink pointing outside the workspace, the diff code reads the target’s bytes and, for UTF-8 text, includes them in the unified diff returned by the public `iso_diff` N-API function. The Git diff path does not do this because Git represents symlinks as link-target text.

**Impact:** A task able to create a symlink in a non-Git isolated workspace can cause the host to read an out-of-workspace file and disclose its contents through the captured diff.

**Recommendation:** Detect symlink entries with `symlink_metadata()` and represent link-target changes without following them; use no-follow file opens for regular-file diff reads and reject path-type changes safely.

### [P2] Native workspace entry cap is applied after the full tree is collected

**Locations:** [`crates/pi-natives/src/workspace.rs`](crates/pi-natives/src/workspace.rs#L27-L28), [`crates/pi-natives/src/workspace.rs`](crates/pi-natives/src/workspace.rs#L189-L228)

`list_workspace()` calls `collect_with_heartbeat()` to materialize every matching entry into the walker's vector, then accumulates, sorts, and deduplicates the result before truncating to `MAX_ENTRIES`. The configured cap therefore bounds only the returned payload, not traversal memory, sorting work, or the intermediate collections. `max_depth` is supplied through the N-API request and can cover very large trees.

**Impact:** Scanning a sufficiently large workspace can exhaust memory or stall startup before the API returns its nominally capped result.

**Recommendation:** Enforce an entry/work budget while visiting entries (including the extra AGENTS.md paths), stop the traversal when the budget is reached, and report truncation from that streaming limit.

## Scope and verification

This is an ongoing static review of project-owned implementation across the monorepo, excluding vendored dependencies, generated output, and test fixtures. The review has inspected high-risk service boundaries, authentication, secret handling, agent-loop interruption and tool-history behavior, compaction pruning, HTML session export rendering, stats dashboard defaults, MCP stdio, blob callback authorization, workspace mutation paths, worker-host dispatch, current Laya pruning and subagent-selection integration, decision-sidecar request limits and calibration wiring, roboomp task/event lifecycle and workspace boundaries, OpenAI-compatible request setup and SSE transport, OpenAI Chat Completions and Responses gateway request schemas/parsing and data-URI image conversion, pi-native gateway request parsing, option allowlisting, SSE event forwarding, and client cancellation, Anthropic, Vertex AI, and Bedrock request-header/auth setup, plus selected Cursor gRPC, Devin Connect, Ollama, Kimi, Gemini CLI, GitLab Duo, and Google ADC token flows, selected shared message transforms (credential redaction, vision filtering, tool-call ID normalization and call/result pairing), archive path normalization, extraction limits, link handling, in-repository extraction call sites, the JSON Schema importer, omptype Zod and TypeBox adapters, auth-broker/auth-gateway request routing, worktree and GC CLI flows, secret-obfuscation paths, plugin manifest resolution, skill discovery boundaries, custom-command loading, custom-tool loading, command execution and cancellation, direnv trust checks, non-interactive child-process environment construction, shared retry/TLS-fetch/stream/abort/temp/lock utilities, coding-agent public search fan-out and HTML fetch/browser fallback size handling, autoresearch run-output capture and storage transitions, async job admission/cancellation/retention/delivery handling, advisor transcript collection, incremental delta coalescing, retry and context recovery, quarantine and emission guards, autolearn capture thresholds, managed-skill mutation and discovery boundaries, local memory lesson persistence and project-root scoping, cleanse checker discovery, execution, parser path filtering, streamed dispatch, and verification, activity transcript indexing, live/persisted row merging, subscriptions, and query/limit semantics, agent runtime tool-batch execution and abort pairing, append-only context synchronization and digest invalidation, compaction conversion and message-cache invalidation, and selected agent-loop pause/replay/proxy paths, TUI stdin framing and paste bounds, key parsing, SGR mouse routing, terminal protocol entry points, and TUI debug-socket path handling; mnemopi persistence/MCP operations, metaharness run-control routes, browser-relay extension/server boundaries, shared collab wire shapes, and TypeScript edit-benchmark generation/verification were also inspected. Catalog review now includes the provider descriptor types, model-cache namespace construction, compatibility cascade/identity resolution and context-window policy, Google and Ollama manager wiring, plus targeted OpenAI-compatible discovery-mapper paths. Rust/native review now also covers pi-edit path-policy and writer boundaries, pi-vcs patch application/worktree mutation, pi-walker symlink-cycle handling and scan-cache invalidation, and pi-shell process-tree management, cancellation, and persistent-shell lifecycle; pi-voice sample-rate/playback buffering and portions of the PulseAudio, WASAPI, and WebRTC paths; the initial native pass covered clipboard DIB decoding, macOS appearance FFI lifetime/cleanup, and fuzzy file-search ranking. The broad source inventory is too large for a claim that every implementation file has been manually inspected; most provider implementations, remaining Rust modules, and other workspace packages remain to be reviewed.

Repository checks performed (latest validation pass on 2026-09-24):

- Installed the locked Bun workspace dependencies with `bun install --frozen-lockfile` so the current TypeScript and Bun tests could execute.
- `bun --cwd=packages/coding-agent run check:types` failed with 31 diagnostics. Four production diagnostics are in `laya-pruning.ts`, one is the SDK's unsupported `signal`, and the remaining diagnostics include stale Laya test fixtures plus two unrelated `agent-session-auto-compaction-queue` test errors.
- `bun test packages/coding-agent/test/laya-*.test.ts` ran 64 tests: 63 passed and the real-environment setup test failed. Live-sidecar assertions were skipped because no daemon was running.
- `python3 -m unittest discover -s decision-sidecar -p "test_*.py"` ran five hardware tests (three passed, two failed) and errored while importing `test_bucketing.py` because NumPy is not installed. The two assertion failures are the incomplete macOS fixture described above.
- `bun run check:ts` stopped in its formatting gate with 66 files reported by `oxfmt --check`, so it did not reach all workspace package checks. `bunx oxlint .` itself exited successfully with one unused-import warning in `scratch/benchmark_logging_event_loop.ts`.
- `bun run lint:py` could not start because `ruff` is unavailable. `bun run test:rs` could not start because `cargo` is unavailable; it also warned that this extracted source tree has no `.git` metadata.

Earlier static-review checks recorded before this validation pass:

- `oxlint .` completed without reported lint errors; the combined `bun run check:tools` failed in its `oxfmt --check` phase with formatting differences across 3,481 files.
- Workspace `check:types` scripts completed across the configured workspaces without reported type errors before the current uncommitted Laya pruning integration was added. Static inspection of the current worktree found that the SDK passes an undeclared `signal` option to `pruneContextWithLaya`, so the earlier result does not cover the present state.
- `bun run lint:py` reported 41 Ruff lint/format findings across Python sources and tests; most output concerned import ordering, exception-type/style rules, and formatting. These were not added as code findings without a demonstrated behavior defect.
- Rust checking could not run because `cargo` is unavailable on this host's PATH. The repository task runner skipped it initially and the forced run failed to find `cargo`.
- At that earlier stage no tests or application behavior had been run; the current results above supersede that limitation for the focused Laya checks.

The report contains confirmed findings from the decision sidecar, roboomp service surfaces, OAuth callback rendering, session storage permissions, coding-agent filesystem/LSP mutation paths, current Laya pruning and subagent-selection changes, TUI debug socket, metaharness API, mnemopi database permissions, Ollama Cloud metadata normalization, native audio queue growth, native VCS symlink traversal, non-Git isolation-diff symlink disclosure, virtual-terminal CSI buffer growth, eval runner staging trust, omptype prototype handling, snapcompact shape validation, Devin gzip expansion, and Bedrock event-stream frame buffering. Provider/runtime and Rust implementation still need deeper manual review; passing earlier type and lint checks do not prove those areas are defect-free. Additional review inspected shared JSON/frontmatter utilities, the virtual terminal parser, browser collab link and AES frame handling, collab-web Markdown rendering, eval bridge/result handling, marketplace plugin source resolution, omptype interpreter/JIT object handling, snapcompact rendering selection, GitHub Copilot header inference, Google ADC/Cloud Code token flows, Ollama request conversion, pi-iso rcopy staging and diff generation, and native HTML/SVG/PDF conversion entry points; no additional confirmed issue was added from those paths.
