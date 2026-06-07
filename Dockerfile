# ═══════════════════════════════════════════════════════════════════════════
# NovelWorld All-in-One Docker Image
# Usage: docker run -p 80:80 -e LLM_API_KEY=sk-xxx novelworld
# ═══════════════════════════════════════════════════════════════════════════

# ─── Stage 1: Build Rust binaries ────────────────────────────────────────
FROM rust:1.86-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.toml
COPY gateway gateway
COPY services services
COPY crates crates

RUN cargo build --release --workspace \
    --exclude integration-tests \
    && strip target/release/gateway \
    && strip target/release/user-service \
    && strip target/release/novel-service \
    && strip target/release/agent-service \
    && strip target/release/narrative-service

# ─── Stage 2: Build frontend ────────────────────────────────────────────
FROM node:22-alpine AS frontend-builder

WORKDIR /build
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN npm install -g pnpm && pnpm install --frozen-lockfile || pnpm install --no-frozen-lockfile

COPY frontend/ .
RUN pnpm build

# ─── Stage 3: Runtime ───────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    postgresql-16 \
    postgresql-16-pgvector \
    redis-server \
    nginx \
    supervisor \
    curl \
    openssl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# ─── Copy binaries ──────────────────────────────────────────────────────
COPY --from=rust-builder /build/target/release/gateway /usr/local/bin/
COPY --from=rust-builder /build/target/release/user-service /usr/local/bin/
COPY --from=rust-builder /build/target/release/novel-service /usr/local/bin/
COPY --from=rust-builder /build/target/release/agent-service /usr/local/bin/
COPY --from=rust-builder /build/target/release/narrative-service /usr/local/bin/

# ─── Copy frontend ──────────────────────────────────────────────────────
COPY --from=frontend-builder /build/dist /var/www/novelworld

# ─── Copy configs ───────────────────────────────────────────────────────
COPY infra/postgres/init.sql /docker-entrypoint-initdb.d/init.sql
COPY infra/nginx/nginx.conf /etc/nginx/nginx.conf

# ─── Nginx config for all-in-one ────────────────────────────────────────
RUN cat > /etc/nginx/conf.d/novelworld.conf << 'NGINX'
server {
    listen 80;

    location / {
        root /var/www/novelworld;
        try_files $uri $uri/ /index.html;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_buffering off;
        proxy_cache off;
    }

    location /health {
        proxy_pass http://127.0.0.1:8080;
    }

    location /metrics {
        proxy_pass http://127.0.0.1:8080;
    }
}
NGINX

# ─── Supervisor config ──────────────────────────────────────────────────
RUN cat > /etc/supervisor/conf.d/novelworld.conf << 'SUPERVISOR'
[supervisord]
nodaemon=true
logfile=/var/log/supervisor/supervisord.log
pidfile=/var/run/supervisord.pid

[program:postgres]
command=/usr/lib/postgresql/16/bin/postgres -D /var/lib/postgresql/data -c listen_addresses=127.0.0.1
user=postgres
autostart=true
autorestart=true
stdout_logfile=/var/log/postgres.log
stderr_logfile=/var/log/postgres-err.log

[program:redis]
command=redis-server --bind 127.0.0.1 --maxmemory 256mb --maxmemory-policy allkeys-lru
autostart=true
autorestart=true
stdout_logfile=/var/log/redis.log
stderr_logfile=/var/log/redis-err.log

[program:nginx]
command=nginx -g "daemon off;"
autostart=true
autorestart=true

[program:gateway]
command=/usr/local/bin/gateway
environment=PORT="8080",JWT_SECRET="%(ENV_JWT_SECRET)s",USER_SERVICE_URL="http://127.0.0.1:8001",NOVEL_SERVICE_URL="http://127.0.0.1:8002",AGENT_SERVICE_URL="http://127.0.0.1:8003",NARRATIVE_SERVICE_URL="http://127.0.0.1:8004",RUST_LOG="info"
autostart=true
autorestart=true
startsecs=3
stdout_logfile=/var/log/gateway.log
stderr_logfile=/var/log/gateway-err.log

