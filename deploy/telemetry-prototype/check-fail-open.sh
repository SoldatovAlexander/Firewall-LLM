#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE=(docker compose -f "$HERE/docker-compose.yml")
TMP_RESPONSE="$(mktemp)"
KEEP_STACK="${KEEP_STACK:-0}"

cleanup() {
  rm -f "$TMP_RESPONSE"
  if [[ "$KEEP_STACK" != "1" ]]; then
    "${COMPOSE[@]}" down -v >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "[1/5] Starting prototype dependencies and gateway..."
"${COMPOSE[@]}" up -d --build redis mock-llm jaeger otel-collector gateway-rust

echo "[2/5] Waiting for gateway health..."
for _ in $(seq 1 60); do
  if curl -fsS "http://localhost:8081/healthz" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://localhost:8081/healthz" >/dev/null

echo "[3/5] Stopping OpenTelemetry Collector..."
"${COMPOSE[@]}" stop otel-collector

echo "[4/5] Sending request while Collector is unavailable..."
HTTP_CODE="$(
  curl -sS     -o "$TMP_RESPONSE"     -w "%{http_code}"     -X POST "http://localhost:8081/v1/chat/completions"     -H "Authorization: Bearer prototype-token"     -H "Content-Type: application/json"     --data '{"model":"prototype-model","messages":[{"role":"user","content":"fail-open smoke test"}],"stream":false}'
)"

if [[ "$HTTP_CODE" != "200" ]]; then
  echo "FAIL: expected HTTP 200 with Collector down, got $HTTP_CODE"
  cat "$TMP_RESPONSE"
  exit 1
fi

if ! grep -q '"choices"' "$TMP_RESPONSE"; then
  echo "FAIL: response does not contain an OpenAI-compatible choices payload"
  cat "$TMP_RESPONSE"
  exit 1
fi

echo "[5/5] Re-checking gateway health..."
curl -fsS "http://localhost:8081/healthz" >/dev/null

echo "PASS: gateway request path remains available with Collector stopped."
echo "This is a smoke test; PR-5 records formal E4 evidence and timing."
