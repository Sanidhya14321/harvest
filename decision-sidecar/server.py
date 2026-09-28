"""Laya Local Decision Sidecar Service.

Provides fast, typed decision endpoints powered by convaiinnovations/laya-typed-decisions.
Runs as a local HTTP service on 127.0.0.1:8177.
"""

from __future__ import annotations

import argparse
import asyncio
import hmac
import json
import logging
import os
import secrets
import sys
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, Dict, Optional, Union

from fastapi import FastAPI, HTTPException, Request, Response, status
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel, Field
import uvicorn

import torch
import laya

try:
    from hardware import detect_hardware
except ImportError:
    from .hardware import detect_hardware

try:
    from bucketing import bucket_items_by_length
except ImportError:
    from .bucketing import bucket_items_by_length

try:
    from calibration import CalibrationManager, CALIBRATION_PARAMS_PATH
except ImportError:
    from .calibration import CalibrationManager, CALIBRATION_PARAMS_PATH

if sys.platform == "win32":
    # Prevent IOCP WinError 64 (ERROR_NETNAME_DELETED) on abrupt client disconnections
    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())
    # Handle WinError 1314 (missing symlink privilege) when huggingface_hub caches files
    import shutil
    _orig_symlink = getattr(os, "symlink", None)
    if _orig_symlink:
        def _safe_symlink(src, dst, target_is_directory=False, *args, **kwargs):
            try:
                return _orig_symlink(src, dst, target_is_directory=target_is_directory, *args, **kwargs)
            except OSError as e:
                if getattr(e, "winerror", None) == 1314 or getattr(e, "errno", None) == 1:
                    src_full = src if os.path.isabs(src) else os.path.normpath(os.path.join(os.path.dirname(dst), src))
                    if os.path.isdir(src_full):
                        return shutil.copytree(src_full, dst, dirs_exist_ok=True)
                    else:
                        return shutil.copyfile(src_full, dst)
                raise
        os.symlink = _safe_symlink

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] [laya-sidecar] %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)
logger = logging.getLogger("laya-sidecar")

MODEL_ID = "convaiinnovations/laya-typed-decisions"
DEFAULT_HOST = os.getenv("LAYA_HOST", "127.0.0.1")
DEFAULT_PORT = int(os.getenv("LAYA_PORT", "8177"))
log_dir = os.getenv("LAYA_LOG_DIR")
if log_dir:
    LOG_FILE_PATH = Path(log_dir) / "decisions.jsonl"
else:
    LOG_FILE_PATH = Path(os.getenv("LAYA_LOG_FILE", Path(__file__).parent / "decisions.jsonl"))

MAX_QUESTIONS = 64
MAX_STATE_CHARS = 500_000
# Per-question/chunk caps: dict states carry one chunk per question, so the
# aggregate char cap above cannot bound per-chunk tokenization work.
MAX_QUESTION_STATE_CHARS = 100_000
MAX_QUESTION_DEF_CHARS = 16_384
MAX_INSTRUCTIONS_CHARS = 4_000
MAX_METADATA_CHARS = 4_096
# Aggregate compute cap estimated BEFORE tokenization/batch allocation
# (chars//4 heuristic + per-question overhead), so hostile inputs cannot drive
# the batched path to its 64x1024-token worst case.
MAX_EST_INPUT_TOKENS = 70_000

# Inference concurrency and wall-clock guard. The semaphore bounds parallel
# inferences; torch threads are divided by the same concurrency (see lifespan)
# to avoid Semaphore(N) x num_threads oversubscription.
INFERENCE_CONCURRENCY = 2
INFERENCE_TIMEOUT_S = float(os.getenv("LAYA_INFERENCE_TIMEOUT_S", "120"))

# Decision-log bounds: size-based rotation plus per-field caps so hostile
# payloads are never logged verbatim beyond a short snippet.
LOG_MAX_BYTES = int(os.getenv("LAYA_LOG_MAX_BYTES", str(10 * 1024 * 1024)))
LOG_BACKUP_COUNT = int(os.getenv("LAYA_LOG_BACKUP_COUNT", "3"))
LOG_STATE_SNIPPET_CHARS = 200
LOG_INSTRUCTIONS_CHARS = 500
LOG_SESSION_ID_CHARS = 128
LOG_CALL_SITE_CHARS = 64

