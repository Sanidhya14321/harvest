import urllib.request
import json
import time
import statistics

relevance_criteria = [
    "Irrelevant: Outdated context, obsolete file contents, or superseded tool errors.",
    "Potentially useful: Background context or historical discussion.",
    "Relevant: Active code, current file contents, or key user requirements.",
]
payload = {
    "state": {
        "chunk_1": "Current Task/Goal:\nFix typo in README\n\nCandidate Chunk (Tool 'read_file' result):\nLine 1: # Readme\nLine 2: Fixed typo here.",
        "chunk_2": "Current Task/Goal:\nOptimize inference latency\n\nCandidate Chunk (Tool 'run_command' result):\n" + "\n".join([f"Step {i}: executed subprocess command with stdout tensor shape [{i*16}, {i*32}] - returncode 0" for i in range(35)])
    },
    "questions": {
        "chunk_1": {"type": "score", "instructions": "Rate relevance", "criteria": relevance_criteria},
        "chunk_2": {"type": "score", "instructions": "Rate relevance", "criteria": relevance_criteria}
    },
    "metadata": {"call_site": "live_step1_retest"}
}
req_data = json.dumps(payload).encode("utf-8")

times_http = []
times_server = []

print("Running live server warmup...", flush=True)
req = urllib.request.Request("http://127.0.0.1:8177/v1/decide", data=req_data, headers={"Content-Type": "application/json"})
urllib.request.urlopen(req)

print("Running 3 measured live HTTP calls...", flush=True)
for i in range(3):
    req = urllib.request.Request("http://127.0.0.1:8177/v1/decide", data=req_data, headers={"Content-Type": "application/json"})
    t0 = time.perf_counter()
    resp = json.loads(urllib.request.urlopen(req).read().decode("utf-8"))
    elapsed = (time.perf_counter() - t0) * 1000
    times_http.append(elapsed)
    times_server.append(resp.get("latency_ms", 0))
    print(f"  Run {i+1}: HTTP = {elapsed:.1f}ms | Server = {resp.get('latency_ms', 0):.1f}ms", flush=True)

print("\n" + "="*70)
print("LIVE HTTP BENCHMARK SUMMARY (N=3):")
print("="*70)
print(f"HTTP Total:   Min = {min(times_http):.1f}ms | Median = {statistics.median(times_http):.1f}ms")
print(f"Server Infer: Min = {min(times_server):.1f}ms | Median = {statistics.median(times_server):.1f}ms")
print("="*70)
