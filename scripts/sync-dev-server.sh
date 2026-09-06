#!/bin/bash
# Sync the release tree to the dev-server WITHOUT clobbering server-local
# files. Lesson learned 2026-09-06: a bare `rsync ./ host:dir/` overwrote
# the server's deploy/.env and deploy/fwllm.yaml with local smoke versions
# (wrong tokens, wrong quotas) and took down auth until noticed.
# Server-local (never synced): .env, fwllm.yaml, secrets/, certs/, data*/,
# models/, audit.db files.
set -eu
HOST="${FWLLM_DEV_HOST:-alex@192.168.88.101}"
DIR="${FWLLM_DEV_DIR:-~/fwllm-0.1.0}"
cd "$(dirname "$0")/.."
rsync -az \
  --exclude=.venv --exclude=.venv-metal \
  --exclude=rust/target \
  --exclude=__pycache__ --exclude=.git \
  --exclude=.mypy_cache --exclude=.ruff_cache --exclude=.pytest_cache \
  --exclude=docs/notebooks \
  --exclude=deploy/.env \
  --exclude=deploy/fwllm.yaml \
  --exclude=deploy/secrets/ \
  --exclude=deploy/certs/ \
  --exclude=deploy/data/ \
  --exclude=deploy/data-rust/ \
  --exclude=deploy/models/ \
  --exclude='*.db' --exclude='*.db-wal' --exclude='*.db-shm' \
  ./ "$HOST:$DIR/"
echo "SYNCED (server-local files untouched)"
