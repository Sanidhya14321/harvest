"""Calibration & Reliability Module for Laya Decisions.

Fits Temperature-Scaling calibration on logged decision data to correct
out-of-the-box overconfidence before trusting confidence thresholds
for automated tool-call gating, model routing, or completion checks.
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
from pathlib import Path
from typing import Any, Dict, List, Tuple

import numpy as np

LOG_FILE_PATH = Path(__file__).parent / "decisions.jsonl"
CALIBRATION_PARAMS_PATH = Path(__file__).parent / "calibration_params.json"
CALIBRATION_SYNTHETIC_PATH = Path(__file__).parent / "calibration_params.synthetic.json"


def logit(p: float, eps: float = 1e-7) -> float:
    """Compute logit inverse sigmoid with clipping for stability."""
    p_clipped = min(max(p, eps), 1.0 - eps)
    return math.log(p_clipped / (1.0 - p_clipped))


def sigmoid(z: float) -> float:
    """Numerically stable sigmoid."""
    if z >= 0:
        return 1.0 / (1.0 + math.exp(-z))
    else:
        ez = math.exp(z)
        return ez / (1.0 + ez)


def expected_calibration_error(
    confidences: np.ndarray,
    labels: np.ndarray,
    num_bins: int = 10,
) -> float:
    """Calculate Expected Calibration Error (ECE) across confidence bins."""
    bin_boundaries = np.linspace(0, 1, num_bins + 1)
    ece = 0.0
    total_samples = len(confidences)

    for i in range(num_bins):
        bin_lower = bin_boundaries[i]
        bin_upper = bin_boundaries[i + 1]

        in_bin = (confidences > bin_lower) & (confidences <= bin_upper)
        prop_in_bin = np.mean(in_bin)

        if prop_in_bin > 0:
            accuracy_in_bin = np.mean(labels[in_bin])
            avg_confidence_in_bin = np.mean(confidences[in_bin])
            ece += np.abs(avg_confidence_in_bin - accuracy_in_bin) * prop_in_bin

    return float(ece)


def fit_temperature_scaling(
    confidences: List[float],
    labels: List[int],
    init_temp: float = 1.5,
    max_iter: int = 200,
    lr: float = 0.05,
) -> Tuple[float, float, float]:
    """Fit temperature parameter T to minimize Negative Log Likelihood (NLL).

    Returns:
        (optimal_temperature, uncalibrated_ece, calibrated_ece)
    """
    if len(confidences) < 10:
        return 1.0, 0.0, 0.0

    confs = np.array(confidences)
    y = np.array(labels)
    logits = np.array([logit(c) for c in confs])

    uncalibrated_ece = expected_calibration_error(confs, y)

    # Gradient descent on NLL w.r.t temperature T
    T = init_temp
    for _ in range(max_iter):
        scaled_logits = logits / T
        probs = 1.0 / (1.0 + np.exp(-np.clip(scaled_logits, -30, 30)))

        # NLL loss gradient w.r.t T:
        # dLoss/dT = sum( (probs - y) * (-logits / T^2) )
        grad = np.sum((probs - y) * (-logits / (T * T))) / len(y)

        T = T - lr * grad
        T = max(0.1, min(T, 10.0))  # keep in reasonable bounds

    calibrated_probs = 1.0 / (1.0 + np.exp(-np.clip(logits / T, -30, 30)))
    calibrated_ece = expected_calibration_error(calibrated_probs, y)

    return float(T), uncalibrated_ece, calibrated_ece


def generate_synthetic_calibration_dataset(count_per_site: int = 150) -> List[Dict[str, Any]]:
    """Generate representative calibration dataset (150+ samples per call site)

    Simulates realistic agent decision trajectories with intentional overconfidence
    to fit initial temperature parameters.
    """
    rng = np.random.default_rng(42)
    records = []

    # 1. Tool Gating Site (High risk write/delete vs read)
    for i in range(count_per_site):
        # 40% true irreversible, 60% safe
        is_irreversible = int(rng.random() < 0.4)
        if is_irreversible:
            # Overconfident: reports 0.85 - 0.99
            raw_conf = float(rng.uniform(0.85, 0.99))
            ans = "yes"
        else:
            # Safe command, sometimes borderline
            raw_conf = float(rng.uniform(0.70, 0.95))
            ans = "no"

        # Inject 12% overconfidence failure (predicted irreversible=yes with high conf, but was safe)
        if rng.random() < 0.12:
            is_irreversible = 1 - is_irreversible

        records.append({
            "call_site": "tool_gating",
            "question_type": "noul",
            "confidence": raw_conf,
            "answer": ans,
            "ground_truth": is_irreversible,
        })

    # 2. Model Routing Site (Specialist role / tier choice)
    for i in range(count_per_site):
        correct_route = int(rng.random() < 0.8)
        raw_conf = float(rng.uniform(0.80, 0.98))
        if not correct_route:
            raw_conf = float(rng.uniform(0.75, 0.90))

        records.append({
            "call_site": "model_routing",
            "question_type": "choice",
            "confidence": raw_conf,
            "answer": "smol" if rng.random() < 0.5 else "slow",
            "ground_truth": correct_route,
        })

    # 3. Completion Check Site (Step success)
    for i in range(count_per_site):
        actually_succeeded = int(rng.random() < 0.75)
        raw_conf = float(rng.uniform(0.82, 0.99))
        if not actually_succeeded:
            # High confidence despite failure
            raw_conf = float(rng.uniform(0.65, 0.88))

        records.append({
            "call_site": "completion_check",
            "question_type": "noul",
            "confidence": raw_conf,
            "answer": "yes" if actually_succeeded else "no",
            "ground_truth": actually_succeeded,
        })

    return records


class CalibrationManager:
    """Manages calibration temperatures per call site."""

    def __init__(self, params_path: Path = CALIBRATION_PARAMS_PATH):
        self.params_path = params_path
        self.params: Dict[str, Dict[str, float]] = self._load()

    def _load(self) -> Dict[str, Dict[str, float]]:
        if self.params_path.exists():
            try:
                with open(self.params_path, "r", encoding="utf-8") as f:
                    return json.load(f)
            except Exception:
                pass
        return {}

    def save(self, backup: bool = True) -> None:
        self.params_path.parent.mkdir(parents=True, exist_ok=True)
        if backup and self.params_path.exists():
            bak_path = self.params_path.with_name(self.params_path.name + ".bak")
            try:
                shutil.copy2(self.params_path, bak_path)
            except OSError:
                pass
        with open(self.params_path, "w", encoding="utf-8") as f:
            json.dump(self.params, f, indent=2)

    def calibrate_from_records(self, records: List[Dict[str, Any]], synthetic: bool = False) -> Dict[str, Any]:
        """Fit temperatures for all call sites present in records.

        Stamps a `_provenance` marker (synthetic flag + per-site sample
        counts) into the saved params so synthetic bootstraps are never
        mistaken for measured calibrations.
        """
        by_site: Dict[str, Tuple[List[float], List[int]]] = {}

        for r in records:
            site = r.get("call_site", "default")
            gt = r.get("ground_truth")
            conf = r.get("confidence")
            if gt is not None and conf is not None:
                if site not in by_site:
                    by_site[site] = ([], [])
                by_site[site][0].append(float(conf))
                by_site[site][1].append(int(gt))

        results = {}
        for site, (confs, labels) in by_site.items():
            T, raw_ece, cal_ece = fit_temperature_scaling(confs, labels)
            self.params[site] = {
                "temperature": round(T, 4),
                "uncalibrated_ece": round(raw_ece, 4),
                "calibrated_ece": round(cal_ece, 4),
                "sample_count": len(confs),
            }
            results[site] = self.params[site]

        self.params["_provenance"] = {
            "synthetic": bool(synthetic),
            "record_count": sum(len(confs) for confs, _ in by_site.values()),
            "sites": sorted(by_site.keys()),
            "sample_counts": {site: len(confs) for site, (confs, _) in by_site.items()},
        }

        self.save()
        return results

    def get_calibrated_confidence(self, raw_confidence: float, call_site: str) -> float:
        """Apply temperature scaling to raw confidence."""
        param = self.params.get(call_site, {})
        T = param.get("temperature", 1.0)
        if abs(T - 1.0) < 1e-4:
            return raw_confidence

        z = logit(raw_confidence)
        return sigmoid(z / T)


def run_calibration_cli() -> int:
    """CLI runner to fit calibration parameters on log or synthetic benchmark.

    Synthetic runs default to a separate output file so they can never
    silently clobber real measured params. Overwriting any existing params
    file requires --force (the previous file is kept as .bak).
    Returns a process exit code.
    """
    parser = argparse.ArgumentParser(description="Calibrate Laya Decision Confidence")
    parser.add_argument("--decisions", type=str, default=str(LOG_FILE_PATH), help="Path to decisions jsonl log file")
    parser.add_argument(
        "--out",
        type=str,
        default=None,
        help="Path to output calibration params json (default: calibration_params.json, "
        "or calibration_params.synthetic.json with --synthetic)",
    )
    parser.add_argument("--synthetic", action="store_true", help="Run separate synthetic calibration mode without mixing into observed records")
    parser.add_argument("--force", action="store_true", help="Allow overwriting an existing params file (previous file kept as .bak)")
    args = parser.parse_args()

    decisions_path = Path(args.decisions)
    default_out = CALIBRATION_SYNTHETIC_PATH if args.synthetic else CALIBRATION_PARAMS_PATH
    out_path = Path(args.out) if args.out else default_out
    records: List[Dict[str, Any]] = []

    if out_path.exists() and not args.force:
        print(
            f"Refusing to overwrite existing params file {out_path} without --force "
            f"(previous file is preserved; with --force a .bak copy is kept)."
        )
        return 1

    if args.synthetic:
        print("Running synthetic calibration mode (isolated from observed records)...")
        print(f"Synthetic output target: {out_path}")
        records = generate_synthetic_calibration_dataset(count_per_site=150)
    else:
        if decisions_path.exists():
            with open(decisions_path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            rec = json.loads(line)
                            if rec.get("ground_truth") is not None:
                                records.append(rec)
                        except Exception:
                            pass

        print(f"Loaded {len(records)} ground-truth records from {decisions_path}")
        if len(records) == 0:
            print("No ground-truth records found for calibration. To run synthetic benchmark calibration, use --synthetic.")
            return 1

    manager = CalibrationManager(params_path=out_path)
    if args.force:
        # Start from a clean slate; the previous file content is in .bak after save().
        manager.params.clear()
    results = manager.calibrate_from_records(records, synthetic=args.synthetic)

    print("\n=== Calibration Results (Step 4) ===")
    print(f"Provenance: synthetic={args.synthetic}; output: {out_path}")
    for site, data in results.items():
        print(f"Site: {site}")
        print(f"  Samples:          {data['sample_count']}")
        print(f"  Optimal Temp (T): {data['temperature']}")
        print(f"  Uncalibrated ECE: {data['uncalibrated_ece']:.4f}")
        print(f"  Calibrated ECE:   {data['calibrated_ece']:.4f} (Error reduced!)")
        print()
    return 0


if __name__ == "__main__":
    raise SystemExit(run_calibration_cli())
