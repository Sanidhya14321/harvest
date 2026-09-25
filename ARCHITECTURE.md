# Architecture: Laya ModernBERT Decision Sidecar Integration

Harvest integrates **Laya** (`convaiinnovations/laya-typed-decisions`), a lightweight, local ModernBERT-large decision model, as a dedicated sidecar service. The integration delivers zero-API-cost, high-speed typed evaluations directly on the user's machine to govern:

1. **Phase 1: Tool Output Pruning & Prompt Cache-Locking** (preserving LLM KV cache prefix while pruning stale context).
2. **Phase 2: Specialized Subagent Selection & Shadow Mode** (classifying user tasks to route to the optimal subagent).
3. **Hardware-Aware Self-Calibration** (empirically benchmarking local hardware to derive timeouts and activation thresholds without hardcoded assumptions).
4. **Single-Command Installer & Bounded Self-Healing Setup** (idempotent, transparent deployment with bounded self-repair for 8 enumerated failure modes and fail-open graceful degradation).

---

## 1. System Overview & Component Topology

```
+-----------------------------------------------------------------------------------+
|                               Harvest Coding Agent (CLI)                          |
|                                                                                   |
|  +---------------------------+   +----------------------------+   +------------+  |
|  |   Phase 1: Context Pruner |   | Phase 2: Subagent Selector |   | Calibrator |  |
|  |   (Prompt Cache-Locked)   |   |   (Shadow Review & Audit)  |   | (4 Shapes) |  |
|  +-------------+-------------+   +--------------+-------------+   +------+-----+  |
|                |                                |                        |        |
|                +--------------------------------+------------------------+        |
|                                                 | HTTP JSON-RPC                   |
+-------------------------------------------------|---------------------------------+
                                                  v
+-----------------------------------------------------------------------------------+
|               Laya Decision Sidecar (http://127.0.0.1:8177 [8178-8185])           |
|                                                                                   |
|  FastAPI Application (lifespan model load, asyncio concurrency semaphore = 2)     |
|  Model: convaiinnovations/laya-typed-decisions (~842MB single checkpoint)         |
|  PyTorch Runtime: CUDA 12.4 (NVIDIA) | MLX / MPS (Apple Silicon) | CPU (8 cores)  |
|  Platform Hardening: WindowsSelectorEventLoopPolicy, _safe_symlink copy fallback  |
+-----------------------------------------------------------------------------------+
```

### Architectural Principles

- **Zero API Cost**: Decision questions (classification, relevance scoring, sanity checks) run entirely on local silicon without outbound network requests or per-token charges.
- **Fail-Open / Non-Blocking**: If the decision sidecar is offline, times out, or encounters an internal exception, Harvest immediately falls back to safe defaults (full unpruned context, caller/heuristic routing). Core Harvest functionality is never blocked.
- **Strictly Single-Checkpoint**: The sidecar exclusively loads `convaiinnovations/laya-typed-decisions` via `laya.load()`. `laya.Router` (which triggers secondary multi-gigabyte downloads like `laya-multilingual`) is structurally audited and barred.
- **Empirically Derived, Never Guessed**: Thresholds and timeouts are computed from live latency benchmarks run on the user's specific hardware during calibration, not static lookup tables.

---

## 2. Decision Sidecar Architecture (`decision-sidecar/server.py`)

### Model Lifecycle & Concurrency
- **Lifespan Startup**: Model weights are loaded once into memory during FastAPI's `@asynccontextmanager` startup lifecycle. Requests never reload the model.
- **Inference Semaphore**: Concurrent requests are throttled via an `asyncio.Semaphore(2)` to prevent out-of-memory errors or CPU saturation during heavy batching.
- **PyTorch Threading**: On CPU devices, PyTorch thread count is explicitly configured to logical CPU cores (`torch.set_num_threads(os.cpu_count() or 4)`), enabling SIMD parallelization via MKL/oneDNN.

