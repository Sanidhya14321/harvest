"""Bound admission and retain inference capacity until synchronous work finishes."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import TypeVar

T = TypeVar("T")


class InferenceBusyError(Exception):
    """The bounded waiting queue is full."""


class InferenceScheduler:
    def __init__(self, concurrency: int, max_waiting: int = 8):
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

    def snapshot(self) -> dict:
        """Point-in-time capacity and outcome counters for diagnostics."""
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
        worker = asyncio.create_task(asyncio.to_thread(predict))
        started_at = loop.time()
        self._active += 1
        self._admitted += 1

        def completed(task: asyncio.Task[T]) -> None:
            self._slots.release()
            self._active -= 1
            self._inference_ms_total += (loop.time() - started_at) * 1000
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
