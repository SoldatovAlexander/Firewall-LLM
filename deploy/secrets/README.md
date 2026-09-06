# Local secret files for docker compose. NOTHING HERE IS COMMITTED
# (see .gitignore). Create from the examples below on every machine:
#
#   mkdir -p deploy/secrets
#   # single line, the raw metrics scrape token (no "Bearer " prefix):
#   printf '%s' '<generate-with-openssl-rand-hex-32>' > deploy/secrets/fwllm_metrics_token
#   chmod 600 deploy/secrets/fwllm_metrics_token
#
# The token must equal one entry of FWLLM_METRICS_TOKENS in deploy/.env
# (format "token:label,..."). Prometheus reads it via bearer_token_file;
# gateways accept it ONLY on /metrics (least privilege, R15).
