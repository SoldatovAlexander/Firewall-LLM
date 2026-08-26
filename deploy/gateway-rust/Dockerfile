FROM rust:1.78-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY fwllm-core ./fwllm-core
COPY fwllm-gateway ./fwllm-gateway
RUN cargo build --release -p fwllm-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/fwllm-gateway /usr/local/bin/fwllm-gateway
EXPOSE 8080 8443
ENV FWLLM_CERTS_DIR=/certs
CMD ["fwllm-gateway"]