[program:user-service]
command=/usr/local/bin/user-service
environment=PORT="8001",DATABASE_URL="postgres://novelworld:novelworld@127.0.0.1:5432/novelworld",JWT_SECRET="%(ENV_JWT_SECRET)s",RUST_LOG="info"
autostart=true
autorestart=true
startsecs=3
stdout_logfile=/var/log/user-service.log
stderr_logfile=/var/log/user-service-err.log

[program:novel-service]
command=/usr/local/bin/novel-service
environment=PORT="8002",DATABASE_URL="postgres://novelworld:novelworld@127.0.0.1:5432/novelworld",LLM_API_KEY="%(ENV_LLM_API_KEY)s",LLM_API_URL="%(ENV_LLM_API_URL)s",LLM_MODEL="%(ENV_LLM_MODEL)s",RUST_LOG="info"
autostart=true
autorestart=true
startsecs=3
stdout_logfile=/var/log/novel-service.log
stderr_logfile=/var/log/novel-service-err.log

[program:agent-service]
command=/usr/local/bin/agent-service
environment=PORT="8003",DATABASE_URL="postgres://novelworld:novelworld@127.0.0.1:5432/novelworld",REDIS_URL="redis://127.0.0.1:6379",LLM_API_KEY="%(ENV_LLM_API_KEY)s",LLM_API_URL="%(ENV_LLM_API_URL)s",LLM_MODEL="%(ENV_LLM_MODEL)s",NOVEL_SERVICE_URL="http://127.0.0.1:8002",RUST_LOG="info"
autostart=true
autorestart=true
startsecs=3
stdout_logfile=/var/log/agent-service.log
stderr_logfile=/var/log/agent-service-err.log

[program:narrative-service]
command=/usr/local/bin/narrative-service
environment=PORT="8004",DATABASE_URL="postgres://novelworld:novelworld@127.0.0.1:5432/novelworld",LLM_API_KEY="%(ENV_LLM_API_KEY)s",LLM_API_URL="%(ENV_LLM_API_URL)s",LLM_MODEL="%(ENV_LLM_MODEL)s",NOVEL_SERVICE_URL="http://127.0.0.1:8002",RUST_LOG="info"
autostart=true
autorestart=true
startsecs=3
stdout_logfile=/var/log/narrative-service.log
stderr_logfile=/var/log/narrative-service-err.log
SUPERVISOR

# ─── Entrypoint script ──────────────────────────────────────────────────
RUN cat > /entrypoint.sh << 'ENTRY'
#!/bin/bash
set -e

# Generate JWT secret if not set
export JWT_SECRET=${JWT_SECRET:-$(openssl rand -hex 32)}
export LLM_API_KEY=${LLM_API_KEY:-""}
export LLM_API_URL=${LLM_API_URL:-"https://api.openai.com"}
export LLM_MODEL=${LLM_MODEL:-"gpt-4o-mini"}

# Initialize PostgreSQL if needed
if [ ! -f /var/lib/postgresql/data/PG_VERSION ]; then
    echo "Initializing PostgreSQL..."
    su postgres -c "/usr/lib/postgresql/16/bin/initdb -D /var/lib/postgresql/data"
    su postgres -c "/usr/lib/postgresql/16/bin/pg_ctl -D /var/lib/postgresql/data start -w"
    su postgres -c "psql -c \"CREATE USER novelworld WITH PASSWORD 'novelworld';\""
    su postgres -c "psql -c \"CREATE DATABASE novelworld OWNER novelworld;\""
    su postgres -c "psql -U novelworld -d novelworld -f /docker-entrypoint-initdb.d/init.sql"
    su postgres -c "/usr/lib/postgresql/16/bin/pg_ctl -D /var/lib/postgresql/data stop -w"
    echo "PostgreSQL initialized."
fi

echo "Starting NovelWorld..."
exec /usr/bin/supervisord -c /etc/supervisor/supervisord.conf
ENTRY
chmod +x /entrypoint.sh

# ─── Volumes & Ports ────────────────────────────────────────────────────
VOLUME ["/var/lib/postgresql/data"]
EXPOSE 80

ENTRYPOINT ["/entrypoint.sh"]
