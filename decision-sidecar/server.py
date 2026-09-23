"""Laya Local Decision Sidecar Service.

Provides fast, typed decision endpoints powered by convaiinnovations/laya-typed-decisions.
Runs as a local HTTP service on 127.0.0.1:8177.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
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
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = int(os.getenv("LAYA_PORT", "8177"))
LOG_FILE_PATH = Path(os.getenv("LAYA_LOG_FILE", Path(__file__).parent / "decisions.jsonl"))

# Module-level singleton and hardware state
_hardware_info: Dict[str, Any] = detect_hardware()
_agent: Optional[laya.agent.Agent] = None
_model_ready: bool = False
_device: str = _hardware_info.get("device", "cpu")


def get_agent() -> laya.agent.Agent:
    """Return the loaded Laya agent singleton."""
    global _agent
    if _agent is None:
        raise RuntimeError("Laya agent model has not been initialized yet.")
    return _agent


def log_decision_record(record: dict[str, Any]) -> None:
    """Append structured decision log for calibration and observability."""
    try:
        LOG_FILE_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(LOG_FILE_PATH, "a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
    except Exception as e:
        logger.warning(f"Failed to write decision log: {e}")


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
            torch.set_num_threads(num_cores)
            logger.info(f"Configured PyTorch CPU thread pool: {torch.get_num_threads()} threads")

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

app.add_middleware(
    CORSMiddleware,
    allow_origins=[
        "http://127.0.0.1",
        "http://127.0.0.1:8177",
        "http://localhost",
        "http://localhost:8177",
    ],
    allow_credentials=False,
    allow_methods=["GET", "POST", "OPTIONS"],
    allow_headers=["Content-Type"],
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


@app.post("/v1/decide", response_model=DecideResponse)
async def decide(req: DecideRequest) -> DecideResponse:
    """Evaluate typed decision questions against state in a single parallel pass."""
    if not _model_ready or _agent is None:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Model is still loading or unavailable",
        )

    state = req.state
    # Check English language requirement
    # ModernBERT-large English checkpoint will hallucinate or degrade on non-English text.
    try:
        if not laya.is_english(state):
            logger.info("Non-English state detected; rejecting for main-LLM fallback")
            return DecideResponse(
                answers={},
                latency_ms=0.0,
                model=MODEL_ID,
                non_english=True,
            )
    except Exception as e:
        logger.debug(f"Language detection check failed: {e}")

    start_time = time.perf_counter()
    try:
        # Check if state is a dictionary providing per-question states (for batched multi-chunk scoring)
        ids = list(req.questions.keys())
        per_question = isinstance(state, dict) and any(qid in state for qid in ids)

        if per_question:
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

            items = []
            max_len = _agent.cfg.get("max_len", 1024)
            head_max_len = _agent.cfg.get("head_max_len", 256)

            for qid in ids:
                q = _agent._to_internal(req.questions[qid])
                q_state = state.get(qid, state)
                seq, markers = build_sequence(_agent.tok, q_state, q, max_len, head_max_len)
                if len(markers) != len(render_options(q)):
                    raise ValueError(f"question {qid!r} options exceed head_max_len={head_max_len}")
                items.append({"ids": seq, "markers": markers, "qtype": QTYPES[q["t"]]})

            b = collate_items([items], _agent.tok.pad_token_id)
            use_amp = _agent.device.type == "cuda"
            with torch.no_grad():
                with torch.autocast(device_type=_agent.device.type, dtype=_agent.dtype, enabled=use_amp):
                    logits, act = _agent.model(
                        b["input_ids"].to(_agent.device),
                        b["attention_mask"].to(_agent.device),
                        b["marker_pos"].to(_agent.device),
                        b["marker_mask"].to(_agent.device),
                        b["qtype"].to(_agent.device),
                    )

                logits = logits.detach().float().cpu().numpy()
                act = torch.softmax(act.detach().float(), -1).cpu().numpy()
            answers = {}
            n_tokens = int(b["attention_mask"].sum())

            for r, qid in enumerate(ids):
                q = _agent._to_internal(req.questions[qid])
                k = len(items[r]["markers"])
                qt = QTYPES[q["t"]]
                t_scale = _agent.temperature_by_options.get(temp_bucket(qt, k), _agent.temperature[qt])
                z = logits[r, :k] / t_scale
                p = np.exp(z - z.max())
                p = p / p.sum()

                conf_score = round(confidence_from_probs(p, k), 4)
                ext = {"act_probability": round(float(act[r, 0]), 4)}

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

            result = {
                "model": "laya-rl-agent",
                "answers": answers,
                "usage": {"input_tokens": n_tokens, "output_tokens": 0},
            }
        else:
            # Run prediction on the singleton agent
            result = _agent.predict(state=state, questions=req.questions)
    except Exception as e:
        logger.error(f"Inference error during predict: {e}", exc_info=True)
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Laya inference error: {str(e)}",
        )

    latency_ms = (time.perf_counter() - start_time) * 1000
    answers = result.get("answers", {})
    usage = result.get("usage", {})

    # Log for calibration and auditing
    call_site = (req.metadata or {}).get("call_site", "unknown")
    session_id = (req.metadata or {}).get("session_id", "unknown")
    state_snippet = str(state)[:200]

    for qid, ans in answers.items():
        q_def = req.questions.get(qid, {})
        log_record = {
            "timestamp": time.time(),
            "session_id": session_id,
            "call_site": call_site,
            "question_id": qid,
            "question_type": q_def.get("type", "unknown"),
            "instructions": q_def.get("instructions", ""),
            "state_snippet": state_snippet,
            "answer": ans.get("answer") or ans.get("action") or ans.get("noul") or ans.get("score"),
            "confidence": ans.get("confidence", 0.0),
            "latency_ms": latency_ms,
            "ground_truth": None,
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
    host = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_HOST
    port = int(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_PORT
    start_server(host, port)
