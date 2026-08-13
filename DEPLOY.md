# NovelWorld 部署指南

## 系统要求

| 组件 | 最低配置 | 推荐配置 |
|---|---|---|
| CPU | 2 核 | 4 核+ |
| 内存 | 4 GB | 8 GB+ |
| 磁盘 | 20 GB SSD | 50 GB SSD |
| 操作系统 | Windows 10/11、Ubuntu 22.04、Debian 12 | Windows 11、Ubuntu 24.04 |
| Docker | 24.0+ | 最新稳定版 |
| Docker Compose | 2.20+ | 最新稳定版 |

---

## 快速部署

### 第 1 步：安装 Docker

Windows 请安装并启动 [Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/)。
Linux 可运行：

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
newgrp docker
```

### 第 2 步：克隆代码

```bash
git clone https://github.com/schorsch888/novelworld.git
cd novelworld
```

### 第 3 步：一条命令启动

```bash
# Linux
./start.sh
```

Windows 在命令提示符运行，或在资源管理器中双击：

```bat
start.cmd
```

脚本会生成数据库、Redis、JWT、配置加密和服务间鉴权密钥，构建并启动
全部容器，然后打开 `http://localhost`。请在网页中选择 DeepSeek 或
OpenAI、填写 API Key，并创建第一个管理员。API Key 不会保存在浏览器中。

首次构建约需 5-15 分钟（Rust 编译较慢）。

### 第 4 步：验证部署

```bash
# 检查所有服务状态
docker compose ps

# 通过公开 Nginx 入口检查聚合健康
curl --fail http://localhost/health
curl --fail http://localhost/ready

# 查看日志
docker compose logs -f gateway
```

访问 `http://your-server-ip` 即可使用。
首次访问会要求配置模型并创建唯一的首位管理员。生产 schema 不再安装默认应用
账号；LLM 密钥由服务端加密保存或从环境变量读取，浏览器不会保存密钥。

---

## 生产升级与回滚

Tag workflow 会先运行完整 CI，只发布以 Git SHA 标记的应用镜像，并生成
`novel-world-release-<git-sha>` artifact。`release.env` 只允许版本、代码 SHA、
六个应用镜像和三个经源码审批的基础镜像 digest；不要部署单个镜像或使用
`latest`。

PostgreSQL、Redis、Nginx 的 digest 固定在 `docker-compose.yml`。普通应用发布
要求候选与当前 release 的三个 digest 完全一致，只检查 PostgreSQL/Redis 健康，
不会重建或降级它们。基础镜像变更必须作为独立基础设施变更，先完成数据库备份、
格式兼容与恢复演练；本脚本会拒绝把它混入应用发布。

自动回滚只适用于新格式 release。如果当前环境已经使用九镜像契约，首次采用前先
验证一份与实际代码和镜像完全一致的基线：

```bash
./infra/docker/release.sh validate /path/to/current-release.env
install -d -m 700 .release
install -m 600 /path/to/current-release.env .release/current.env
```

旧版 all-in-one 环境无法建立上述基线。此时必须先保留并验证旧源码 bundle、精确
镜像和数据库备份的人工恢复路径，再执行一次性采用；按提示输入
`ADOPT-<release-git-sha>`。采用成功只建立 current，不伪造 previous，因此自动
rollback 要到下一次成功 upgrade 后才可用：

```bash
./infra/docker/release.sh adopt /path/to/artifact/release.env
```

升级时把 artifact 解压到临时目录。脚本使用 `set -euo pipefail`，严格拒绝额外、
重复或缺失字段，从不执行或 `source` manifest；Compose 同时读取生产 `.env` 和
已验证的候选 manifest。候选只有在全部 readiness 成功后才原子提升为
`.release/current.env`，旧 current 才成为 previous：

```bash
./infra/docker/release.sh upgrade /path/to/artifact/release.env
```