### Platform Hardening
- **Windows Event Loop Policy**: On Windows, the sidecar configures `asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())` to prevent IOCP `WinError 64` (`ERROR_NETNAME_DELETED`) when client connections close prematurely.
- **Symlink Privilege Bypass (WinError 1314)**: HuggingFace cache relies on filesystem symlinks. On Windows machines lacking Developer Mode / `SeCreateSymbolicLinkPrivilege`, `os.symlink` is monkey-patched with `_safe_symlink`, falling back transparently to `shutil.copyfile` and `shutil.copytree` when error 1314 or errno 1 is raised.

---

## 3. Phase 1: Context Pruning & Prompt Cache-Locking (`src/core/harvest/laya-pruning.ts`)

### Prompt Cache-Locking Contract
Modern frontier LLMs (Anthropic Claude, OpenAI, Google Gemini) utilize prompt prefix caching. In-place deletion of early conversation turns breaks the cache prefix, forcing full recomputation at 10x the latency and cost.

1. **Prefix Invariance**: System prompts, project declarations, and initial user prompt turns are strictly locked against pruning.
2. **Intermediate Pruning**: Only aged-out tool result outputs situated after the stable cache boundary are considered for relevance scoring.
3. **Hardware Gate**: If calibration benchmarks show that evaluating candidate chunks on CPU exceeds the user's configured per-turn latency budget (`maxAcceptableLatencyPerTurnMs`, default 150ms), pruning is automatically disabled to preserve interactive turn responsiveness.

---

## 4. Phase 2: Subagent Selection & Shadow Mode (`src/core/harvest/laya-subagent-selection.ts`)

### Task Classification
When an agent or user dispatches a subtask, Laya classifies the prompt using a single-choice decision shape across 5 candidate subagent roles:
- `scout`: Fast, read-only exploratory codebase search and pattern location.
- `reviewer`: Code quality, regression, and pull-request analysis.
- `security-reviewer`: Evidence-backed vulnerability discovery and CWE analysis.
- `sonic`: Low-reasoning agent for mechanical edits, formatting, or bulk changes.
- `task`: Multi-step implementation, complex coding, refactoring.

### Shadow Mode & Human Review
To validate accuracy without risking misrouting:
- **Shadow Mode**: Evaluates incoming tasks in parallel with caller selection, logging audit traces to `~/.harvest/agent/laya-subagent-audit.jsonl` with confidence scores and agreement metrics.
- **Shadow Review CLI (`harvest laya review-shadow`)**: Interactive TUI for inspecting disagreements between Laya and caller selection.
- **Empirical Recalibration (`harvest laya recalibrate-subagent`)**: Derives optimal confidence thresholds from human-labeled feedback using ROC curve analysis.

---

## 5. Hardware Self-Calibration Engine (`src/core/harvest/laya-calibration.ts`)

Instead of hardcoding device profiles or checking model strings in TypeScript, Harvest runs a local benchmark across 4 representative payload shapes:

| Benchmark Shape | Payload Size | Harvest Call Site Analogy |
|---|---|---|
| `ultraShort` | ~40 tokens | Sanity baseline (noul question) |
| `singleChoice` | ~350 tokens | Phase 2 Subagent selection |
| `singleScore` | ~150 tokens | Phase 1 Short tool result scoring |
| `batchedScore` | B=2, L=1024 | Phase 1 Typical candidate pool |

### Derived Settings Contract
- `subagentSelectionTimeoutMs`: Set to `2 * median(singleChoice.latencyMs)`, guaranteeing requests complete within safe bounds before falling back.
- `subagentSelectionRecommendEnabled`: Auto-enabled when single-choice latency is within acceptable interactive range.
- `pruningRecommendEnabled`: Enabled only if estimated per-turn chunk scoring latency is less than `maxAcceptableLatencyPerTurnMs`.
- `signature`: SHA-256 / hex hash of CPU cores, GPU driver status, and architecture. If hardware changes, the system flags the calibration as stale.

---

