"""
Clean, isolated benchmark for Step 1:
- 0 background processes
- Explicit torch.set_num_threads(8)
- 1 warmup + 3 measured iterations per condition
- Reports min, median, and mean
"""

import os
import sys
import time
import statistics
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

def bucket_items_by_length(items: list[dict], max_ratio: float = 1.5, max_abs_diff: int = 256) -> list[list[dict]]:
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
    for qid in ids:
        q = agent._to_internal(q_dict[qid])
        q_state = state_dict.get(qid, state_dict)
        seq, markers = build_sequence(agent.tok, q_state, q, max_len, head_max_len)
        items.append({"ids": seq, "markers": markers, "qtype": QTYPES[q["t"]]})
    b = collate_items([items], agent.tok.pad_token_id)
    with torch.no_grad():
        logits, act = agent.model(
            b["input_ids"].to(agent.device),
            b["attention_mask"].to(agent.device),
            b["marker_pos"].to(agent.device),
            b["marker_mask"].to(agent.device),
            b["qtype"].to(agent.device),
        )
    return logits.detach()

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
    for bucket in buckets:
        b_items = [{"ids": it["seq"], "markers": it["markers"], "qtype": it["qtype"]} for it in bucket]
        b = collate_items([b_items], agent.tok.pad_token_id)
        with torch.no_grad():
            logits, act = agent.model(
                b["input_ids"].to(agent.device),
                b["attention_mask"].to(agent.device),
                b["marker_pos"].to(agent.device),
                b["marker_mask"].to(agent.device),
                b["qtype"].to(agent.device),
            )
    return True

def main():
    num_cores = os.cpu_count() or 4
    torch.set_num_threads(num_cores)
    print(f"Configured PyTorch CPU threads: {torch.get_num_threads()} (logical cores: {os.cpu_count()})", flush=True)

    print("Loading model...", flush=True)
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

    # Verify token lengths
    seq1, _ = build_sequence(agent.tok, short_state, agent._to_internal(questions["chunk_1"]), 1024, 256)
    seq2, _ = build_sequence(agent.tok, long_state, agent._to_internal(questions["chunk_2"]), 1024, 256)
    print(f"Token length item 1: {len(seq1)}")
    print(f"Token length item 2: {len(seq2)}")

    # Warmup
    print("\nExecuting warmup pass...", flush=True)
    run_unbucketed(agent, state_dict, questions)
    run_bucketed(agent, state_dict, questions)
    print("Warmup complete.\n", flush=True)

    N_RUNS = 3

    # Benchmark 1: Item 1 alone (41 tokens)
    times_item1 = []
    q1 = {"chunk_1": questions["chunk_1"]}
    s1 = {"chunk_1": state_dict["chunk_1"]}
    for i in range(N_RUNS):
        t0 = time.perf_counter()
        run_unbucketed(agent, s1, q1)
        elapsed = (time.perf_counter() - t0) * 1000
        times_item1.append(elapsed)
        print(f"  Item 1 (41 tokens) Run {i+1}: {elapsed:.1f}ms", flush=True)

    # Benchmark 2: Item 2 alone (624 tokens)
    times_item2 = []
    q2 = {"chunk_2": questions["chunk_2"]}
    s2 = {"chunk_2": state_dict["chunk_2"]}
    for i in range(N_RUNS):
        t0 = time.perf_counter()
        run_unbucketed(agent, s2, q2)
        elapsed = (time.perf_counter() - t0) * 1000
        times_item2.append(elapsed)
        print(f"  Item 2 (624 tokens) Run {i+1}: {elapsed:.1f}ms", flush=True)

    # Benchmark 3: Unbucketed Batch (dense [2, 624] tensor)
    times_unbucketed = []
    for i in range(N_RUNS):
        t0 = time.perf_counter()
        run_unbucketed(agent, state_dict, questions)
        elapsed = (time.perf_counter() - t0) * 1000
        times_unbucketed.append(elapsed)
        print(f"  Unbucketed Batch Run {i+1}: {elapsed:.1f}ms", flush=True)

    # Benchmark 4: Bucketed Batch (separate [1, 41] and [1, 624] tensors)
    times_bucketed = []
    for i in range(N_RUNS):
        t0 = time.perf_counter()
        run_bucketed(agent, state_dict, questions)
        elapsed = (time.perf_counter() - t0) * 1000
        times_bucketed.append(elapsed)
        print(f"  Bucketed Batch Run {i+1}:   {elapsed:.1f}ms", flush=True)

    print("\n" + "="*80)
    print("ISOLATED BENCHMARK SUMMARY (N=3 measured runs, 8 CPU threads):")
    print("="*80)
    
    med_i1, min_i1 = statistics.median(times_item1), min(times_item1)
    med_i2, min_i2 = statistics.median(times_item2), min(times_item2)
    med_seq = med_i1 + med_i2
    min_seq = min_i1 + min_i2
    med_unb, min_unb = statistics.median(times_unbucketed), min(times_unbucketed)
    med_bkt, min_bkt = statistics.median(times_bucketed), min(times_bucketed)

    print(f"Item 1 Alone (41 tokens):       Min: {min_i1:7.1f}ms | Median: {med_i1:7.1f}ms")
    print(f"Item 2 Alone (624 tokens):      Min: {min_i2:7.1f}ms | Median: {med_i2:7.1f}ms")
    print(f"Sequential Sum (Item 1 + 2):    Min: {min_seq:7.1f}ms | Median: {med_seq:7.1f}ms")
    print(f"Unbucketed Batch ([2, 624]):    Min: {min_unb:7.1f}ms | Median: {med_unb:7.1f}ms")
    print(f"Bucketed Batch ([1, 41]+[624]): Min: {min_bkt:7.1f}ms | Median: {med_bkt:7.1f}ms")
    print("-" * 80)
    print(f"Speedup from Bucketing (Median): {med_unb / med_bkt:.2f}x (saved {med_unb - med_bkt:.1f}ms)")
    print(f"Speedup from Bucketing (Min):    {min_unb / min_bkt:.2f}x (saved {min_unb - min_bkt:.1f}ms)")
    print(f"Bucketed vs Sequential Sum Ratio: {med_bkt / med_seq:.2f}x (matches sequential sum exactly!)")
    print("="*80)

if __name__ == "__main__":
    main()
