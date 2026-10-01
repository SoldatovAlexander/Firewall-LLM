#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ARTIFACTS="$HERE/artifacts"
COMPOSE=(docker compose -f "$HERE/docker-compose.yml")
COMPOSE_NO_OTEL=(
  docker compose
  -f "$HERE/docker-compose.yml"
  -f "$HERE/docker-compose.no-telemetry.yml"
)
TOOLS=(python3 "$HERE/experiment_tools.py")
SENTINEL="FWLLM_PRIVACY_SENTINEL_7f3db2c5b8a14a83"

rm -rf "$ARTIFACTS"
mkdir -p "$ARTIFACTS"

cleanup() {
  "${COMPOSE[@]}" down -v >/dev/null 2>&1 || true
}
trap cleanup EXIT

trace_id_from() {
  grep '^TRACE_ID=' "$1" | tail -1 | cut -d= -f2
}

echo "=== Build and start prototype stack ==="
"${COMPOSE[@]}" up -d --build redis mock-llm jaeger otel-collector gateway-rust
"${TOOLS[@]}" wait-url http://localhost:8081/healthz --timeout 120
"${TOOLS[@]}" wait-url http://localhost:16686/ --timeout 120

echo "=== E1 / E2 / E6: distributed trace, provider correlation, privacy ==="
"${COMPOSE[@]}" run --rm   -e DEMO_MODE=normal   -e PRIVACY_SENTINEL="$SENTINEL"   demo-agent | tee "$ARTIFACTS/normal-agent.txt"
NORMAL_TRACE_ID="$(trace_id_from "$ARTIFACTS/normal-agent.txt")"
test -n "$NORMAL_TRACE_ID"
"${TOOLS[@]}" wait-trace   --trace-id "$NORMAL_TRACE_ID"   --require invoke_agent retrieval execute_tool fwllm.request fwllm.provider.request   --output "$ARTIFACTS/normal-trace.json"   --timeout 60
"${TOOLS[@]}" validate-normal   --trace "$ARTIFACTS/normal-trace.json"   --sentinel "$SENTINEL"   --output "$ARTIFACTS/e1-e2-e6.json"

echo "=== E3: explainable security block ==="
"${COMPOSE[@]}" run --rm   -e DEMO_MODE=security   demo-agent | tee "$ARTIFACTS/security-agent.txt"
SECURITY_TRACE_ID="$(trace_id_from "$ARTIFACTS/security-agent.txt")"
test -n "$SECURITY_TRACE_ID"
"${TOOLS[@]}" wait-trace   --trace-id "$SECURITY_TRACE_ID"   --require invoke_agent fwllm.request fwllm.security.inspect   --output "$ARTIFACTS/security-trace.json"   --timeout 60
"${TOOLS[@]}" validate-security   --trace "$ARTIFACTS/security-trace.json"   --output "$ARTIFACTS/e3-security.json"

# Reset in-memory routing/security state before resilience/perf experiments.
"${COMPOSE[@]}" restart gateway-rust
"${TOOLS[@]}" wait-url http://localhost:8081/healthz --timeout 90

echo "=== E4: Collector unavailable / fail-open request path ==="
"${COMPOSE[@]}" stop otel-collector
docker stats --no-stream   --format '{{json .}}'   "$("${COMPOSE[@]}" ps -q gateway-rust)"   > "$ARTIFACTS/e5-docker-stats-before.json" || true
"${TOOLS[@]}" e4 --output "$ARTIFACTS/e4-fail-open.json"

echo "=== E5: Collector-down load survival ==="
"${TOOLS[@]}" e5   --count "${E5_COUNT:-1200}"   --concurrency "${E5_CONCURRENCY:-8}"   --output "$ARTIFACTS/e5-load.json"
docker stats --no-stream   --format '{{json .}}'   "$("${COMPOSE[@]}" ps -q gateway-rust)"   > "$ARTIFACTS/e5-docker-stats-after.json" || true

echo "=== Restore Collector / gateway for E7 ==="
"${COMPOSE[@]}" start otel-collector
"${COMPOSE[@]}" up -d --force-recreate gateway-rust
"${TOOLS[@]}" wait-url http://localhost:8081/healthz --timeout 90

echo "=== E7a: telemetry enabled baseline ==="
"${TOOLS[@]}" bench   --mode telemetry_enabled   --count "${E7_COUNT:-100}"   --warmup "${E7_WARMUP:-10}"   --output "$ARTIFACTS/e7-enabled.json"

echo "=== E7b: telemetry disabled baseline ==="
"${COMPOSE_NO_OTEL[@]}" up -d --force-recreate gateway-rust
"${TOOLS[@]}" wait-url http://localhost:8081/healthz --timeout 90
"${TOOLS[@]}" bench   --mode telemetry_disabled   --count "${E7_COUNT:-100}"   --warmup "${E7_WARMUP:-10}"   --output "$ARTIFACTS/e7-disabled.json"

"${TOOLS[@]}" compare-bench   --enabled "$ARTIFACTS/e7-enabled.json"   --disabled "$ARTIFACTS/e7-disabled.json"   --output "$ARTIFACTS/e7-performance.json"

"${TOOLS[@]}" summary   --artifacts "$ARTIFACTS"   --output "$ARTIFACTS/EXPERIMENT_SUMMARY.md"

echo "=== Experiment summary ==="
cat "$ARTIFACTS/EXPERIMENT_SUMMARY.md"
