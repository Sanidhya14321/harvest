"""Tests for decision-sidecar hardening: auth, caps, language gate, log bounds,
calibration provenance, and inference timeout.

Dependency-light: no model checkpoint is loaded. Inference is exercised with a
fake agent; model-backed equivalence lives in test_bucketing.py behind
RUN_LAYA_BENCHMARK.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

try:
    import calibration
    import server
except ImportError:
    from . import calibration, server

from fastapi import HTTPException


def _make_request(headers=None):
    from starlette.requests import Request

    raw = [(k.lower().encode(), v.encode()) for k, v in (headers or {}).items()]
    async def receive():
        return {"type": "http.request", "body": b"", "more_body": False}

    return Request({"type": "http", "method": "POST", "path": "/v1/decide", "headers": raw}, receive)


def _load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _decide_body(**overrides):
    body = {
        "state": "the bash tool ran ls on the project directory",
        "questions": {
            "q1": {"type": "noul", "instructions": "does this call write or delete irreversibly?"},
        },
        "metadata": {"call_site": "tool_gating", "session_id": "sess-1"},
    }
    body.update(overrides)
    return server.DecideRequest(**body)


class _FakeAgent:
    """Minimal stand-in for laya.agent.Agent single-question predict path."""

    def __init__(self, sleep_s=0.0):
        self.calls = []
        self.sleep_s = sleep_s

    def predict(self, state=None, questions=None):
        if self.sleep_s:
            time.sleep(self.sleep_s)
        self.calls.append((state, questions))
        return {
            "answers": {
                "q1": {
                    "type": "noul",
                    "noul": 0.9,
                    "confidence": 0.9,
                    "action": {"act_probability": 0.9},
                }
            },
            "usage": {"input_tokens": 1, "output_tokens": 0},
        }


class ServerGlobalsMixin:
    """Save/restore mutated server globals and env around each test."""

    def setUp(self):
        super().setUp()
        self._saved = {
            "model_ready": server._model_ready,
            "agent": server._agent,
            "auth_token": server._auth_token,
            "log_path": server.LOG_FILE_PATH,
            "log_max": server.LOG_MAX_BYTES,
            "log_backups": server.LOG_BACKUP_COUNT,
            "timeout": server.INFERENCE_TIMEOUT_S,
            "scheduler": server._inference_scheduler,
        }
        server._inference_scheduler = None
        self._tmpdir = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmpdir.cleanup)
        env_patch = mock.patch.dict(os.environ, {"LAYA_DISABLE_AUTH": "1"})
        env_patch.start()
        self.addCleanup(env_patch.stop)

    def tearDown(self):
        server._model_ready = self._saved["model_ready"]
        server._agent = self._saved["agent"]
        server._auth_token = self._saved["auth_token"]
        server.LOG_FILE_PATH = self._saved["log_path"]
        server.LOG_MAX_BYTES = self._saved["log_max"]
        server.LOG_BACKUP_COUNT = self._saved["log_backups"]
        server.INFERENCE_TIMEOUT_S = self._saved["timeout"]
        server._inference_scheduler = self._saved["scheduler"]
        super().tearDown()


class TestSidecarAuth(unittest.TestCase):
    def setUp(self):
        super().setUp()
        self._saved_token = server._auth_token
        self.addCleanup(setattr, server, "_auth_token", self._saved_token)
        server._auth_token = "test-secret"

    def test_missing_token_rejected(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("LAYA_DISABLE_AUTH", None)
            with self.assertRaises(HTTPException) as ctx:
                server.verify_request_auth(_make_request())
        self.assertEqual(ctx.exception.status_code, 401)

    def test_wrong_token_rejected(self):
        os.environ.pop("LAYA_DISABLE_AUTH", None)
        with self.assertRaises(HTTPException) as ctx:
            server.verify_request_auth(_make_request({"x-laya-token": "wrong"}))
        self.assertEqual(ctx.exception.status_code, 401)

    def test_header_token_accepted(self):
        os.environ.pop("LAYA_DISABLE_AUTH", None)
        server.verify_request_auth(_make_request({"x-laya-token": "test-secret"}))

    def test_bearer_token_accepted(self):
        os.environ.pop("LAYA_DISABLE_AUTH", None)
        server.verify_request_auth(_make_request({"authorization": "Bearer test-secret"}))

    def test_auth_disabled_allows_anonymous(self):
        with mock.patch.dict(os.environ, {"LAYA_DISABLE_AUTH": "1"}):
            server.verify_request_auth(_make_request())

    def test_env_token_used_without_file_write(self):
        server._auth_token = None
        with mock.patch.dict(os.environ, {"LAYA_TOKEN": "env-secret", "LAYA_TOKEN_FILE": os.path.join("nonexistent-dir-xyz", "tok")}):
            self.assertEqual(server.get_auth_token(), "env-secret")

    def test_generated_token_persisted(self):
        import shutil

        server._auth_token = None
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmpdir, True)
        token_file = os.path.join(tmpdir, "laya-token")
        with mock.patch.dict(os.environ, {"LAYA_TOKEN_FILE": token_file}):
            os.environ.pop("LAYA_TOKEN", None)
            token = server.get_auth_token()
        self.assertTrue(len(token) >= 32)
        with open(token_file, encoding="utf-8") as f:
            self.assertEqual(f.read(), token)


class TestRequestCaps(ServerGlobalsMixin, unittest.TestCase):
    def test_too_many_questions_rejected(self):
        req = _decide_body(questions={f"q{i}": {"type": "noul", "instructions": "x"} for i in range(65)})
        with self.assertRaises(HTTPException) as ctx:
            server._validate_request(req)
        self.assertEqual(ctx.exception.status_code, 400)

    def test_oversize_state_rejected(self):
        req = _decide_body(state="x" * (server.MAX_STATE_CHARS + 1))
        with self.assertRaises(HTTPException) as ctx:
            server._validate_request(req)
        self.assertEqual(ctx.exception.status_code, 413)

    def test_per_question_chunk_cap(self):
        big = "y" * (server.MAX_QUESTION_STATE_CHARS + 1)
        req = _decide_body(
            state={"q1": "small", "q2": big},
            questions={
                "q1": {"type": "noul", "instructions": "a"},
                "q2": {"type": "noul", "instructions": "b"},
            },
        )
        with self.assertRaises(HTTPException) as ctx:
            server._validate_request(req)
        self.assertEqual(ctx.exception.status_code, 413)

    def test_estimated_token_cap_before_tokenization(self):
        # Shared state under the char cap but 64 questions push the estimate over.
        req = _decide_body(
            state="y" * 220_000,
            questions={f"q{i}": {"type": "noul", "instructions": "a"} for i in range(64)},
        )
        est = server._estimate_input_tokens(req.state, list(req.questions.keys()))
        self.assertGreater(est, server.MAX_EST_INPUT_TOKENS)
        with self.assertRaises(HTTPException) as ctx:
            server._validate_request(req)
        self.assertEqual(ctx.exception.status_code, 413)

    def test_oversize_instructions_rejected(self):
        req = _decide_body(
            questions={"q1": {"type": "noul", "instructions": "z" * (server.MAX_INSTRUCTIONS_CHARS + 1)}}
        )
        with self.assertRaises(HTTPException) as ctx:
            server._validate_request(req)
        self.assertEqual(ctx.exception.status_code, 400)

    def test_end_to_end_cap_returns_400_not_500(self):
        server._model_ready = True
        server._agent = _FakeAgent()
        req = _decide_body(questions={f"q{i}": {"type": "noul", "instructions": "x"} for i in range(65)})
        with self.assertRaises(HTTPException) as ctx:
            asyncio.run(server.decide(req, _make_request()))
        self.assertEqual(ctx.exception.status_code, 400)
        self.assertEqual(server._agent.calls, [])


class TestEnglishGate(unittest.TestCase):
    def test_per_chunk_detection(self):
        state = {"q1": "plain english project status update", "q2": "FRENCH MARQUEUR contenu"}
        with mock.patch("laya.is_english", side_effect=lambda c: "FRENCH" not in c):
            self.assertFalse(server._check_english_gate(state, ["q1", "q2"], "model_routing"))
            self.assertTrue(server._check_english_gate({"q1": "plain english"}, ["q1"], "model_routing"))

    def test_detection_exception_fails_closed_for_gating(self):
        with mock.patch("laya.is_english", side_effect=RuntimeError("boom")):
            with self.assertRaises(HTTPException) as ctx:
                server._check_english_gate("anything", ["q1"], "tool_gating")
            self.assertEqual(ctx.exception.status_code, 503)

    def test_detection_exception_fails_open_otherwise(self):
        with mock.patch("laya.is_english", side_effect=RuntimeError("boom")):
            self.assertTrue(server._check_english_gate("anything", ["q1"], "model_routing"))

    def test_non_english_short_circuits_before_inference(self):
        agent = _FakeAgent()
        server._model_ready = True
        server._agent = agent
        saved = (server._model_ready, server._agent)
        try:
            with mock.patch.dict(os.environ, {"LAYA_DISABLE_AUTH": "1"}), mock.patch(
                "laya.is_english", return_value=False
            ):
                resp = asyncio.run(server.decide(_decide_body(), _make_request()))
            self.assertTrue(resp.non_english)
            self.assertEqual(resp.answers, {})
            self.assertEqual(agent.calls, [])
        finally:
            server._model_ready, server._agent = saved[0], saved[1]


class TestDecideLogging(ServerGlobalsMixin, unittest.TestCase):
    def _run_decide(self, **overrides):
        from pathlib import Path

        server.LOG_FILE_PATH = Path(self._tmpdir.name) / "decisions.jsonl"
        server._model_ready = True
        server._agent = _FakeAgent()
        metadata = overrides.pop("metadata", {"call_site": "tool_gating", "session_id": "sess-1"})
        req = _decide_body(metadata=metadata, **overrides)
        resp = asyncio.run(server.decide(req, _make_request()))
        with open(server.LOG_FILE_PATH, encoding="utf-8") as f:
            records = [json.loads(line) for line in f if line.strip()]
        return resp, records

    def test_logged_fields_capped(self):
        _, records = self._run_decide(
            state="s" * 1000,
            metadata={"call_site": "tool_gating", "session_id": "x" * 200},
            questions={"q1": {"type": "noul", "instructions": "i" * 1000}},
        )
        self.assertEqual(len(records), 1)
        rec = records[0]
        self.assertEqual(len(rec["state_snippet"]), server.LOG_STATE_SNIPPET_CHARS)
        self.assertLessEqual(len(rec["session_id"]), server.LOG_SESSION_ID_CHARS)
        self.assertLessEqual(len(rec["instructions"]), server.LOG_INSTRUCTIONS_CHARS)
        self.assertNotIn("s" * 1000, json.dumps(rec))

    def test_ground_truth_logged_when_provided(self):
        _, records = self._run_decide(ground_truth={"q1": 1})
        self.assertEqual(records[0]["ground_truth"], 1)

    def test_ground_truth_defaults_null(self):
        _, records = self._run_decide()
        self.assertIsNone(records[0]["ground_truth"])

    def test_log_rotation_bounds_disk(self):
        from pathlib import Path

        server.LOG_FILE_PATH = Path(self._tmpdir.name) / "decisions.jsonl"
        server.LOG_MAX_BYTES = 200
        server.LOG_BACKUP_COUNT = 2
        for i in range(10):
            server.log_decision_record({"i": i, "pad": "x" * 100})
        current_size = server.LOG_FILE_PATH.stat().st_size
        self.assertLess(current_size, 200 + 256)
        backups = [Path(str(server.LOG_FILE_PATH) + f".{i}") for i in (1, 2)]
        self.assertTrue(any(p.exists() for p in backups))
        self.assertFalse(Path(str(server.LOG_FILE_PATH) + ".3").exists())


class TestInferenceTimeout(ServerGlobalsMixin, unittest.TestCase):
    def test_slow_inference_returns_504(self):
        server._model_ready = True
        server._agent = _FakeAgent(sleep_s=0.5)
        server.INFERENCE_TIMEOUT_S = 0.05
        with self.assertRaises(HTTPException) as ctx:
            asyncio.run(server.decide(_decide_body(), _make_request()))
        self.assertEqual(ctx.exception.status_code, 504)

    def test_model_not_ready_returns_503(self):
        server._model_ready = False
        server._agent = None
        with self.assertRaises(HTTPException) as ctx:
            asyncio.run(server.decide(_decide_body(), _make_request()))
        self.assertEqual(ctx.exception.status_code, 503)


class TestCalibrationProvenance(unittest.TestCase):
    def _records(self, n=12, site="tool_gating"):
        return [
            {"call_site": site, "confidence": 0.6 + 0.03 * (i % 10), "ground_truth": i % 2}
            for i in range(n)
        ]

    def test_provenance_stamped_real(self):
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(tmpdir, ignore_errors=True))
        path = os.path.join(tmpdir, "params.json")
        mgr = calibration.CalibrationManager(params_path=__import__("pathlib").Path(path))
        mgr.calibrate_from_records(self._records(), synthetic=False)
        saved = _load_json(path)
        self.assertFalse(saved["_provenance"]["synthetic"])
        self.assertEqual(saved["_provenance"]["sample_counts"], {"tool_gating": 12})

    def test_save_keeps_backup(self):
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(tmpdir, ignore_errors=True))
        path = os.path.join(tmpdir, "params.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump({"old": True}, f)
        mgr = calibration.CalibrationManager(params_path=__import__("pathlib").Path(path))
        mgr.params = {"new": True}
        mgr.save()
        with open(path + ".bak", encoding="utf-8") as f:
            self.assertEqual(json.load(f), {"old": True})

    def test_synthetic_defaults_to_separate_path(self):
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(tmpdir, ignore_errors=True))
        real_path = os.path.join(tmpdir, "calibration_params.json")
        synth_path = os.path.join(tmpdir, "calibration_params.synthetic.json")
        with open(real_path, "w", encoding="utf-8") as f:
            json.dump({"real": True}, f)
        from pathlib import Path

        with (
            mock.patch.object(calibration, "CALIBRATION_PARAMS_PATH", Path(real_path)),
            mock.patch.object(calibration, "CALIBRATION_SYNTHETIC_PATH", Path(synth_path)),
            mock.patch.object(sys, "argv", ["calibration.py", "--synthetic"]),
        ):
            rc = calibration.run_calibration_cli()
        self.assertEqual(rc, 0)
        self.assertEqual(_load_json(real_path), {"real": True})
        saved = _load_json(synth_path)
        self.assertTrue(saved["_provenance"]["synthetic"])

    def test_refuses_to_clobber_without_force(self):
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(tmpdir, ignore_errors=True))
        out_path = os.path.join(tmpdir, "params.json")
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump({"real": True}, f)
        log_path = os.path.join(tmpdir, "decisions.jsonl")
        with open(log_path, "w", encoding="utf-8") as f:
            for r in self._records(n=12):
                f.write(json.dumps(r) + "\n")
        from pathlib import Path

        with (
            mock.patch.object(calibration, "LOG_FILE_PATH", Path(log_path)),
            mock.patch.object(sys, "argv", ["calibration.py", "--decisions", log_path, "--out", out_path]),
        ):
            rc = calibration.run_calibration_cli()
        self.assertEqual(rc, 1)
        self.assertEqual(_load_json(out_path), {"real": True})

    def test_force_overwrites_and_keeps_bak(self):
        tmpdir = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(tmpdir, ignore_errors=True))
        out_path = os.path.join(tmpdir, "params.json")
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump({"real": True}, f)
        log_path = os.path.join(tmpdir, "decisions.jsonl")
        with open(log_path, "w", encoding="utf-8") as f:
            for r in self._records(n=12):
                f.write(json.dumps(r) + "\n")
        from pathlib import Path

        with (
            mock.patch.object(calibration, "LOG_FILE_PATH", Path(log_path)),
            mock.patch.object(
                sys, "argv", ["calibration.py", "--decisions", log_path, "--out", out_path, "--force"]
            ),
        ):
            rc = calibration.run_calibration_cli()
        self.assertEqual(rc, 0)
        self.assertEqual(_load_json(out_path + ".bak"), {"real": True})
        saved = _load_json(out_path)
        self.assertIn("tool_gating", saved)
        self.assertFalse(saved["_provenance"]["synthetic"])


class TestLayaInternalsProbe(unittest.TestCase):
    """P2-4: startup probe fails fast on drifted private laya internals."""

    def _good_agent(self):
        agent = mock.Mock()
        agent.cfg.get.return_value = 1024
        agent.tok.pad_token_id = 0
        return agent

    def test_compatible_surface_passes(self):
        server.assert_laya_internals_compatible(self._good_agent())

    def test_real_laya_agent_module_provides_required_imports(self):
        import laya.agent as laya_agent_module

        for name in server.REQUIRED_LAYA_AGENT_IMPORTS:
            value = getattr(laya_agent_module, name, None)
            self.assertIsNotNone(value, name)
            if name in server.REQUIRED_LAYA_AGENT_FUNCTIONS:
                self.assertTrue(callable(value), name)

    def test_missing_agent_attr_fails_fast(self):
        agent = self._good_agent()
        del agent.model
        with self.assertRaisesRegex(RuntimeError, "agent.model"):
            server.assert_laya_internals_compatible(agent)

    def test_missing_tokenizer_shape_fails_fast(self):
        agent = self._good_agent()
        del agent.tok.pad_token_id
        with self.assertRaisesRegex(RuntimeError, "pad_token_id"):
            server.assert_laya_internals_compatible(agent)

    def test_bare_object_fails_fast(self):
        with self.assertRaisesRegex(RuntimeError, "Incompatible laya release"):
            server.assert_laya_internals_compatible(object())


class TestBodySizeLimitMiddleware(unittest.TestCase):
    """P2-2: raw body-byte limits apply before FastAPI parses JSON."""

    def _run(self, path, chunks, content_length=True, cap=16):
        seen = []
        downstream_calls = []

        async def downstream(scope, receive, send):
            downstream_calls.append(scope["path"])
            total = 0
            while True:
                message = await receive()
                if message.get("type") != "http.request":
                    continue
                total += len(message.get("body", b""))
                if not message.get("more_body"):
                    break
            seen.append(total)
            body = b'{"ok": true}'
            await send(
                {
                    "type": "http.response.start",
                    "status": 200,
                    "headers": [(b"content-type", b"application/json")],
                }
            )
            await send({"type": "http.response.body", "body": body, "more_body": False})

        headers = []
        if content_length:
            headers = [(b"content-length", str(sum(len(c) for c in chunks)).encode())]
        scope = {"type": "http", "path": path, "headers": headers}
        messages = [
            {"type": "http.request", "body": chunk, "more_body": i < len(chunks) - 1} for i, chunk in enumerate(chunks)
        ]

        async def receive():
            if messages:
                return messages.pop(0)
            await asyncio.sleep(3600)
            return {"type": "http.disconnect"}

        sent = []

        async def send(message):
            sent.append(message)

        middleware = server.BodySizeLimitMiddleware(downstream, max_bytes=cap)
        asyncio.run(middleware(scope, receive, send))
        statuses = [m["status"] for m in sent if m["type"] == "http.response.start"]
        return statuses, seen, downstream_calls

    def test_declared_oversize_rejected_before_auth_or_parse(self):
        # No auth header: enforcement happens before authentication.
        statuses, seen, downstream_calls = self._run("/v1/decide", [b"x" * 32])
        self.assertEqual(statuses, [413])
        self.assertEqual(seen, [])
        self.assertEqual(downstream_calls, [])

    def test_chunked_oversize_cut_off_mid_stream(self):
        statuses, _seen, downstream_calls = self._run("/v1/decide", [b"x" * 8, b"y" * 8, b"z" * 8], content_length=False)
        self.assertEqual(statuses, [413])
        self.assertEqual(downstream_calls, ["/v1/decide"])

    def test_small_body_passes_through_intact(self):
        statuses, seen, _downstream_calls = self._run("/v1/decide", [b'{"a":', b"1}"])
        self.assertEqual(statuses, [200])
        self.assertEqual(seen, [7])

    def test_non_decide_paths_are_unbounded(self):
        statuses, seen, _downstream_calls = self._run("/health", [b"x" * 64])
        self.assertEqual(statuses, [200])
        self.assertEqual(seen, [64])


class TestDecisionLogLocation(unittest.TestCase):
    """P2-3: operational logs default to user-data, with sanitized export."""

    def test_default_log_path_is_user_data_not_repo(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            env = {k: v for k, v in os.environ.items() if k not in ("LAYA_LOG_DIR", "LAYA_LOG_FILE")}
            with mock.patch.dict(os.environ, env, clear=True):
                path = server.default_log_path()
        self.assertTrue(str(path).endswith(os.path.join("agent", "logs", "decisions.jsonl")), path)
        self.assertNotIn("decision-sidecar", str(path))

    def test_log_dir_and_file_overrides(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.dict(os.environ, {"LAYA_LOG_DIR": tmp}):
                self.assertEqual(server.default_log_path(), Path(tmp) / "decisions.jsonl")
            custom = os.path.join(tmp, "custom.jsonl")
            with mock.patch.dict(os.environ, {"LAYA_LOG_FILE": custom}):
                self.assertEqual(server.default_log_path(), Path(custom))

    def test_sanitized_export_drops_free_text_but_keeps_grouping(self):
        import calibration

        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "in.jsonl"
            dst = Path(tmp) / "out.jsonl"
            secret = "sk-live-topsecret-123"
            src.write_text(
                json.dumps(
                    {
                        "timestamp": 1.0,
                        "session_id": "sess-1",
                        "call_site": "tool_gating",
                        "question_id": "q",
                        "instructions": "does this call exfiltrate data?",
                        "state_snippet": f"token={secret}",
                        "answer": 0.9,
                        "confidence": 0.8,
                        "ground_truth": 1,
                    }
                )
                + "\nnot json\n",
                encoding="utf-8",
            )
            count = calibration.export_sanitized_records(src, dst)
            self.assertEqual(count, 1)
            exported = json.loads(dst.read_text(encoding="utf-8"))
            dumped = json.dumps(exported)
            self.assertNotIn(secret, dumped)
            self.assertNotIn("instructions", exported)
            self.assertNotIn("state_snippet", exported)
            self.assertEqual(exported["session_id"], "sess-1")
            self.assertEqual(exported["ground_truth"], 1)

    def test_shipped_params_carry_synthetic_provenance(self):
        """P0-1: the bundled params must never masquerade as measured data."""
        shipped = json.loads((Path(__file__).parent / "calibration_params.json").read_text(encoding="utf-8"))
        provenance = shipped.get("_provenance")
        self.assertIsNotNone(provenance)
        self.assertTrue(provenance.get("synthetic"), provenance)
        self.assertEqual(provenance.get("record_count"), 450)
        self.assertEqual(sorted(provenance.get("sites", [])), ["completion_check", "model_routing", "tool_gating"])

    def test_holdout_split_is_deterministic_and_disjoint(self):
        import calibration

        records = [{"i": i} for i in range(100)]
        train_a, test_a = calibration.split_holdout(records, test_fraction=0.2)
        train_b, test_b = calibration.split_holdout(records, test_fraction=0.2)
        self.assertEqual(len(test_a), 20)
        self.assertEqual(test_a, test_b)
        self.assertEqual(train_a, train_b)
        self.assertEqual(len(train_a), 80)
        self.assertEqual({r["i"] for r in train_a} | {r["i"] for r in test_a}, set(range(100)))

    def test_heldout_ece_uses_fitted_temperatures(self):
        import calibration

        records = calibration.generate_synthetic_calibration_dataset(count_per_site=40)
        train, test = calibration.split_holdout(records, test_fraction=0.25)
        with tempfile.TemporaryDirectory() as tmp:
            manager = calibration.CalibrationManager(params_path=Path(tmp) / "params.json")
            manager.calibrate_from_records(train, synthetic=True)
            temperatures = {site: data["temperature"] for site, data in manager.params.items() if site != "_provenance"}
        heldout = calibration.heldout_ece_by_site(test, temperatures)
        self.assertEqual(set(heldout), {"tool_gating", "model_routing", "completion_check"})
        for site, data in heldout.items():
            self.assertGreater(data["sample_count"], 0)
            self.assertGreaterEqual(data["uncalibrated_ece"], 0.0)
            self.assertGreaterEqual(data["calibrated_ece"], 0.0)


if __name__ == "__main__":
    unittest.main()