# Header (or Authorization: Bearer) carrying the per-process sidecar secret.
AUTH_HEADER = "x-laya-token"
_auth_token: Optional[str] = None

# Module-level singleton and hardware state
_hardware_info: Dict[str, Any] = detect_hardware()
_agent: Optional[laya.agent.Agent] = None
_model_ready: bool = False
_device: str = _hardware_info.get("device", "cpu")
_device_override = os.getenv("LAYA_DEVICE")
if _device_override:
    if _device_override in ("cpu", "cuda", "mps"):
        _device = _device_override
    else:
        logger.warning(f"Ignoring invalid LAYA_DEVICE={_device_override!r}; using detected device {_device!r}")
_calibration_manager: CalibrationManager = CalibrationManager()
_inference_semaphore: Optional[asyncio.Semaphore] = None


def get_inference_semaphore() -> asyncio.Semaphore:
    global _inference_semaphore
    if _inference_semaphore is None:
        _inference_semaphore = asyncio.Semaphore(INFERENCE_CONCURRENCY)
    return _inference_semaphore


def _default_token_path() -> Path:
    return Path(os.getenv("LAYA_TOKEN_FILE", str(Path.home() / ".harvest" / "laya-token")))


def auth_enabled() -> bool:
    """Auth is on unless explicitly disabled (tests / local dev without a client)."""
    return os.getenv("LAYA_DISABLE_AUTH", "0").lower() not in ("1", "true", "yes")


def get_auth_token() -> str:
    """Return the per-process secret guarding /v1/decide.

    Uses LAYA_TOKEN when set (client workstream sends it via header);
    otherwise generates a secret at startup and persists it 0600 so the
    local TS client can read it back.
    """
    global _auth_token
    if _auth_token is not None:
        return _auth_token
    env_token = os.getenv("LAYA_TOKEN")
    if env_token:
        _auth_token = env_token
        return _auth_token
    _auth_token = secrets.token_urlsafe(32)
    path = _default_token_path()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            os.write(fd, _auth_token.encode("utf-8"))
        finally:
            os.close(fd)
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
    except Exception as e:
        logger.warning(f"Could not persist Laya auth token to {path}: {e}")
    return _auth_token


def verify_request_auth(request: Request) -> None:
    """Require the sidecar secret on mutation/inference endpoints. Read-only
    diagnostics (/health, /v1/hardware) stay unauthenticated."""
    if not auth_enabled():
        return
    expected = get_auth_token()
    provided = request.headers.get(AUTH_HEADER)
    if not provided:
        bearer = request.headers.get("authorization", "")
        if bearer.lower().startswith("bearer "):
            provided = bearer[7:].strip()
    if not provided or not hmac.compare_digest(provided, expected):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Missing or invalid sidecar auth token",
        )


def get_agent() -> laya.agent.Agent:
    """Return the loaded Laya agent singleton."""
    global _agent
    if _agent is None:
        raise RuntimeError("Laya agent model has not been initialized yet.")
    return _agent


def _rotate_log_if_needed() -> None:
    """Size-based rotation: decisions.jsonl -> .1 -> .2 ... keeping LOG_BACKUP_COUNT."""
    try:
        if LOG_BACKUP_COUNT < 1:
            return
        if not LOG_FILE_PATH.exists():
            return
        if LOG_FILE_PATH.stat().st_size < LOG_MAX_BYTES:
            return
        oldest = Path(str(LOG_FILE_PATH) + f".{LOG_BACKUP_COUNT}")
        try:
            if oldest.exists():
                oldest.unlink()
        except OSError:
            pass
        for i in range(LOG_BACKUP_COUNT - 1, 0, -1):
            src = Path(str(LOG_FILE_PATH) + f".{i}")
            dst = Path(str(LOG_FILE_PATH) + f".{i + 1}")
            try:
                if src.exists():
                    os.replace(src, dst)
            except OSError:
                pass
        try:
            os.replace(LOG_FILE_PATH, Path(str(LOG_FILE_PATH) + ".1"))
        except OSError as e:
            logger.warning(f"Decision log rotation failed: {e}")
    except Exception as e:
        logger.warning(f"Decision log rotation check failed: {e}")


