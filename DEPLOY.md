# NovelWorld 部署指南

> 当前支持边界是管理员控制的私有单节点自托管预览。默认仅限 localhost；任何
> 非本机访问必须由管理员提供加密隧道或 TLS 边界。
> 默认栈没有通过公网托管所需的 TLS、CORS、滥用治理、内容政策、法务和持续运维
> 资格审查。完整边界见 [产品合同](./docs/PRODUCT_CONTRACT.md)。

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

首次交互启动会引导确认 L0 所需的 PostgreSQL 用户名和数据库名，自动生成数据库
密码，最后写入 `BOOTSTRAP_L0_COMPLETE=true`，并在任何容器启动前自动重启
启动器一次。有效的旧版或预配置 `.env` 会无提示迁移；未完成配置的非交互启动
会明确失败并要求预置 `POSTGRES_USER`、`POSTGRES_DB` 和强密码。

重启后脚本只自动生成 JWT、配置加密和服务间鉴权所需的 L1 启动根。
新安装默认持久化 `CACHE_MODE=postgres`，不生成 Redis 密码，也不启动 Redis。
脚本使用 Compose `--wait`，只在整个选定 profile 达到 readiness 后才打开
`http://localhost`。先创建唯一的首位管理员；DeepSeek/OpenAI 可稍后在设置页
配置，API Key 不会保存在浏览器中。

首次构建约需 5-15 分钟（Rust 编译较慢）。

### 第 4 步：验证部署

```bash
# 检查基础 profile 状态
docker compose ps

# 通过公开 Nginx 入口检查聚合健康
curl --fail http://localhost/health
curl --fail http://localhost/ready

# 查看日志
docker compose logs -f gateway

# 按 .env 中的 CACHE_MODE 检查；Redis 仅在 redis 模式中是必需健康项
./infra/ops/health-checks.sh
```

默认仅在本机通过 `http://localhost` 访问；非本机访问必须先增加加密传输边界。
首次访问只需创建唯一的首位管理员。未配置 LLM 时基础服务仍可 ready，
AI 操作返回明确的 `503 llm_not_configured`。LLM 密钥由 User Service 加密保存
或从环境变量读取，浏览器不会保存密钥。

### 显式启用 Redis 投影

在 `.env` 中同时设置 `CACHE_MODE=redis` 和至少 16 位、包含至少 8 种字符的
URL-safe `REDIS_PASSWORD`（`A-Z a-z 0-9 . _ ~ -`），再重新运行 `start.sh` 或
`start.cmd`。脚本会由该唯一 mode 同时派生 Redis profile 和 `REDIS_URL`；
缺密码、占位值、未知 mode 或只选中一半都会拒绝启动。旧版启动器已
生成 Redis 密码但没有 `CACHE_MODE` 的安装，首次运行新脚本会一次性持久化
`redis`，避免升级后静默切换适配器。

---

## 生产升级与回滚

`v*` Tag workflow 会先运行完整 CI，只发布以 Git SHA 标记的应用镜像。
全部镜像和 Windows/Linux/macOS 客户端构建成功后，同一 GitHub Release
会附加 `release.env`、SBOM、客户端压缩包和 `desktop-SHA256SUMS`。
`release.env` 只允许版本、代码 SHA、六个应用镜像和三个经源码审批的
基础镜像 digest；不要部署单个镜像或使用 `latest`。

PostgreSQL、Redis、Nginx 的 digest 固定在 `docker-compose.yml`。普通应用发布
要求候选与当前 release 的三个 digest 完全一致，总是检查 PostgreSQL，
只在 `CACHE_MODE=redis` 时检查 Redis，
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

回滚到本最小启动决策之前的 release 会在替换任何服务前拒绝，除非：
`CACHE_MODE=redis`、Redis 密码能认证健康容器；并且有有效的 LLM 环境覆盖，
或仍运行且未使用环境覆盖的当前 User Service 能以内部身份访问既有运行时 LLM
端点。守卫只检查 HTTP 状态并丢弃响应体，不读取或输出密钥/密文。

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
| redis | 6379 | ❌ 默认不启动（`redis` profile） | 可选重建投影 |

