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

    async def test_snapshot_records_outcomes_and_capacity(self):
        scheduler = InferenceScheduler(1, max_waiting=1)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(2)

        active = asyncio.create_task(scheduler.run(blocked, 5))
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            mid = scheduler.snapshot()
            self.assertEqual(mid["in_flight"], 1)
            self.assertEqual(mid["admitted"], 1)
            queued = asyncio.create_task(scheduler.run(lambda: "queued", 5))
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
        self.assertEqual(await scheduler.run(lambda: "ok", 1), "ok")
        snap = scheduler.snapshot()
        self.assertEqual(snap["in_flight"], 0)
        self.assertEqual(snap["completed"], 2)
        self.assertEqual(snap["busy_rejected"], 1)
        self.assertGreaterEqual(snap["inference_ms_total"], 0)
        self.assertGreaterEqual(snap["queue_wait_ms_total"], 0)

    async def test_timeout_counts_without_freeing_the_worker_slot(self):
        scheduler = InferenceScheduler(1)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(2)

        active = asyncio.create_task(scheduler.run(blocked, 5))
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            effects = []
            with self.assertRaises(TimeoutError):
                await scheduler.run(lambda: effects.append("ran"), 0.03)
            self.assertEqual(scheduler.snapshot()["timed_out"], 1)
            self.assertEqual(scheduler.snapshot()["in_flight"], 1)
            self.assertEqual(effects, [])
        finally:
            release.set()
            await active
        self.assertEqual(scheduler.snapshot()["in_flight"], 0)

    async def test_snapshot_idle_reports_no_stuck_slot(self):
        scheduler = InferenceScheduler(1, stuck_threshold_s=0.05)
        snap = scheduler.snapshot()
        self.assertEqual(snap["in_flight"], 0)
        self.assertIsNone(snap["oldest_in_flight_ms"])
        self.assertFalse(snap["stuck"])
        self.assertIsNone(snap["restart_advisory"])
        self.assertEqual(snap["worker_model"], "thread")

    async def test_snapshot_exposes_stuck_slot_age_and_restart_advisory(self):
        scheduler = InferenceScheduler(1, stuck_threshold_s=0.05)
        entered = threading.Event()
        release = threading.Event()

        def blocked():
            entered.set()
            release.wait(5)

        active = asyncio.create_task(scheduler.run(blocked, 5))
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            early = scheduler.snapshot()
            self.assertEqual(early["in_flight"], 1)
            self.assertIsNotNone(early["oldest_in_flight_ms"])
            self.assertGreaterEqual(early["oldest_in_flight_ms"], 0)
            await asyncio.sleep(0.15)
            stuck = scheduler.snapshot()
            self.assertEqual(stuck["in_flight"], 1)
            self.assertTrue(stuck["stuck"])
            self.assertGreater(stuck["oldest_in_flight_ms"], 50)
            self.assertIsNotNone(stuck["restart_advisory"])
        finally:
            release.set()
            await active
        recovered = scheduler.snapshot()
        self.assertEqual(recovered["in_flight"], 0)
        self.assertIsNone(recovered["oldest_in_flight_ms"])
        self.assertFalse(recovered["stuck"])

    async def test_permanently_stuck_thread_holds_slot_spike_proof(self):
        """Spike proof: a never-finishing thread keeps its slot; only the
        stuck flag + restart advisory signal it (no silent oversubscribe)."""
        scheduler = InferenceScheduler(1, stuck_threshold_s=0.05)
        entered = threading.Event()

        def never_returns():
            entered.set()
            threading.Event().wait(30)

        leaked = asyncio.create_task(scheduler.run(never_returns, 5))
        try:
            await asyncio.wait_for(asyncio.to_thread(entered.wait), 1)
            await asyncio.sleep(0.15)
            mid = scheduler.snapshot()
            self.assertEqual(mid["in_flight"], 1)
            self.assertTrue(mid["stuck"])
            effects = []
            with self.assertRaises(TimeoutError):
                await scheduler.run(lambda: effects.append("ran"), 0.03)
            self.assertEqual(effects, [])
            self.assertEqual(scheduler.snapshot()["in_flight"], 1)
        finally:
            leaked.cancel()
            try:
                await leaked
            except (asyncio.CancelledError, TimeoutError):
                pass
            # Slot is still held: the orphan thread cannot be killed
            # in-process; a sidecar restart is the documented recovery.
            self.assertEqual(scheduler.snapshot()["in_flight"], 1)

    def test_process_worker_model_rejected_with_spike_rationale(self):
        with self.assertRaisesRegex(RuntimeError, "killable-process spike"):
            InferenceScheduler(1, worker_model="process")


if __name__ == "__main__":
    unittest.main()
