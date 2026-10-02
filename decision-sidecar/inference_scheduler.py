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

    async def run(
        self,
        predict: Callable[[], T],
        timeout_s: float,
        before_start: Callable[[], Awaitable[None]] | None = None,
    ) -> T:
        if self._waiting >= self._max_waiting:
            raise InferenceBusyError()
        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout_s
        self._waiting += 1
        try:
            await asyncio.wait_for(self._slots.acquire(), timeout=timeout_s)
        finally:
            self._waiting -= 1
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
            raise TimeoutError()

        # Cancelling asyncio.to_thread cannot stop a running torch operation.
        # Only its completion callback releases capacity, even after the HTTP
        # waiter has timed out or disconnected. Retrieve late exceptions too.
        worker = asyncio.create_task(asyncio.to_thread(predict))

        def completed(task: asyncio.Task[T]) -> None:
            self._slots.release()
            if not task.cancelled():
                task.exception()

        worker.add_done_callback(completed)
        return await asyncio.wait_for(asyncio.shield(worker), timeout=remaining)
