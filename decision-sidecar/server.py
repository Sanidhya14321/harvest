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

import laya

if sys.platform == "win32":
    # Prevent IOCP WinError 64 (ERROR_NETNAME_DELETED) on abrupt client disconnections
    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())

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

# Module-level singleton
_agent: Optional[laya.agent.Agent] = None
_model_ready: bool = False
_device: str = "cpu"


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
    logger.info(f"Loading single-model checkpoint: {MODEL_ID} on {_device}...")
    start_time = time.perf_counter()
    try:
        # Strictly load the single checkpoint, avoiding Router which pulls laya-multilingual
        _agent = laya.load(MODEL_ID, device=_device)
        _model_ready = True
        elapsed = (time.perf_counter() - start_time) * 1000
        logger.info(f"Loaded {MODEL_ID} successfully in {elapsed:.1f}ms")
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
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
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
    """Health check endpoint: ready/not-ready status."""
    return {
        "status": "ok" if _model_ready else "loading",
        "ready": _model_ready,
        "model": MODEL_ID,
        "device": _device,
        "port": DEFAULT_PORT,
    }


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
    """Start uvicorn server binding strictly to 127.0.0.1."""
    uvicorn.run(app, host=host, port=port, log_level="info")


if __name__ == "__main__":
    host = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_HOST
    port = int(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_PORT
    start_server(host, port)
