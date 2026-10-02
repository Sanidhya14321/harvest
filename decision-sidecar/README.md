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

Authentication: `/v1/decide` requires the per-process secret in the
`x-laya-token` header (or `Authorization: Bearer <token>`). The server logs the
token file location at startup (default `~/.harvest/laya-token`, mode `0600`);
set `LAYA_TOKEN` to use a fixed secret (the TS client sends it from its own
`LAYA_TOKEN` env). Read-only `GET /health` and `GET /v1/hardware` stay
unauthenticated. Set `LAYA_DISABLE_AUTH=1` only for local tests.
Unauthenticated or over-limit requests get `401`/`400`/`413`, all of which the
TS client treats as fallback (tool gating fails CLOSED on fallback).

Request limits (enforced before tokenization): 64 questions max, 500k state
chars aggregate, 100k chars per question chunk, 16k chars per question
definition, 4k chars instructions, ~70k estimated input tokens aggregate.
Inference runs at most 2 concurrent (`LAYA_INFERENCE_TIMEOUT_S`, default 120s),
with at most 8 requests waiting for capacity. A full queue returns 503. Clients
may send positive `metadata.request_timeout_ms` up to the configured server limit;
this budget includes waiting and inference. Harvest sends its remaining client
budget. Expired queued requests never begin inference, and disconnected callers
are checked before dispatch. A timed-out or cancelled running request retains
its slot until synchronous prediction actually finishes: Torch threads cannot be
stopped by cancelling an HTTP waiter. A permanently stuck prediction requires
sidecar restart; it cannot silently oversubscribe inference capacity.

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
Logged fields are length-capped (state kept to a 200-char snippet, instructions to 500 chars) and the log rotates size-based (`decisions.jsonl.1..N`).
Requests may pass optional per-question `ground_truth` labels (`question_id -> 0/1`) which are stored in the log record (default `null`) for later calibration.

---

## 3. Installation & Manual Run

### Dependencies
```sh
pip install -r requirements.txt
```

### Running the Server
```sh
python -m uvicorn server:app --host 127.0.0.1 --port 8177
# or
python server.py --host 127.0.0.1 --port 8177
# (positional `python server.py 127.0.0.1 8177` still works)
```

### Environment Variables
- `LAYA_HOST` (default: `127.0.0.1`) — Host interface binding.
- `LAYA_PORT` (default: `8177`) — Port binding.
- `LAYA_DEVICE` — Optional PyTorch device override (`cpu`, `cuda`, `mps`); invalid values fall back to auto-detection with a warning.
- `LAYA_LOG_DIR` — Custom directory for `decisions.jsonl`.
- `LAYA_TOKEN` — Fixed sidecar secret (otherwise generated per-process). TS client sends it as `x-laya-token`.
- `LAYA_TOKEN_FILE` (default: `~/.harvest/laya-token`) — Where the generated secret is persisted (0600).
- `LAYA_DISABLE_AUTH=1` — Disable `/v1/decide` auth (tests only).
- `LAYA_INFERENCE_TIMEOUT_S` (default: `120`) — Wall-clock inference guard.
- `LAYA_LOG_MAX_BYTES` (default: `10485760`) / `LAYA_LOG_BACKUP_COUNT` (default: `3`) — Decision-log rotation.

---

## 4. Temperature Calibration

Laya models can exhibit overconfidence out-of-the-box. The sidecar includes a dedicated calibration engine in `calibration.py`:

```sh
python calibration.py --decisions decisions.jsonl --out calibration_params.json
# Synthetic bootstrap (never touches the real params file by default):
python calibration.py --synthetic
# Overwrite an existing params file (previous content kept as .bak):
python calibration.py --decisions decisions.jsonl --out calibration_params.json --force
```

**Calibration Pipeline:**
1. Parses historical or synthetic decisions from `decisions.jsonl`.
2. Computes the pre-calibration **Expected Calibration Error (ECE)**.
3. Fits an optimal temperature parameter ($T$) per call site via negative log-likelihood minimization.
4. Generates calibrated probabilities: $\hat{p} = \sigma(z / T)$.
5. Saves learned parameters to `calibration_params.json`, which the server automatically loads at startup.

**Provenance & safety:** saved params carry a top-level `_provenance` marker
(`synthetic: true/false` plus per-site sample counts) so synthetic bootstraps
are never mistaken for measured calibrations. `--synthetic` writes to
`calibration_params.synthetic.json` by default; overwriting any existing params
file requires `--force` (previous content kept as `<file>.bak`). Note: the
shipped `calibration_params.json` is a synthetic bootstrap (150 samples/site,
seed 42) until real `ground_truth`-labelled logs are collected.
