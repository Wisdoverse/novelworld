# ─── 构建阶段 ─────────────────────────────────────────────────────────────
FROM rust:1.96-slim-bookworm AS builder

# 安装系统依赖
RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev libpq-dev curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 先复制 Cargo 文件以利用 Docker 层缓存
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# 复制源码并构建
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ─── 运行阶段 ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 libpq5 curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/novel-service /app/novel-service

# 非 root 用户运行
RUN useradd -r -s /bin/false appuser && chown -R appuser:appuser /app
USER appuser

EXPOSE 8002

HEALTHCHECK --interval=15s --timeout=5s --retries=3 \
    CMD curl -f http://localhost:8002/health || exit 1

CMD ["/app/novel-service"]
