# NovelWorld 部署指南

## 系统要求

| 组件 | 最低配置 | 推荐配置 |
|---|---|---|
| CPU | 2 核 | 4 核+ |
| 内存 | 4 GB | 8 GB+ |
| 磁盘 | 20 GB SSD | 50 GB SSD |
| 操作系统 | Ubuntu 22.04 / Debian 12 | Ubuntu 24.04 |
| Docker | 24.0+ | 最新稳定版 |
| Docker Compose | 2.20+ | 最新稳定版 |

---

## 快速部署（5 步）

### 第 1 步：安装 Docker

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
newgrp docker
```

### 第 2 步：克隆代码

```bash
git clone https://github.com/your-org/novel-world.git
cd novel-world
```

### 第 3 步：配置环境变量

```bash
cp .env.example .env
nano .env   # 填入以下必填项：
            # POSTGRES_PASSWORD — 数据库密码
            # REDIS_PASSWORD    — Redis 密码
            # JWT_SECRET        — 至少 32 位随机字符串
            # LLM_API_KEY       — OpenAI API Key
```

生成安全的 JWT_SECRET：
```bash
openssl rand -base64 32
```

### 第 4 步：启动所有服务

```bash
docker compose up -d --build
```

首次构建约需 5-15 分钟（Rust 编译较慢）。

### 第 5 步：验证部署

```bash
# 检查所有服务状态
docker compose ps

# 检查 Gateway 健康
curl http://localhost:8080/health

# 查看日志
docker compose logs -f gateway
```

访问 `http://your-server-ip` 即可使用。

---

## 服务端口说明

| 服务 | 内部端口 | 对外暴露 | 说明 |
|---|---|---|---|
| nginx | 80, 443 | ✅ 80, 443 | 反向代理入口 |
| gateway | 8080 | ✅ 8080（可选） | API 网关 |
| user-service | 8001 | ❌ 内部 | 用户认证 |
| novel-service | 8002 | ❌ 内部 | 小说解析 |
| agent-service | 8003 | ❌ 内部 | 角色对话 |
| narrative-service | 8004 | ❌ 内部 | 分支叙事 |
| postgres | 5432 | ✅ 5432（建议关闭） | 数据库 |
| redis | 6379 | ✅ 6379（建议关闭） | 缓存 |

**生产环境建议**：关闭 postgres 和 redis 的对外端口映射，仅通过内部网络访问。

---

## API 接口总览

所有请求通过 Gateway（`/api/`）路由：

### 用户认证
```
POST   /api/auth/register     — 注册
POST   /api/auth/login        — 登录，返回 JWT
POST   /api/auth/refresh      — 刷新 Token
GET    /api/auth/me           — 当前用户信息
```

### 小说管理
```
GET    /api/novels            — 书架列表
POST   /api/novels            — 导入小说（粘贴文本）
POST   /api/novels/upload     — 上传文件（TXT/PDF）
GET    /api/novels/:id        — 小说详情
GET    /api/novels/:id/status — 解析状态（轮询）
DELETE /api/novels/:id        — 删除小说
```

### 章节
```
GET    /api/novels/:id/chapters          — 章节列表
GET    /api/novels/:id/chapters/:num     — 章节内容
```

### 角色
```
GET    /api/novels/:id/characters        — 角色列表
GET    /api/characters/:id              — 角色详情
POST   /api/characters/:id/generate-avatar — 触发头像生成
```

### 角色对话（SSE 流式）
```
POST   /api/chat/:characterId/stream    — 流式对话（SSE）
GET    /api/chat/:characterId/history   — 对话历史
DELETE /api/chat/:characterId/history   — 清除对话历史
```

### 分支叙事
```
GET    /api/narrative/:novelId/:chapter — 获取分支节点
POST   /api/narrative/choose            — 提交选择
GET    /api/narrative/:novelId/world-state — 世界状态
```

### 阅读进度
```
GET    /api/progress/:novelId           — 阅读进度
PUT    /api/progress/:novelId           — 更新进度
PUT    /api/progress/:novelId/identity  — 设置读者身份
```

---

## 记忆系统说明（4层金字塔）

```
永久记忆 (Permanent)  ←── 核心事件，永不消失
    ↑ 重要性提升
长期记忆 (Long-term)  ←── 重要对话摘要，长期保留
    ↑ 压缩合并
中期记忆 (Mid-term)   ←── 近期对话摘要，定期压缩
    ↑ 自动摘要
短期记忆 (Short-term) ←── 原始对话记录，超出阈值后压缩
```

每次角色对话时，Agent 会：
1. 从短期记忆取最近 N 条对话
2. 用向量相似度从长期/永久记忆检索相关内容
3. 将世界状态（读者的选择历史）注入 system prompt
4. 生成符合角色人格和当前语境的回复

---

## 常见问题

**Q: Rust 编译太慢怎么办？**

A: 首次编译需要 5-15 分钟，后续增量编译很快。可以预先拉取 Rust 镜像：
```bash
docker pull rust:1.82-slim-bookworm
```

**Q: 如何更换 LLM 提供商？**

A: 修改 `.env` 中的 `LLM_API_URL` 为任意 OpenAI 兼容接口（如 Anthropic、DeepSeek、本地 Ollama）。

**Q: pgvector 扩展安装失败？**

A: PostgreSQL 18 的 pgvector 需要从源码编译。如遇问题，可将 `init.sql` 中的向量相关代码注释掉，记忆系统将退回到基于关键词的检索。

**Q: 如何备份数据库？**

```bash
docker exec novel-postgres pg_dump -U novel novel_world > backup_$(date +%Y%m%d).sql
```

---

## 生产环境加固

1. **HTTPS**：将 SSL 证书放入 `infra/nginx/certs/`，更新 `nginx.conf` 启用 443
2. **防火墙**：只开放 80/443 端口，关闭 5432/6379/8080
3. **数据库密码**：使用 `openssl rand -base64 24` 生成强密码
4. **定期备份**：设置 cron 任务每日备份 PostgreSQL
5. **监控**：可选接入 Prometheus + Grafana（gateway 暴露 `/metrics`）
