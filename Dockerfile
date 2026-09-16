# syntax=docker/dockerfile:1

# --- builder: Spin runtime + Rust + Trunk; builds both components via `spin build` ---
FROM ghcr.io/spinframework/spin:v4.1.0 AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends curl ca-certificates build-essential && \
    rm -rf /var/lib/apt/lists/*

ENV PATH="/root/.cargo/bin:${PATH}"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain stable

# Trunk (version must match .github/workflows/ci.yml)
ARG TARGETARCH
RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) trunk_arch=x86_64-unknown-linux-gnu ;; \
        arm64) trunk_arch=aarch64-unknown-linux-gnu ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    curl -sSL "https://github.com/trunk-rs/trunk/releases/download/v0.21.14/trunk-${trunk_arch}.tar.gz" \
        | tar -xz -C /usr/local/bin trunk

WORKDIR /src
COPY . .
# Runs the [component.x.build] commands from spin.toml:
#   backend  -> cargo build -p openwebide-backend --target wasm32-wasip2 --release
#   frontend -> cd frontend && trunk build --release
RUN spin build

# --- runtime: Spin + prebuilt components ---
FROM ghcr.io/spinframework/spin:v4.1.0

WORKDIR /app
COPY --from=builder /src/spin.toml ./
COPY --from=builder /src/target/wasm32-wasip2/release/openwebide_backend.wasm \
     ./target/wasm32-wasip2/release/
COPY --from=builder /src/frontend/dist ./frontend/dist

# Frontend and API share one port.
EXPOSE 3000
# SQLite data lives in .spin/ (mount a volume here to persist it).
VOLUME /app/.spin

# The base image's entrypoint is already `spin`.
CMD ["up", "--listen", "0.0.0.0:3000"]
