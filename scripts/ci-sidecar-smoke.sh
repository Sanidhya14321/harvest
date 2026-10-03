#!/usr/bin/env bash
# Release-gate smoke test for the Laya decision sidecar (CPU-only).
#
# Proves, on a clean host: dependency install with an explicit CPU torch
# wheel, model load of the single pinned checkpoint, /health readiness with
# model identity, one authenticated /v1/decide call, token rotation across a
# restart (old token rejected, new token accepted), and clean teardown.
#
# Usage: bash scripts/ci-sidecar-smoke.sh
# Env: LAYA_SMOKE_PORT (default 8199), LAYA_SMOKE_DIR (default: fresh mktemp dir)
#   HF_HUB_OFFLINE=1 reuses a warm HF cache with zero network (cache-only model
#   load; full-network-offline CI is intentionally not gated — see the offline
#   note in .github/workflows/release.yml). macOS note: this script avoids
#   GNU-only tools so the documented manual macOS gate runs it unmodified.
set -euo pipefail

PORT="${LAYA_SMOKE_PORT:-8199}"
WORK="${LAYA_SMOKE_DIR:-$(mktemp -d)}"
TOKEN_FILE="$WORK/laya-token"
export LAYA_TOKEN_FILE="$TOKEN_FILE"
export LAYA_LOG_DIR="$WORK/logs"
export HF_HUB_OFFLINE="${HF_HUB_OFFLINE:-0}"

cd "$(dirname "$0")/../decision-sidecar"

echo "--- installing CPU torch first so laya keeps it (torch>=2.0.0) ---"
python -m pip install --quiet --index-url https://download.pytorch.org/whl/cpu torch
python -m pip install --quiet -r requirements.txt
python -c "import torch; assert torch.version.cuda is None, torch.version.cuda; print('torch CPU wheel:', torch.__version__)"

echo "--- starting sidecar on $PORT ---"
python server.py --port "$PORT" >"$WORK/server1.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

echo "--- waiting for /health ready (model download on cold cache can take minutes) ---"
READY=""
for _ in {1..200}; do
  READY=$(curl -sf "http://127.0.0.1:$PORT/health" || true)
  if echo "$READY" | grep -q '"ready": *true'; then break; fi
  sleep 3
done
echo "$READY" | grep -q '"ready": *true' || { echo "sidecar never became ready"; cat "$WORK/server1.log"; exit 1; }
echo "$READY" | grep -q "convaiinnovations/laya-typed-decisions" || { echo "wrong model identity"; exit 1; }
test -s "$TOKEN_FILE" || { echo "no token file minted"; exit 1; }
TOKEN1=$(cat "$TOKEN_FILE")

echo "--- one authenticated /v1/decide call ---"
DECIDE_OUT="$WORK/decide.json"
curl -sf -X POST "http://127.0.0.1:$PORT/v1/decide" \
  -H "Content-Type: application/json" \
  -H "x-laya-token: $TOKEN1" \
  -d '{"state": "the bash tool ran ls on the project directory", "questions": {"risk": {"type": "noul", "instructions": "does this call write, delete, publish, or change access irreversibly?"}}, "metadata": {"call_site": "ci_smoke"}}' \
  -o "$DECIDE_OUT"
python -c "import json,sys; d=json.load(open('$DECIDE_OUT')); assert 'risk' in d['answers'], d; print('decide ok:', list(d['answers']))"

echo "--- restart: token must rotate, old token rejected ---"
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
python server.py --port "$PORT" >"$WORK/server2.log" 2>&1 &
SERVER_PID=$!
READY=""
for _ in {1..200}; do
  READY=$(curl -sf "http://127.0.0.1:$PORT/health" || true)
  if echo "$READY" | grep -q '"ready": *true'; then break; fi
  sleep 3
done
echo "$READY" | grep -q '"ready": *true' || { echo "sidecar never became ready after restart"; cat "$WORK/server2.log"; exit 1; }
TOKEN2=$(cat "$TOKEN_FILE")
test "$TOKEN1" != "$TOKEN2" || { echo "token did not rotate across restart"; exit 1; }
if curl -sf -X POST "http://127.0.0.1:$PORT/v1/decide" \
  -H "Content-Type: application/json" -H "x-laya-token: $TOKEN1" \
  -d '{"state": "x", "questions": {"risk": {"type": "noul", "instructions": "y"}}}' -o /dev/null; then
  echo "stale token still accepted"; exit 1;
fi
curl -sf -X POST "http://127.0.0.1:$PORT/v1/decide" \
  -H "Content-Type: application/json" -H "x-laya-token: $TOKEN2" \
  -d '{"state": "x", "questions": {"risk": {"type": "noul", "instructions": "y"}}}' -o /dev/null \
  || { echo "rotated token rejected"; exit 1; }

echo "--- teardown ---"
kill "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
trap - EXIT
sleep 2
if curl -sf -o /dev/null "http://127.0.0.1:$PORT/health"; then
  echo "port $PORT still bound after teardown"; exit 1;
fi
echo "sidecar smoke: ok (CPU inference, rotation, teardown)"
