"""Bound admission and retain inference capacity until synchronous work finishes.

Killable-worker spike (P0-2 / Stream-E) decision: production stays on
``asyncio.to_thread`` (``LAYA_WORKER_MODEL=thread``). A ``multiprocessing``
worker would make a stuck torch op killable, but the Laya agent (model,
tokenizer, CUDA/CPU pools) is not safely fork/pickle-able per request: each
child would need a full checkpoint reload (~421M params), shared-tokenizer
races, and a new IPC/timeout protocol — all failure modes that risk the
fail-open contract. So ``worker_model="process"`` is rejected at construction
with an explanatory error; stuck slots are instead made observable via
:py:meth:`InferenceScheduler.snapshot` (``oldest_in_flight_ms`` + ``stuck``)
and the restart procedure in ``README.md``. The timeout-retains-slot tests in
``test_inference_scheduler.py`` are the standing proof that a stuck thread
still holds its slot.
"""

from __future__ import annotations

import asyncio
import os
import time
from collections.abc import Awaitable, Callable
from typing import TypeVar

T = TypeVar("T")

#: Default age (seconds) after which an in-flight slot counts as stuck in
#: :py:meth:`InferenceScheduler.snapshot`. Overridable per scheduler or via
#: ``LAYA_STUCK_THRESHOLD_S``.
DEFAULT_STUCK_THRESHOLD_S = 120.0

#: Only supported worker backend. ``LAYA_WORKER_MODEL=process`` is accepted as
#: a spelling but rejected at construction (see module docstring); the env var
#: exists so a future spike can gate a real process pool without renaming.
THREAD_WORKER_MODEL = "thread"


def _resolve_stuck_threshold(explicit: float | None) -> float:
    if explicit is not None:
        return float(explicit)
    raw = os.getenv("LAYA_STUCK_THRESHOLD_S")
    if raw is None or not raw.strip():
        return DEFAULT_STUCK_THRESHOLD_S
    try:
        value = float(raw)
    except ValueError:
        return DEFAULT_STUCK_THRESHOLD_S
    return value if value > 0 else DEFAULT_STUCK_THRESHOLD_S


def _resolve_worker_model(explicit: str | None) -> str:
    if explicit is not None:
        return explicit
    return os.getenv("LAYA_WORKER_MODEL", THREAD_WORKER_MODEL).strip() or THREAD_WORKER_MODEL


class InferenceBusyError(Exception):
    """The bounded waiting queue is full."""


class InferenceScheduler:
    def __init__(
        self,
        concurrency: int,
        max_waiting: int = 8,
        stuck_threshold_s: float | None = None,
        worker_model: str | None = None,
    ):
        if concurrency < 1:
            raise ValueError("concurrency must be >= 1")
        model = _resolve_worker_model(worker_model)
        if model != THREAD_WORKER_MODEL:
            raise RuntimeError(
                f"LAYA_WORKER_MODEL={model!r} is not supported: the killable-process "
                "spike was evaluated and rejected (Laya agent/model state is not "
                "safely fork/pickle-able per request; each child would need a full "
                "checkpoint reload and a new IPC protocol, risking the fail-open "
                "contract). Production stays on 'thread'; detect stuck slots via "
                "snapshot()['stuck'] and restart the sidecar per README.md."
            )
        self._slots = asyncio.Semaphore(concurrency)
        self._waiting = 0
        self._max_waiting = max_waiting
        self._active = 0
        self._admitted = 0
        self._completed = 0
        self._timed_out = 0
        self._cancelled = 0
        self._busy_rejected = 0
        self._queue_wait_ms_total = 0.0
        self._inference_ms_total = 0.0
        self._worker_model = model
        self._stuck_threshold_s = _resolve_stuck_threshold(stuck_threshold_s)
        # Monotonic start times (time.monotonic()) of currently in-flight
        # workers. Mutated only on the event-loop thread (run body + done
        # callback), so no lock is needed. time.monotonic() keeps snapshot()
        # callable without a running loop (e.g. /health always can report).
        self._in_flight_since: list[float] = []

    def snapshot(self) -> dict:
        """Point-in-time capacity and outcome counters for diagnostics."""
        if self._in_flight_since:
            oldest = min(self._in_flight_since)
            oldest_age_ms = round((time.monotonic() - oldest) * 1000, 1)
            stuck = oldest_age_ms > self._stuck_threshold_s * 1000
        else:
            oldest_age_ms = None
            stuck = False
        return {
            "waiting": self._waiting,
            "in_flight": self._active,
            "admitted": self._admitted,
            "completed": self._completed,
            "timed_out": self._timed_out,
            "cancelled": self._cancelled,
            "busy_rejected": self._busy_rejected,
            "queue_wait_ms_total": round(self._queue_wait_ms_total, 1),
            "inference_ms_total": round(self._inference_ms_total, 1),
            "oldest_in_flight_ms": oldest_age_ms,
            "stuck": stuck,
            "stuck_threshold_s": self._stuck_threshold_s,
            "worker_model": self._worker_model,
            "restart_advisory": (
                "Inference slot stuck beyond threshold; restart the sidecar "
                "(see README.md) — stuck torch work cannot be cancelled in-process."
                if stuck
                else None
            ),
        }

    async def run(
        self,
        predict: Callable[[], T],
        timeout_s: float,
        before_start: Callable[[], Awaitable[None]] | None = None,
    ) -> T:
        if self._waiting >= self._max_waiting:
            self._busy_rejected += 1
            raise InferenceBusyError()
        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout_s
        queued_at = loop.time()
        self._waiting += 1
        try:
            await asyncio.wait_for(self._slots.acquire(), timeout=timeout_s)
        except asyncio.TimeoutError:
            self._timed_out += 1
            raise
        except asyncio.CancelledError:
            self._cancelled += 1
            raise
        finally:
            self._waiting -= 1
            self._queue_wait_ms_total += (loop.time() - queued_at) * 1000
        try:
            if before_start is not None:
                await asyncio.wait_for(
                    before_start(), timeout=max(0, deadline - loop.time())
                )
        except BaseException:
            self._slots.release()
            raise
        remaining = deadline - loop.time()
        if remaining <= 0:
            self._slots.release()
            self._timed_out += 1
            raise TimeoutError()

        # Cancelling asyncio.to_thread cannot stop a running torch operation.
        # Only its completion callback releases capacity, even after the HTTP
        # waiter has timed out or disconnected. Retrieve late exceptions too.
        # Stuck-slot observability: record the monotonic start so snapshot()
        # can report oldest_in_flight_ms / stuck without a running loop.
        worker = asyncio.create_task(asyncio.to_thread(predict))
        started_at = time.monotonic()
        self._active += 1
        self._admitted += 1
        self._in_flight_since.append(started_at)

        def completed(task: asyncio.Task[T]) -> None:
            self._slots.release()
            self._active -= 1
            try:
                self._in_flight_since.remove(started_at)
            except ValueError:
                pass
            self._inference_ms_total += (time.monotonic() - started_at) * 1000
            if task.cancelled():
                self._cancelled += 1
            else:
                task.exception()
                self._completed += 1

        worker.add_done_callback(completed)
        try:
            return await asyncio.wait_for(asyncio.shield(worker), timeout=remaining)
        except asyncio.TimeoutError:
            self._timed_out += 1
            raise
