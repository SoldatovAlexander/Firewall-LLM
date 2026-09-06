# R08: pinned trixie toolchain — the ort (ONNX) prebuilt binaries require
# glibc >= 2.38, which bookworm (2.36) cannot link. Keep builder and runtime
# on the same release.
FROM rust:trixie AS builder
# R08: ort (ONNX) links C++ — the final link needs libstdc++ (cargo links
# with the cc driver, which does not pull it automatically).
RUN apt-get update && apt-get install -y --no-install-recommends g++ && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY rust/Cargo.toml rust/Cargo.lock ./
COPY rust/fwllm-core ./fwllm-core
COPY rust/fwllm-gateway ./fwllm-gateway
COPY rust/fwllm-agent ./fwllm-agent
# prefetch deps cache
RUN cargo fetch --locked
# R08: cargo links with the cc driver, which does not pull libstdc++ for
# the onnxruntime static lib — pass it explicitly.
RUN RUSTFLAGS="-C link-arg=-lstdc++" cargo build --locked --release -p fwllm-gateway

FROM debian:trixie-slim
# R08: the gateway links onnxruntime statically, but its C++ runtime
# (libstdc++/libgomp) stays dynamic — ship it or the binary won't start.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgomp1 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/fwllm-gateway /usr/local/bin/fwllm-gateway
EXPOSE 8080 8443
ENV FWLLM_CERTS_DIR=/certs
CMD ["fwllm-gateway"]