## 6. Single-Command Installer & Bounded Self-Healing Framework

Harvest deploys Laya through a unified installer with **bounded self-healing** across 8 strictly enumerated failure signatures. The installer never attempts open-ended or arbitrary repairs.

### The 8 Known Failure Modes & Remediation Contracts

| Signature | Detection Mechanism | Bounded Remediation | Logging |
|---|---|---|---|
| `WRONG_TORCH_WHEEL` | Host NVIDIA GPU driver present (`nvcuda.dll`/`libcuda.so`), but `torch.cuda.is_available()` is `False`. | Reinstalls PyTorch from CUDA 12.4 wheel index (`https://download.pytorch.org/whl/cu124`). | Logs before/after `torch.cuda.is_available()` to `laya-setup.log`. |
| `WINDOWS_HF_SYMLINK_RESTRICTION` | `WinError 1314` or privilege error during cache writes. | Sets `HF_HUB_DISABLE_SYMLINKS=1` and `HF_HUB_DISABLE_SYMLINKS_WARNING=1` with safe copy fallback. | Logs bypass activation to `laya-setup.log`. |
| `PORT_CONFLICT` | Port 8177 occupied on startup. | Checks `/health`: reuses active Harvest sidecar; if foreign process, preserves foreign process (never terminates) and allocates alternate port in range `8178-8185`. | Logs reuse or redirection without process termination. |
| `CORRUPTED_CHECKPOINT` | Cached `model.safetensors` < 800MB or corrupt header on `safe_open`. | Purges corrupt snapshot directory and retries clean download **exactly once** (halts on second failure). | Logs corruption signature, purge event, and retry outcome. |
| `ROUTER_ACCIDENTAL_INVOCATION` | Static audit reveals `Router` import in `server.py` or secondary checkpoint in cache. | Enforces single checkpoint contract (`laya.load('convaiinnovations/laya-typed-decisions')`). | Logs architecture assertion result. |
| `INSUFFICIENT_DISK_SPACE` | Pre-flight `statfs` check detects < 2,000 MB available on cache volume. | Halts download before starting, preventing partial write corruption. | Logs available vs required MB. |
| `STALE_CALIBRATION_SIGNATURE` | Hardware signature in `laya-calibration.json` does not match active hardware signature. | Triggers automatic recalibration benchmark to derive fresh latency thresholds. | Logs signature mismatch and recalibration event. |
| `MISSING_PYTHON` | Python 3.9+ binary missing from PATH and standard directories. | Autonomous bootstrap via system package manager (`winget`/`brew`) or emits clear OS-specific commands. | Logs missing runtime and tailored installation instructions. |

### Unrecognized Failures & Diagnostic Bundling
If an unexpected failure occurs outside the 8 enumerated signatures:
1. A structured JSON diagnostic bundle is generated at `~/.harvest/agent/logs/laya-diagnostic-<timestamp>.json` capturing OS release, Bun version, Python runtime, PyTorch CUDA status, available disk space, active port, and the last 50 log lines.
2. The diagnostic bundle is forwarded to the second-tier LLM-assisted diagnosis engine (detailed below).
3. If LLM diagnosis is unavailable or declined, setup halts cleanly with fail-open status and instructions for manual review.

### Graceful Degradation (Fail-Open Guarantee)
If Laya setup fails irrecoverably:
- Core Harvest functionality is **never blocked**.
- `laya.enabled` is set to `false` in settings.
- The installer exits with clear notice that Core Harvest is operational while Laya decision features are disabled.

### Idempotency Fast-Path
When `harvest setup laya` is run on an already configured machine:
- Probes `/health` and runs `/v1/decide` smoke test.
- Verifies calibration signature against current hardware.
- If verified, completes in **< 100ms** without re-downloading packages or model weights.

---

## 7. LLM-Assisted Failure Diagnosis (Second-Tier Recovery)

When an unexpected error does not match any of the 8 deterministic failure signatures, Harvest activates a second-tier recovery analysis using Harvest's already-configured main LLM connection (`src/core/harvest/laya-llm-diagnosis.ts`).

