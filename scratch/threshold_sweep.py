"""
Compute accuracy, correct auto-pick rate, and wrong auto-pick rate
across candidate threshold cuts for Format A vs Format C.
"""

# Format A data (from empirical 10-task evaluation)
# (id, is_correct, confidence)
format_a_data = [
    ("TC-01", False, 0.0142),
    ("TC-02", False, 0.0213),
    ("TC-03", True,  0.0800),
    ("TC-04", True,  0.0468),
    ("TC-05", True,  0.0385),
    ("TC-06", True,  0.0129),
    ("TC-07", True,  0.0724),
    ("TC-08", False, 0.0293),
    ("TC-09", True,  0.0445),
    ("TC-10", True,  0.0443),
]

# Format C data (from empirical 10-task evaluation)
format_c_data = [
    ("TC-01", False, 0.0013),
    ("TC-02", False, 0.0050),
    ("TC-03", True,  0.0289),
    ("TC-04", True,  0.0283),
    ("TC-05", True,  0.0294),
    ("TC-06", True,  0.0128),
    ("TC-07", True,  0.0152),
    ("TC-08", True,  0.0092),
    ("TC-09", True,  0.0313),
    ("TC-10", True,  0.0104),
]

def sweep_thresholds(data, name):
    print(f"\n{'='*75}\nTHRESHOLD SWEEP FOR: {name}\n{'='*75}")
    print(f"{'Threshold (th)':<15} | {'Auto-Picked':<12} | {'Correct Auto':<14} | {'Wrong Auto':<12} | {'Auto Precision':<15} | {'Escalated':<10}")
    print("-" * 75)
    
    # Collect all unique confidence thresholds plus standard cuts
    thresholds = sorted(list(set([0.0, 0.005, 0.010, 0.012, 0.015, 0.020, 0.025, 0.030, 0.040, 0.050, 0.080, 0.80] + [d[2] for d in data])))
    
    for th in thresholds:
        auto_picks = [d for d in data if d[2] >= th]
        escalated = [d for d in data if d[2] < th]
        
        correct_auto = [d for d in auto_picks if d[1]]
        wrong_auto = [d for d in auto_picks if not d[1]]
        
        precision = (len(correct_auto) / len(auto_picks) * 100) if auto_picks else 100.0
        
        print(f"{th:<15.4f} | {len(auto_picks):<12} | {len(correct_auto):<14} | {len(wrong_auto):<12} | {precision:<14.1f}% | {len(escalated):<10}")

def main():
    sweep_thresholds(format_a_data, "Format A: Current Baseline (1-line phrases)")
    sweep_thresholds(format_c_data, "Format C: Workflow-Oriented with Concrete Examples")

    print("\n" + "="*75)
    print("TC-02 vs TC-06 COMPARISON:")
    print("="*75)
    print("Format A:")
    print("  TC-02 (WRONG):   conf = 0.0213")
    print("  TC-06 (CORRECT): conf = 0.0129")
    print("  Status: INVERTED (Wrong has HIGHER confidence than Correct!) -> Separation impossible.")
    print("\nFormat C:")
    print("  TC-02 (WRONG):   conf = 0.0050")
    print("  TC-06 (CORRECT): conf = 0.0128")
    print("  Status: SEPARABLE (Correct is 2.56x higher than Wrong!) -> Clear threshold boundary.")

if __name__ == "__main__":
    main()