**生产环境建议**：关闭 postgres 和 redis 的对外端口映射，仅通过内部网络访问。

---

## API 契约

部署文档不重复维护接口清单。当前规范见 [SPEC §10](./SPEC.md#10-api-contract)，
当前支持范围见 [PRODUCT_CONTRACT.md](./docs/PRODUCT_CONTRACT.md)。

---

## LLM 指标与发布预算

`user-service`、`novel-service`、`agent-service` 和 `narrative-service`
在各自的内部端口暴露 `/metrics`。公网 Nginx 对 `/metrics` 返回 404；由
内部 Prometheus 网络直接抓取服务端口。

指标按受控的 `service/provider/model/operation/mode/status` 标签记录逻辑
请求、实际 provider 尝试、重试、延迟、首 token、usage 缺失、输入/输出/
缓存命中 token，以及 cached-input/uncached-input/output 计费 token。指标不
包含 prompt、URL、错误正文、用户或小说标识。成本在查询时用
`LLM_PRICING_USD_PER_MILLION` 中当前 provider/model 价格乘计费 token；
代码不内置会过期的价格表。设置页的管理员统计卡从 Prometheus 查询近 30 天
增量：页面语言以 `zh` 开头时使用配置的 `USD_CNY_RATE` 显示人民币，其他
语言显示美元。未配置价格或汇率时仍显示 token，并明确标出未定价部分。

H3 发布样本使用版本化策略校验：

```bash
python3 tools/llm-budget/verify.py \
  --policy tools/llm-budget/policy-v2.json \
  --metrics release-sample.prom \
  --commit "$(git rev-parse HEAD)"
```

样本必须来自一个完成的、有边界的发布测试窗口；进程重启会重置计数器。
`h3-llm-budget-v2` 使用 3 个 30 分钟 summary 分桶，使任一样本至少保留 60 分钟
（最多 90 分钟），覆盖 45 分钟发布任务上限；
若已启动 operation 的 output-token-limit 窗口为空，校验器会失败而不是把零当作优秀结果。
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

A: 创建首位管理员后，可在受保护的设置页选择 DeepSeek 或 OpenAI。高级部署
可在启动前设置 `.env` 中的 `LLM_API_URL`、`LLM_API_KEY` 和 `LLM_MODEL`，
然后重新运行启动器；环境配置优先于数据库设置并在网页中只读显示。

**Q: pgvector 扩展安装失败？**

A: pgvector 是当前 schema 的必需扩展。不要修改 `init.sql` 绕过它；修复镜像或扩展
安装后重新执行迁移。项目目前不承诺无 pgvector 的生产降级模式。

**Q: 如何备份数据库？**

```bash
docker exec novel-postgres pg_dump -U novel novel_world > backup_$(date +%Y%m%d).sql
```

以上示例未加密、未做完整性校验，也未经恢复演练验证。已批准的恢复目标
（RPO/RTO）、加密与完整性要求、保留上限和擦除重放契约见
[`docs/BACKUP_RESTORE.md`](docs/BACKUP_RESTORE.md)；配套的脚本化备份/恢复
工具随该政策的实现变更交付。

---

## 私有部署安全基线

1. **网络**：保持在 localhost；非本机访问使用管理员维护的加密隧道或 TLS，且不要
   把默认 Nginx 直接暴露到公网。
2. **端口**：只开放私网入口，关闭 5432/6379/8080 的外部映射。
3. **密钥**：使用启动脚本生成的随机值并限制 `.env` 文件权限。
4. **备份**：制定并演练 PostgreSQL、S3 和操作员日志的恢复与删除策略；示例
   `pg_dump` 命令不是已验证的 RPO/RTO。
5. **监控**：从内部网络抓取各服务 `/metrics`；Nginx 继续对公网路径返回 404。

公网托管需要 H2 的独立安全、隐私、内容、滥用、供应链、TLS/CORS 和恢复审查；
上述基线不能替代该资格门槛。