```
+-------------------------------------------------------------------------+
|                  Step 2: Deterministic Self-Healing                     |
|           (Modes 1-8: Wheel, Symlink, Port, Checkpoint, etc.)           |
+------------------------------------+------------------------------------+
                                     |
                          [No Known Signature Match]
                                     |
                                     v
+-------------------------------------------------------------------------+
|                   Diagnostic Bundle Assembled & Dumped                  |
|          (~/.harvest/agent/logs/laya-diagnostic-<timestamp>.json)       |
+------------------------------------+------------------------------------+
                                     |
                                     v
+-------------------------------------------------------------------------+
|              Tier 2: Harvest Main LLM Connection (Zero New APIs)        |
|    Prompt: src/prompts/system/laya-diagnosis-system.md                  |
|    Returns: diagnosis, proposedFix, suggestedAction, riskLevel          |
+------------------------------------+------------------------------------+
                                     |
                                     v
+-------------------------------------------------------------------------+
|             Independent Safety & Risk Evaluation Gate                   |
+------------------------------------+------------------------------------+
           /                                              \
 [Low-Risk Reversible]                       [Touches System State / Ambiguous]
 (venv package install / retry)              (ports, processes, file deletion)
           |                                              |
           v                                              v
 [Auto-Applied & Logged]                     [Explicit User Confirmation Required]
 "auto-applied: <what, why>"                 (NEVER auto-applied unilaterally)
```

### Key Architectural Invariants
1. **Zero New APIs / Zero Credential Drift**: Uses the user's existing configured model from `Settings` and `ModelRegistry` (e.g. `default` or `smol` role).
2. **Cost-Free on Successful Runs**: The LLM diagnosis tier only fires on genuinely unrecognized failures, never during normal successful setup or known deterministic fixes.
3. **Independent Risk Classification Gate**: The LLM proposes, but never acts unilaterally. An independent validator (`evaluateRiskClassification`) inspects all proposed actions against a strict pattern engine. Even if the LLM self-assesses an action as "low risk", any action touching ports, terminating processes, deleting files outside cache, or escalating privileges is unconditionally overridden to `touches-system-state`.
4. **Transparent Audit Logging**: Every LLM diagnosis input bundle, output recommendation, and auto-applied fix is distinctly logged to `~/.harvest/agent/logs/laya-setup.log`. Auto-applied actions are recorded as `[LLM_DIAGNOSIS] auto-applied: <what, why>`.
5. **Fail-Open Graceful Degradation**: If the network is down, the provider fails, or no LLM credentials exist, Harvest degrades cleanly to the diagnostic dump without throwing or blocking Core Harvest.
6. **No Automatic Signature Mutation**: A successful LLM-assisted repair is never automatically promoted to the deterministic 8-mode table. Promotion requires manual evidence, code change, and testing.

---

## 8. Verification Matrix & Hardware Environment

| Platform | Tier | Sidecar Daemon | Calibration | Bounded Self-Healing | LLM Diagnosis | Verification Status |
|---|---|---|---|---|---|---|
| **Windows 11 x64** | CPU (Intel 8 cores) | Operational | Verified (3115ms choice) | All 8 modes verified | Verified (low/high-risk & fallback) | **TESTED & VERIFIED** on local testbed |
| **Windows x64** | NVIDIA CUDA | Implemented | Hardware-adaptive | Auto-remedies CPU wheel to cu124 | Fail-open fallback | *UNTESTED* (requires GPU testbed) |
| **macOS Apple Silicon** | MLX / MPS | Implemented | Hardware-adaptive | Pre-flight disk & venv isolation | Fail-open fallback | *UNTESTED* (requires Apple Silicon host) |
| **Linux x86_64** | CUDA / CPU | Implemented | Hardware-adaptive | Safe port & venv isolation | Fail-open fallback | *UNTESTED* (requires Linux host) |

