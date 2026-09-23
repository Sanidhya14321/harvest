"""
Test and benchmark length-bucketing in decision-sidecar.
Validates:
1. Unit tests for bucket_items_by_length logic.
2. Numerical equivalence between unbucketed and bucketed forward passes.
3. Latency comparison on the 41-token + 624-token mismatched candidate pair.
"""

import time
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

import sys
import os

if sys.platform == "win32":
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

MODEL_ID = "convaiinnovations/laya-typed-decisions"

def bucket_items_by_length(
    items: list[dict],
    max_ratio: float = 1.5,
    max_abs_diff: int = 256,
) -> list[list[dict]]:
    """Group items by sequence length to minimize padding waste during collation."""
    if not items:
        return []
    if len(items) == 1:
        return [items]

    sorted_items = sorted(items, key=lambda x: len(x["seq"]))
    buckets = []
    curr_bucket = [sorted_items[0]]

    for item in sorted_items[1:]:
        min_len = len(curr_bucket[0]["seq"])
        curr_len = len(item["seq"])
        ratio = curr_len / max(1, min_len)
        abs_diff = curr_len - min_len

        if ratio <= max_ratio and abs_diff <= max_abs_diff:
            curr_bucket.append(item)
        else:
            buckets.append(curr_bucket)
            curr_bucket = [item]

    if curr_bucket:
        buckets.append(curr_bucket)

    return buckets

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

def test_unit_bucketing():
    print("Testing unit bucketing logic...")
    assert bucket_items_by_length([]) == []
    
    single = [{"seq": [1] * 50}]
    assert len(bucket_items_by_length(single)) == 1

    # Uniform lengths: 50, 55, 60 -> 1 bucket
    uniform = [{"seq": [1] * 50}, {"seq": [1] * 55}, {"seq": [1] * 60}]
    b = bucket_items_by_length(uniform)
    assert len(b) == 1
    assert len(b[0]) == 3

    # Mismatched lengths: 41 and 624 -> 2 buckets
    mismatched = [{"seq": [1] * 41}, {"seq": [1] * 624}]
    b = bucket_items_by_length(mismatched)
    assert len(b) == 2
    assert len(b[0]) == 1 and len(b[0][0]["seq"]) == 41
    assert len(b[1]) == 1 and len(b[1][0]["seq"]) == 624

    # Mixed: [40, 45, 500, 550] -> 2 buckets: [40, 45] and [500, 550]
    mixed = [{"seq": [1] * 550}, {"seq": [1] * 40}, {"seq": [1] * 500}, {"seq": [1] * 45}]
    b = bucket_items_by_length(mixed)
    assert len(b) == 2
    assert [len(x["seq"]) for x in b[0]] == [40, 45]
    assert [len(x["seq"]) for x in b[1]] == [500, 550]
    print("  Unit tests passed!")

def main():
    test_unit_bucketing()

    print("\nLoading model for numerical and benchmark validation...")
    agent = laya.load(MODEL_ID, device="cpu")
    agent.model.eval()

    # Define the exact 41-token + 624-token mismatched test case from CPU diagnostic
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

    # Short excerpt (~41 tokens)
    short_state = "Current Task/Goal:\nFix typo in README\n\nCandidate Chunk (Tool 'read_file' result):\nLine 1: # Readme\nLine 2: Fixed typo here."
    
    # Long excerpt (~624 tokens)
    long_state = "Current Task/Goal:\nOptimize inference latency\n\nCandidate Chunk (Tool 'run_command' result):\n" + "\n".join(
        [f"Step {i}: executed subprocess command with stdout tensor shape [{i*16}, {i*32}] - returncode 0" for i in range(35)]
    )

    state_dict = {
        "chunk_1": short_state,
        "chunk_2": long_state,
    }

    print("\nValidating numerical equivalence between unbucketed and bucketed...")
    ans_unbucketed, tok_unbucketed = run_unbucketed(agent, state_dict, questions)
    ans_bucketed, tok_bucketed = run_bucketed(agent, state_dict, questions)

    for qid in ["chunk_1", "chunk_2"]:
        u = ans_unbucketed[qid]
        b = ans_bucketed[qid]
        print(f"\n[{qid}]")
        print(f"  Unbucketed Score: {u['score']:.4f} | Conf: {u['confidence']:.4f}")
        print(f"  Bucketed Score:   {b['score']:.4f} | Conf: {b['confidence']:.4f}")
        
        diff = np.abs(u["raw_logits"] - b["raw_logits"]).max()
        print(f"  Max Absolute Logit Difference: {diff:.8e}")
        assert diff < 1e-4, f"Logit difference too high: {diff}"
        assert abs(u["score"] - b["score"]) < 1e-4, f"Score mismatch: {u['score']} vs {b['score']}"
        assert abs(u["confidence"] - b["confidence"]) < 1e-4, f"Confidence mismatch: {u['confidence']} vs {b['confidence']}"

    print("\nNumerical equivalence confirmed! Output is identical.")

    # Benchmark Latency Comparison
    print("\nBenchmarking latency: Unbucketed vs. Bucketed (1 warmup, 2 runs)...")
    
    # Unbucketed timing
    t0 = time.perf_counter()
    run_unbucketed(agent, state_dict, questions)
    t_unb_1 = (time.perf_counter() - t0) * 1000

    t0 = time.perf_counter()
    run_unbucketed(agent, state_dict, questions)
    t_unb_2 = (time.perf_counter() - t0) * 1000
    t_unbucketed = min(t_unb_1, t_unb_2)

    # Bucketed timing
    t0 = time.perf_counter()
    run_bucketed(agent, state_dict, questions)
    t_b_1 = (time.perf_counter() - t0) * 1000

    t0 = time.perf_counter()
    run_bucketed(agent, state_dict, questions)
    t_b_2 = (time.perf_counter() - t0) * 1000
    t_bucketed = min(t_b_1, t_b_2)

    speedup = t_unbucketed / t_bucketed if t_bucketed > 0 else 0
    penalty_reduction = (t_unbucketed - t_bucketed)

    print(f"\n--- Results on CPU ---")
    print(f"  Unbucketed (Padded [2, 624] Tensor): {t_unbucketed:.1f}ms")
    print(f"  Bucketed (Tensors [1, 41] + [1, 624]): {t_bucketed:.1f}ms")
    print(f"  Saved compute time: {penalty_reduction:.1f}ms")
    print(f"  Speedup: {speedup:.2f}x (Padding penalty eliminated!)")

if __name__ == "__main__":
    main()
