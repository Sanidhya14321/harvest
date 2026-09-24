"""Test for length-bucketing in decision-sidecar.

Validates:
1. Unit tests for bucket_items_by_length logic portably without ML dependencies.
2. Optional model-backed numerical equivalence when RUN_LAYA_BENCHMARK is set.
"""

from __future__ import annotations

import os
import sys
import unittest

try:
    from bucketing import bucket_items_by_length
except ImportError:
    from .bucketing import bucket_items_by_length


class TestBucketing(unittest.TestCase):
    """Unit tests for bucket_items_by_length."""

    def test_empty(self):
        self.assertEqual(bucket_items_by_length([]), [])

    def test_single(self):
        single = [{"seq": [1] * 50}]
        res = bucket_items_by_length(single)
        self.assertEqual(len(res), 1)
        self.assertEqual(len(res[0]), 1)

    def test_uniform(self):
        # Uniform lengths: 50, 55, 60 -> 1 bucket
        uniform = [{"seq": [1] * 50}, {"seq": [1] * 55}, {"seq": [1] * 60}]
        b = bucket_items_by_length(uniform)
        self.assertEqual(len(b), 1)
        self.assertEqual(len(b[0]), 3)

    def test_mismatched(self):
        # Mismatched lengths: 41 and 624 -> 2 buckets
        mismatched = [{"seq": [1] * 41}, {"seq": [1] * 624}]
        b = bucket_items_by_length(mismatched)
        self.assertEqual(len(b), 2)
        self.assertEqual(len(b[0]), 1)
        self.assertEqual(len(b[0][0]["seq"]), 41)
        self.assertEqual(len(b[1]), 1)
        self.assertEqual(len(b[1][0]["seq"]), 624)

    def test_mixed(self):
        # Mixed: [40, 45, 500, 550] -> 2 buckets: [40, 45] and [500, 550]
        mixed = [{"seq": [1] * 550}, {"seq": [1] * 40}, {"seq": [1] * 500}, {"seq": [1] * 45}]
        b = bucket_items_by_length(mixed)
        self.assertEqual(len(b), 2)
        self.assertEqual([len(x["seq"]) for x in b[0]], [40, 45])
        self.assertEqual([len(x["seq"]) for x in b[1]], [500, 550])


