# Harvest Decision Sidecar (Laya Local Decision Layer)

Fast, self-hosted, zero-cloud-cost local decision microservice powered by [Laya](https://github.com/convaiinnovations/laya) and the fine-tuned ModernBERT model [`convaiinnovations/laya-typed-decisions`](https://huggingface.co/convaiinnovations/laya-typed-decisions) (421M parameters, 1024-token context).

The sidecar runs locally as a dedicated Python process listening on `127.0.0.1:8177`, communicating with Harvest's TypeScript/Bun agent loop over low-overhead HTTP.

---

## 1. Overview & Architecture

Harvest delegates narrow classification questions to Laya rather than executing slow, expensive round-trips to the primary LLM:

- **Single-Model Mode**: The sidecar directly invokes `laya.load("convaiinnovations/laya-typed-decisions")` as a module-level singleton. It explicitly avoids the `Router` construct to prevent unintentional background downloads of secondary checkpoints (such as `laya-multilingual`).
- **Language Filtering**: Upstream and in-sidecar guards inspect prompt/state input. If non-English characters or scripts are detected, Laya inference is bypassed immediately and falls back to the main LLM.
- **Strict Fallback Contracts**:
  - **Tool-Call Gating**: Fails **CLOSED** (requires human interactive approval) if the sidecar is offline, times out (300ms default), or predicts irreversible actions ($P > 0.35$).
  - **Model Routing**: Fails **OPEN** (falls back to configured default model tier).
  - **Step Completion**: Fails **OPEN** (falls back to existing full-LLM evaluation check).
- **Temperature-Scaled Calibration**: Pre-calibrated scaling parameters adjust raw overconfident probabilities before decisions reach Harvest's execution gates.

---

## 2. API Reference

The service binds exclusively to `127.0.0.1` (configurable via `--host` / `--port` or environment variables).

### `GET /health`
Returns readiness status, loaded model identity, device, and active port.

**Response:**
```json
{
  "status": "ok",
  "ready": true,
  "model": "convaiinnovations/laya-typed-decisions",
  "device": "cpu",
  "port": 8177
}
```

### `POST /v1/decide`
Executes one or more structured questions against the provided `state`.

**Request Body:**
```json
{
  "state": {
    "tool": "bash",
    "args": { "command": "rm -rf /tmp/scratch" }
  },
  "questions": {
    "irreversibility": {
      "type": "noul",
      "instructions": "does this call write, delete, publish, or change access irreversibly?"
    }
  },
  "metadata": {
    "call_site": "tool_gating",
    "session_id": "optional-uuid"
  }
}
```

**Supported Question Types:**
| Question Type | Output Shape | Used For |
|---|---|---|
| `noul` | `{ noul: float, confidence: float }` | Binary risk / irreversibility gating ($0.0$ to $1.0$). |
| `choice` | `{ answer: string, confidence: float, probabilities: {...} }` | Selecting among model tiers (`smol`, `slow`, `default`) or roles. |
| `score` | `{ score: float, confidence: float }` | Continuous ratings (0 to 10) for completion quality. |

**Audit Logging:**
Every request and computed decision is appended to `decision-sidecar/decisions.jsonl` for continuous observability and offline calibration.

---

## 3. Installation & Manual Run

### Dependencies
```sh
pip install -r requirements.txt
```

### Running the Server
```sh
python -m uvicorn server:app --host 127.0.0.1 --port 8177
```

### Environment Variables
- `LAYA_HOST` (default: `127.0.0.1`) — Host interface binding.
- `LAYA_PORT` (default: `8177`) — Port binding.
- `LAYA_DEVICE` — Optional PyTorch device override (`cpu`, `cuda`, `mps`).
- `LAYA_LOG_DIR` — Custom directory for `decisions.jsonl`.

---

## 4. Temperature Calibration

Laya models can exhibit overconfidence out-of-the-box. The sidecar includes a dedicated calibration engine in `calibration.py`:

```sh
python calibration.py --decisions decisions.jsonl --out calibration_params.json
```

**Calibration Pipeline:**
1. Parses historical or synthetic decisions from `decisions.jsonl`.
2. Computes the pre-calibration **Expected Calibration Error (ECE)**.
3. Fits an optimal temperature parameter ($T$) per call site via negative log-likelihood minimization.
4. Generates calibrated probabilities: $\hat{p} = \sigma(z / T)$.
5. Saves learned parameters to `calibration_params.json`, which the server automatically loads at startup.