def log_decision_record(record: dict[str, Any]) -> None:
    """Append structured decision log for calibration and observability."""
    try:
        LOG_FILE_PATH.parent.mkdir(parents=True, exist_ok=True)
        _rotate_log_if_needed()
        with open(LOG_FILE_PATH, "a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
    except Exception as e:
        logger.warning(f"Failed to write decision log: {e}")


def _truncate(value: Any, limit: int) -> str:
    text = value if isinstance(value, str) else str(value)
    return text[:limit]



@asynccontextmanager
async def lifespan(app: FastAPI):
    """Load model once at startup, never reload per request."""
    global _agent, _model_ready, _device
    logger.info(
        f"Selected hardware tier '{_hardware_info['tier']}' on '{_hardware_info['device_name']}' "
        f"[signature: {_hardware_info['signature']}]. Loading {MODEL_ID} on {_device}..."
    )
    start_time = time.perf_counter()
    try:
        device_str = _device if isinstance(_device, str) else getattr(_device, "type", "cpu")
        if device_str == "cpu":
            num_cores = os.cpu_count() or 4
            # Divide the pool by inference concurrency: Semaphore(N) workers
            # share this pool, so N x cpu_count threads would oversubscribe.
            num_threads = max(1, num_cores // INFERENCE_CONCURRENCY)
            torch.set_num_threads(num_threads)
            logger.info(f"Configured PyTorch CPU thread pool: {torch.get_num_threads()} threads")

        if auth_enabled():
            token_path = _default_token_path()
            get_auth_token()
            logger.info(
                f"Sidecar auth enabled; token file: {token_path} (mode 0600). "
                f"Send via '{AUTH_HEADER}' header or Authorization: Bearer."
            )
        else:
            logger.warning("Sidecar auth DISABLED via LAYA_DISABLE_AUTH; /v1/decide accepts unauthenticated loopback requests.")

        # Strictly load the single checkpoint, avoiding Router which pulls laya-multilingual
        _agent = laya.load(MODEL_ID, device=_device)
        _agent.model.eval()
        _model_ready = True
        elapsed = (time.perf_counter() - start_time) * 1000
        logger.info(f"Loaded {MODEL_ID} successfully in {elapsed:.1f}ms (eval_mode={not _agent.model.training})")
    except Exception as e:
        logger.error(f"Failed to load {MODEL_ID}: {e}", exc_info=True)
        _model_ready = False
        raise

    yield

    logger.info("Shutting down Laya sidecar service.")


app = FastAPI(
    title="Laya Decision Sidecar",
    version="1.0.0",
    description="Local typed decision microservice for Harvest agent harness",
    lifespan=lifespan,
)


class DecideRequest(BaseModel):
    state: Union[str, Dict[str, Any], list[Any]] = Field(
        ..., description="Input state representation (string, JSON object, or list)"
    )
    questions: Dict[str, Dict[str, Any]] = Field(
        ..., description="Dictionary mapping question_id -> question definition"
    )
    metadata: Optional[Dict[str, Any]] = Field(
        default=None, description="Optional caller context (call_site, session_id, etc.)"
    )
    ground_truth: Optional[Dict[str, Any]] = Field(
        default=None,
        description="Optional per-question outcome labels for calibration (question_id -> 0/1)",
    )


class DecideResponse(BaseModel):
    answers: Dict[str, Any]
    usage: Optional[Dict[str, Any]] = None
    latency_ms: float
    model: str = MODEL_ID
    non_english: bool = False


@app.get("/health")
async def health_check() -> Dict[str, Any]:
    """Health check endpoint: ready/not-ready status and hardware summary."""
    return {
        "status": "ok" if _model_ready else "loading",
        "ready": _model_ready,
        "model": MODEL_ID,
        "device": _device,
        "device_name": _hardware_info.get("device_name", "unknown"),
        "hardware_tier": _hardware_info.get("tier", "unknown"),
        "hardware_signature": _hardware_info.get("signature", "unknown"),
        "port": DEFAULT_PORT,
    }


@app.get("/v1/hardware")
async def get_hardware() -> Dict[str, Any]:
    """Return detailed hardware detection result, device diagnostics, and signature."""
    return _hardware_info


def _state_chunks(state: Union[str, Dict[str, Any], list], question_ids: list) -> list:
    """Split state into one normalized string chunk per question.

    Dict states may carry per-question chunks (batched multi-chunk scoring);
    every chunk is normalized via str() so language detection and token
    estimation see the same text the model will score.
    """
    if isinstance(state, dict) and any(qid in state for qid in question_ids):
        return [str(state.get(qid, state)) for qid in question_ids]
    return [str(state)]


def _check_english_gate(state: Union[str, Dict[str, Any], list], question_ids: list, call_site: str) -> bool:
    """Return True when every question chunk is English.

    Detection errors fail CLOSED for tool_gating (503 -> TS client falls back
    to requiring human approval) and fail OPEN otherwise, per README contracts.
    """
    try:
        chunks = _state_chunks(state, question_ids)
    except Exception as e:
        logger.warning(f"Language-detection normalization failed: {e}")
        if call_site == "tool_gating":
            raise HTTPException(
                status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
                detail="Language detection unavailable; failing closed for tool_gating",
            )
        return True
    try:
        return all(laya.is_english(chunk) for chunk in chunks)
    except Exception as e:
        logger.warning(f"Language detection check failed: {e}")
        if call_site == "tool_gating":
            raise HTTPException(
                status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
                detail="Language detection unavailable; failing closed for tool_gating",
            )
        return True


def _estimate_input_tokens(state: Union[str, Dict[str, Any], list], question_ids: list) -> int:
    """Heuristic aggregate token estimate (chars//4 + per-question overhead)."""
    chunks = _state_chunks(state, question_ids)
    if len(chunks) == 1:
        return len(chunks[0]) // 4 + len(question_ids) * 256
    return sum(min(len(c), MAX_QUESTION_STATE_CHARS) // 4 + 256 for c in chunks)


def _validate_request(req: DecideRequest) -> str:
    """Enforce count / char / estimated-token caps before tokenization.

    Returns the aggregate state string for downstream use.
    """
    if len(req.questions) > MAX_QUESTIONS:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail=f"Question count {len(req.questions)} exceeds maximum limit of {MAX_QUESTIONS}",
        )
    if req.metadata is not None and len(json.dumps(req.metadata, ensure_ascii=False)) > MAX_METADATA_CHARS:
        raise HTTPException(
            status_code=status.HTTP_413_REQUEST_ENTITY_TOO_LARGE,
            detail=f"Metadata size exceeds maximum limit of {MAX_METADATA_CHARS} characters",
        )
    state_str = str(req.state)
    if len(state_str) > MAX_STATE_CHARS:
        raise HTTPException(
            status_code=status.HTTP_413_REQUEST_ENTITY_TOO_LARGE,
            detail=f"State size {len(state_str)} exceeds maximum limit of {MAX_STATE_CHARS} characters",
        )
    ids = list(req.questions.keys())
    if isinstance(req.state, dict) and any(qid in req.state for qid in ids):
        for qid in ids:
            chunk_len = len(str(req.state.get(qid, req.state)))
            if chunk_len > MAX_QUESTION_STATE_CHARS:
                raise HTTPException(
                    status_code=status.HTTP_413_REQUEST_ENTITY_TOO_LARGE,
                    detail=f"State for question {qid!r} ({chunk_len} chars) exceeds per-question limit of {MAX_QUESTION_STATE_CHARS}",
                )
    for qid, q_def in req.questions.items():
        if not isinstance(q_def, dict):
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=f"Question {qid!r} must be an object",
            )
        if len(json.dumps(q_def, ensure_ascii=False)) > MAX_QUESTION_DEF_CHARS:
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=f"Question {qid!r} definition exceeds maximum size of {MAX_QUESTION_DEF_CHARS} characters",
            )
        if len(str(q_def.get("instructions", ""))) > MAX_INSTRUCTIONS_CHARS:
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=f"Question {qid!r} instructions exceed maximum of {MAX_INSTRUCTIONS_CHARS} characters",
            )
    est_tokens = _estimate_input_tokens(req.state, ids)
    if est_tokens > MAX_EST_INPUT_TOKENS:
        raise HTTPException(
            status_code=status.HTTP_413_REQUEST_ENTITY_TOO_LARGE,
            detail=f"Estimated input size {est_tokens} tokens exceeds maximum of {MAX_EST_INPUT_TOKENS}",
        )
    return state_str