def run_benchmark():
    """Optional benchmark for numerical equivalence and latency."""
    import numpy as np
    import torch
    import laya
    from laya.agent import (
        QTYPES,
        build_sequence,
        collate_items,
        render_options,
        temp_bucket,
        confidence_from_probs,
    )

    MODEL_ID = "convaiinnovations/laya-typed-decisions"
    print("\nLoading model for numerical and benchmark validation...")
    agent = laya.load(MODEL_ID, device="cpu")
    agent.model.eval()

    relevance_criteria = [
        "Irrelevant: Outdated context, obsolete file contents, or superseded tool errors.",
        "Potentially useful: Background context or historical discussion.",
        "Relevant: Active code, current file contents, or key user requirements.",
    ]

    questions = {
        "chunk_1": {
            "type": "score",
            "instructions": "Rate how relevant this Tool 'read_file' result is to the current task/goal",
            "criteria": relevance_criteria,
        },
        "chunk_2": {
            "type": "score",
            "instructions": "Rate how relevant this Tool 'run_command' result is to the current task/goal",
            "criteria": relevance_criteria,
        },
    }

    short_state = "Current Task/Goal:\nFix typo in README\n\nCandidate Chunk (Tool 'read_file' result):\nLine 1: # Readme\nLine 2: Fixed typo here."
    long_state = "Current Task/Goal:\nOptimize inference latency\n\nCandidate Chunk (Tool 'run_command' result):\n" + "\n".join(
        [f"Step {i}: executed subprocess command with stdout tensor shape [{i*16}, {i*32}] - returncode 0" for i in range(35)]
    )

    state_dict = {
        "chunk_1": short_state,
        "chunk_2": long_state,
    }

    def run_unbucketed(agent, state_dict, q_dict, max_len=1024, head_max_len=256):
        ids = list(q_dict.keys())
        items = []
        raw_meta = []
        for qid in ids:
            q = agent._to_internal(q_dict[qid])
            q_state = state_dict.get(qid, state_dict)
            seq, markers = build_sequence(agent.tok, q_state, q, max_len, head_max_len)
            items.append({"ids": seq, "markers": markers, "qtype": QTYPES[q["t"]]})
            raw_meta.append({"qid": qid, "q": q, "markers": markers, "qtype": QTYPES[q["t"]]})

        b = collate_items([items], agent.tok.pad_token_id)
        with torch.no_grad():
            logits, act = agent.model(
                b["input_ids"].to(agent.device),
                b["attention_mask"].to(agent.device),
                b["marker_pos"].to(agent.device),
                b["marker_mask"].to(agent.device),
                b["qtype"].to(agent.device),
            )
        b_logits = logits.detach().float().cpu().numpy()
        b_act = torch.softmax(act.detach().float(), -1).cpu().numpy()

        answers = {}
        for r, meta in enumerate(raw_meta):
            qid = meta["qid"]
            q = meta["q"]
            k = len(meta["markers"])
            qt = meta["qtype"]
            t_scale = agent.temperature_by_options.get(temp_bucket(qt, k), agent.temperature[qt])
            z = b_logits[r, :k] / t_scale
            p = np.exp(z - z.max())
            p = p / p.sum()
            conf_score = round(confidence_from_probs(p, k), 4)
            ext = {"act_probability": round(float(b_act[r, 0]), 4)}
            exp_score = float((np.arange(k) * p).sum())
            answers[qid] = {
                "type": "score",
                "score": round(exp_score, 4),
                "probabilities": {str(i): round(float(v), 4) for i, v in enumerate(p)},
                "confidence": conf_score,
                "action": ext,
                "raw_logits": b_logits[r, :k],
            }
        return answers, int(b["attention_mask"].sum())

    def run_bucketed(agent, state_dict, q_dict, max_len=1024, head_max_len=256):
        ids = list(q_dict.keys())
        raw_items = []
        for qid in ids:
            q = agent._to_internal(q_dict[qid])
            q_state = state_dict.get(qid, state_dict)
            seq, markers = build_sequence(agent.tok, q_state, q, max_len, head_max_len)
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
        for bucket in buckets:
            b_items = [{"ids": it["seq"], "markers": it["markers"], "qtype": it["qtype"]} for it in bucket]
            b = collate_items([b_items], agent.tok.pad_token_id)
            total_tokens += int(b["attention_mask"].sum())
            with torch.no_grad():
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
                exp_score = float((np.arange(k) * p).sum())
                answers[qid] = {
                    "type": "score",
                    "score": round(exp_score, 4),
                    "probabilities": {str(i): round(float(v), 4) for i, v in enumerate(p)},
                    "confidence": conf_score,
                    "action": ext,
                    "raw_logits": b_logits[r, :k],
                }
        return answers, total_tokens

    print("\nValidating numerical equivalence between unbucketed and bucketed...")
    ans_unbucketed, tok_unbucketed = run_unbucketed(agent, state_dict, questions)
    ans_bucketed, tok_bucketed = run_bucketed(agent, state_dict, questions)
    print(f"Tokens unbucketed: {tok_unbucketed} vs bucketed: {tok_bucketed}")
    for qid in questions:
        diff = np.abs(ans_unbucketed[qid]["raw_logits"] - ans_bucketed[qid]["raw_logits"]).max()
        print(f"Max logit difference for {qid}: {diff:.6e}")
        assert diff < 1e-4, f"Numerical divergence in logits for {qid}: {diff}"


if __name__ == "__main__":
    if "--benchmark" in sys.argv or os.getenv("RUN_LAYA_BENCHMARK"):
        run_benchmark()
    else:
        unittest.main()
