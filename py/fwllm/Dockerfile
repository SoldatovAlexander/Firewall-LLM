FROM python:3.12-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY py/fwllm/pyproject.toml ./
COPY py/fwllm/src ./src
COPY py/fwllm-enterprise ./enterprise
# rtcompat: CPU-compatible polars build for servers without AVX2
RUN pip install --no-cache-dir "polars[rtcompat]" \
    && pip install --no-cache-dir onnxruntime tokenizers \
    && pip install --no-cache-dir . ./enterprise

ENV POLARS_SKIP_CPU_CHECK=1

# 0.1.1 capacity: pre-fork workers (UVICORN_WORKERS, default 4) so one
# CPU-bound request cannot stall the loop for everyone. Prometheus
# multiprocess mode aggregates per-worker mmap files from a container-local
# dir (fresh every start — never a mounted volume, or stale worker files
# would inflate counters after unclean shutdowns).
ENV PROMETHEUS_MULTIPROC_DIR=/tmp/prom-multiproc
RUN mkdir -p /tmp/prom-multiproc

EXPOSE 8080
CMD ["sh", "-c", "exec uvicorn fwllm.main:app --host 0.0.0.0 --port 8080 --workers ${UVICORN_WORKERS:-4}"]