def _predict_sync(state: Union[str, Dict[str, Any], list], questions: Dict[str, Dict[str, Any]]) -> Dict[str, Any]:
    """Blocking inference body, run via asyncio.to_thread so the timeout applies."""
    agent = get_agent()
    ids = list(questions.keys())
    per_question = isinstance(state, dict) and any(qid in state for qid in ids)

    if per_question or len(ids) > 1:
        import numpy as np
        import torch
        from laya.agent import (
            QTYPES,
            build_sequence,
            collate_items,
            render_options,
            temp_bucket,
            confidence_from_probs,
        )

        max_len = agent.cfg.get("max_len", 1024)
        head_max_len = agent.cfg.get("head_max_len", 256)

        raw_items = []
        for qid in ids:
            q = agent._to_internal(questions[qid])
            q_state = state.get(qid, state) if isinstance(state, dict) else state
            seq, markers = build_sequence(agent.tok, q_state, q, max_len, head_max_len)
            if len(markers) != len(render_options(q)):
                raise ValueError(f"question {qid!r} options exceed head_max_len={head_max_len}")
            raw_items.append({
                "qid": qid,
                "q": q,
                "seq": seq,
                "markers": markers,
                "qtype": QTYPES[q["t"]],
            })

        buckets = bucket_items_by_length(raw_items)
        answers = {}
        total_tokens = 0
        use_amp = agent.device.type == "cuda"

        for bucket in buckets:
            b_items = [{"ids": it["seq"], "markers": it["markers"], "qtype": it["qtype"]} for it in bucket]
            b = collate_items([b_items], agent.tok.pad_token_id)
            total_tokens += int(b["attention_mask"].sum())

            with torch.no_grad():
                with torch.autocast(device_type=agent.device.type, dtype=agent.dtype, enabled=use_amp):
                    logits, act = agent.model(
                        b["input_ids"].to(agent.device),
                        b["attention_mask"].to(agent.device),
                        b["marker_pos"].to(agent.device),
                        b["marker_mask"].to(agent.device),
                        b["qtype"].to(agent.device),
                    )

                b_logits = logits.detach().float().cpu().numpy()
                b_act = torch.softmax(act.detach().float(), -1).cpu().numpy()

            for r, it in enumerate(bucket):
                qid = it["qid"]
                q = it["q"]
                k = len(it["markers"])
                qt = it["qtype"]
                t_scale = agent.temperature_by_options.get(temp_bucket(qt, k), agent.temperature[qt])
                z = b_logits[r, :k] / t_scale
                p = np.exp(z - z.max())
                p = p / p.sum()

                conf_score = round(confidence_from_probs(p, k), 4)
                ext = {"act_probability": round(float(b_act[r, 0]), 4)}

                if q["t"] == "choice":
                    keys = list(q["crit"].keys())
                    answers[qid] = {
                        "type": "choice",
                        "choice": keys[int(p.argmax())],
                        "probabilities": {kk: round(float(v), 4) for kk, v in zip(keys, p)},
                        "confidence": conf_score,
                        "action": ext,
                    }
                elif q["t"] == "score":
                    exp_score = float((np.arange(k) * p).sum())
                    answers[qid] = {
                        "type": "score",
                        "score": round(exp_score, 4),
                        "legend": {str(i): c for i, c in enumerate(q["crit"])},
                        "probabilities": {str(i): round(float(v), 4) for i, v in enumerate(p)},
                        "confidence": conf_score,
                        "action": ext,
                    }
                else:
                    answers[qid] = {
                        "type": "noul",
                        "noul": round(float(p[1]), 4),
                        "confidence": round(max(float(p[1]), 1.0 - float(p[1])), 4),
                        "action": ext,
                    }

        return {
            "model": "laya-rl-agent",
            "answers": answers,
            "usage": {"input_tokens": total_tokens, "output_tokens": 0},
        }
    # Run prediction on the singleton agent
    return agent.predict(state=state, questions=questions)


