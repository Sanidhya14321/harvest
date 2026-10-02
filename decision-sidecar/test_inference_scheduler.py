"""Inference capacity and admission contracts without torch or checkpoint downloads."""

import asyncio
import threading
import unittest

from inference_scheduler import InferenceBusyError, InferenceScheduler


class InferenceSchedulerTests(unittest.IsolatedAsyncioTestCase):
    async def test_disconnected_caller_does_not_dispatch_and_capacity_recovers(self):
        scheduler = InferenceScheduler(1)
        effects = []

        async def disconnected():
            raise ConnectionError("disconnected")

        with self.assertRaises(ConnectionError):
            await scheduler.run(
                lambda: effects.append("ran"), 1, before_start=disconnected
            )
        self.assertEqual(effects, [])
        self.assertEqual(await scheduler.run(lambda: "recovered", 1), "recovered")

    async def test_timeout_retains_slot_until_worker_finishes(self):
        scheduler = InferenceScheduler(1)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(2)

        try:
            with self.assertRaises(TimeoutError):
                await scheduler.run(blocked, 0.05)
            self.assertTrue(entered.is_set())
            effects = []
            with self.assertRaises(TimeoutError):
                await scheduler.run(lambda: effects.append("ran"), 0.03)
            self.assertEqual(effects, [])
        finally:
            release.set()
        self.assertEqual(await scheduler.run(lambda: "recovered", 1), "recovered")

    async def test_cancelled_waiter_does_not_release_active_inference(self):
        scheduler = InferenceScheduler(1)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(2)

        task = asyncio.create_task(scheduler.run(blocked, 1))
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
            effects = []
            with self.assertRaises(TimeoutError):
                await scheduler.run(lambda: effects.append("ran"), 0.03)
            self.assertEqual(effects, [])
        finally:
            release.set()
        self.assertEqual(await scheduler.run(lambda: "recovered", 1), "recovered")

    async def test_full_waiting_queue_rejects_without_dispatch(self):
        scheduler = InferenceScheduler(1, max_waiting=1)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(2)

        active = asyncio.create_task(scheduler.run(blocked, 1))
        queued = None
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            queued = asyncio.create_task(scheduler.run(lambda: "queued", 0.5))
            await asyncio.sleep(0)
            effects = []
            with self.assertRaises(InferenceBusyError):
                await scheduler.run(lambda: effects.append("ran"), 1)
            self.assertEqual(effects, [])
            queued.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await queued
        finally:
            release.set()
            await active
        self.assertEqual(await scheduler.run(lambda: "recovered", 1), "recovered")


if __name__ == "__main__":
    unittest.main()
