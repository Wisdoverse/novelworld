# ─── 构建阶段 ─────────────────────────────────────────────────────────────
FROM node:26-alpine@sha256:2d984a15c9b54fd0aeb608b8e0d0d83529eb34d2966db27a1fb4f1edc3d298a3 AS builder

ARG VITE_API_URL=/api
ENV VITE_API_URL=$VITE_API_URL

WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN npm install -g pnpm@11.24.0 && pnpm install --frozen-lockfile

COPY . .
RUN pnpm build

# ─── 运行阶段（Nginx 静态服务）────────────────────────────────────────────
FROM nginx:alpine@sha256:db35bfc6b2951e7f8a72db5db120288c127ffaeeb4a6d4b95a26fead017d5913 AS runtime

RUN apk add --no-cache --upgrade libcrypto3=3.5.8-r0 libssl3=3.5.8-r0

COPY --from=builder /app/dist /usr/share/nginx/html
COPY nginx-spa.conf /etc/nginx/conf.d/default.conf

EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