@app.post("/v1/decide", response_model=DecideResponse)
async def decide(req: DecideRequest, request: Request) -> DecideResponse:
    """Evaluate typed decision questions against state in a single parallel pass."""
    verify_request_auth(request)
    if not _model_ready or _agent is None:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Model is still loading or unavailable",
        )

    state_str = _validate_request(req)
    state = req.state

    # Check English language requirement per question chunk.
    # ModernBERT-large English checkpoint will hallucinate or degrade on non-English text.
    call_site = str((req.metadata or {}).get("call_site", "default"))
    if not _check_english_gate(state, list(req.questions.keys()), call_site):
        logger.info("Non-English state detected; rejecting for main-LLM fallback")
        return DecideResponse(
            answers={},
            latency_ms=0.0,
            model=MODEL_ID,
            non_english=True,
        )

    semaphore = get_inference_semaphore()
    async with semaphore:
        start_time = time.perf_counter()
        try:
            result = await asyncio.wait_for(
                asyncio.to_thread(_predict_sync, state, req.questions),
                timeout=INFERENCE_TIMEOUT_S,
            )
        except asyncio.TimeoutError:
            logger.error(f"Inference timed out after {INFERENCE_TIMEOUT_S}s; semaphore slot released")
            raise HTTPException(
                status_code=status.HTTP_504_GATEWAY_TIMEOUT,
                detail=f"Laya inference timed out after {INFERENCE_TIMEOUT_S}s",
            )
        except Exception as e:
            logger.error(f"Inference error during predict: {e}", exc_info=True)
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail=f"Laya inference error: {str(e)}",
            )

    latency_ms = (time.perf_counter() - start_time) * 1000
    answers = result.get("answers", {})
    usage = result.get("usage", {})

    # Apply calibration per call-site
    for qid, ans in answers.items():
        if isinstance(ans, dict) and "confidence" in ans and ans["confidence"] is not None:
            raw_conf = float(ans["confidence"])
            calibrated_conf = round(_calibration_manager.get_calibrated_confidence(raw_conf, call_site), 4)
            ans["raw_confidence"] = raw_conf
            ans["confidence"] = calibrated_conf

    # Log for calibration and auditing (field lengths capped; hostile payloads
    # kept to a short snippet, never verbatim).
    session_id = _truncate((req.metadata or {}).get("session_id", "unknown"), LOG_SESSION_ID_CHARS)
    call_site = _truncate(call_site, LOG_CALL_SITE_CHARS)
    state_snippet = state_str[:LOG_STATE_SNIPPET_CHARS]
    ground_truth = req.ground_truth or {}

    for qid, ans in answers.items():
        q_def = req.questions.get(qid, {})
        log_record = {
            "timestamp": time.time(),
            "session_id": session_id,
            "call_site": call_site,
            "question_id": _truncate(qid, LOG_SESSION_ID_CHARS),
            "question_type": _truncate(q_def.get("type", "unknown"), 32),
            "instructions": _truncate(q_def.get("instructions", ""), LOG_INSTRUCTIONS_CHARS),
            "state_snippet": state_snippet,
            "answer": ans.get("answer") or ans.get("action") or ans.get("noul") or ans.get("score"),
            "confidence": ans.get("confidence", 0.0),
            "latency_ms": latency_ms,
            "ground_truth": ground_truth.get(qid),
        }
        log_decision_record(log_record)

    return DecideResponse(
        answers=answers,
        usage=usage,
        latency_ms=round(latency_ms, 2),
        model=MODEL_ID,
        non_english=False,
    )


