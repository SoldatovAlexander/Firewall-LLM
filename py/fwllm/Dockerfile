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

EXPOSE 8080
CMD ["uvicorn", "fwllm.main:app", "--host", "0.0.0.0", "--port", "8080"]