脚本先更新兼容前端并暂停。确认章节切换成功返回
`PUT /api/progress/:novelId`，设置状态在旧后端上明确显示暂不可用，且聊天请求携带
UUID v4 `Idempotency-Key` 后，输入脚本
显示的代码 SHA；随后才会执行迁移和后端更新。旧标签页会收到
`426 client_upgrade_required`，不会以请求体中的过期章节启动 LLM。新 Agent 只在
同一事务写入一对消息并完成 turn；`done` 是数据库提交确认。

升级中途失败时 current/previous 不会被覆盖。恢复最后成功的 current：

```bash
./infra/docker/release.sh restore
```

成功上线后若需回滚，数据库 migration 保持前向兼容且不执行 down migration；脚本
部署 previous，通过 readiness 后再交换 current/previous：

```bash
./infra/docker/release.sh rollback <previous-release-git-sha>
```

---

## 服务端口说明

| 服务 | 内部端口 | 对外暴露 | 说明 |
|---|---|---|---|
| nginx | 80 | ✅ 80 | 反向代理入口 |
| gateway | 8080 | ❌ 默认关闭（可选 8080） | API 网关 |
| user-service | 8001 | ❌ 内部 | 用户认证 |
| novel-service | 8002 | ❌ 内部 | 小说解析 |
| agent-service | 8003 | ❌ 内部 | 角色对话 |
| narrative-service | 8004 | ❌ 内部 | 分支叙事 |
| postgres | 5432 | ❌ 默认关闭（可选 5432） | 数据库 |
| redis | 6379 | ❌ 默认关闭（可选 6379） | 缓存 |

**生产环境建议**：关闭 postgres 和 redis 的对外端口映射，仅通过内部网络访问。

---

## API 接口总览

所有请求通过 Gateway（`/api/`）路由：

### 用户认证
```
GET    /api/setup/status     — 管理员与模型设置是否完成
POST   /api/setup/init       — 验证模型并原子创建首次设置（仅空库）
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

## LLM 指标与发布预算

`user-service`、`novel-service`、`agent-service` 和 `narrative-service`
在各自的内部端口暴露 `/metrics`。公网 Nginx 对 `/metrics` 返回 404；由
内部 Prometheus 网络直接抓取服务端口。

指标按受控的 `service/provider/model/operation/mode/status` 标签记录逻辑
请求、实际 provider 尝试、重试、延迟、首 token、usage 缺失、输入/输出/
缓存命中 token，以及 cached-input/uncached-input/output 计费 token。指标不
包含 prompt、URL、错误正文、用户或小说标识。美元成本应在查询时用当前
provider 价格乘计费 token；代码不内置会过期的价格表。

H3 发布样本使用版本化策略校验：

```bash
python3 tools/llm-budget/verify.py \
  --policy tools/llm-budget/policy-v1.json \
  --metrics release-sample.prom \
  --commit "$(git rev-parse HEAD)"
```

样本必须来自一个完成的、有边界的发布测试窗口；进程重启会重置计数器。
校验器会拒绝缺服务、缺 operation、usage 缺失、未完成请求、未知/敏感标签、
超出重试/错误/延迟/首 token/计费 token 预算或 provider 实际 token 上限的样本。

---

## 常见问题

**Q: Rust 编译太慢怎么办？**

A: 首次编译需要 5-15 分钟，后续增量编译很快。可以预先拉取 Rust 镜像：
```bash
docker pull rust:1.82-slim-bookworm
```

**Q: 如何更换 LLM 提供商？**

A: 新安装可在网页向导选择 DeepSeek 或 OpenAI。高级部署可在启动前设置
`.env` 中的 `LLM_API_URL`、`LLM_API_KEY` 和 `LLM_MODEL`，然后重建三个
LLM 服务；环境配置优先于网页设置。

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
5. **监控**：从内部网络抓取各服务 `/metrics`；公网 Nginx 保持 404