def start_server(host: str = DEFAULT_HOST, port: int = DEFAULT_PORT):
    """Start uvicorn server binding strictly to 127.0.0.1 with resilient Windows socket error handling."""
    import asyncio
    def exception_handler(loop, context):
        exception = context.get("exception")
        if isinstance(exception, OSError) and getattr(exception, "winerror", None) in (64, 10054):
            return  # Suppress client connection aborted/reset on Windows
        loop.default_exception_handler(context)

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    loop.set_exception_handler(exception_handler)

    config = uvicorn.Config(app, host=host, port=port, log_level="info", loop="asyncio")
    server = uvicorn.Server(config)
    loop.run_until_complete(server.serve())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Laya decision sidecar (loopback microservice)")
    parser.add_argument("--host", default=None, help="Bind host (default: LAYA_HOST or 127.0.0.1)")
    parser.add_argument("--port", type=int, default=None, help="Bind port (default: LAYA_PORT or 8177)")
    parser.add_argument("host_pos", nargs="?", help="Positional bind host (back-compat)")
    parser.add_argument("port_pos", type=int, nargs="?", help="Positional bind port (back-compat)")
    args = parser.parse_args()
    host = args.host or args.host_pos or DEFAULT_HOST
    port = args.port or args.port_pos or DEFAULT_PORT
    if host not in ("127.0.0.1", "localhost", "::1"):
        logger.warning(f"Binding non-loopback host {host!r}; /v1/decide requires the auth token. Prefer loopback.")
    start_server(host, port)
