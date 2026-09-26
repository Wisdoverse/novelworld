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
git clone https://github.com/Wisdoverse/novelworld.git
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
国内模型 API、Coding/Token Plan 的中国版与国际版在 Settings 中分开选择；
各版本的端点、模型和 Key 说明见 [LLM 服务商配置](./docs/LLM_PROVIDERS.md)。
切换服务商、地区或套餐须重新填写 Key，套餐耗尽不会自动转到普通 API。

AI 操作返回明确的 `503 llm_not_configured`。LLM 密钥由 User Service 加密保存
或从环境变量读取，浏览器不会保存密钥。

### 显式启用 Redis 投影

在 `.env` 中同时设置 `CACHE_MODE=redis` 和至少 16 位、包含至少 8 种字符的
URL-safe `REDIS_PASSWORD`（`A-Z a-z 0-9 . _ ~ -`），再重新运行 `start.sh` 或
`start.cmd`。脚本会由该唯一 mode 同时派生 Redis profile 和 `REDIS_URL`；
缺密码、占位值、未知 mode 或只选中一半都会拒绝启动。旧版启动器已
生成 Redis 密码但没有 `CACHE_MODE` 的安装，首次运行新脚本会一次性持久化
`redis`，避免升级后静默切换适配器。

Redis 为 Agent Service 提供可重建的消息投影，不承接小说解析队列。上传接收后
持久化的 PostgreSQL 导入任务通过 claim、lease 和恢复扫描等待解析槽位；解析
繁忙不阻止有接收容量的新书入队。批量上传最多选择 50 本，浏览器按每批最多
5 本、合计 40 MiB 顺序发送。共享书库可直接添加已有 Ready 小说，避免重新
上传和解析；尚未提供自动文件哈希去重。

### 显式启用 RustFS 原文件存储

RustFS 使用已有 S3 适配器，由管理员独立部署；仓库 Compose 不会自动创建
RustFS 容器或 bucket。为其固定镜像 digest、配置独立持久化数据卷，并让
Novel Service 通过共同的私有 Docker 网络访问。无需向宿主机发布 S3 或
控制台端口；应用凭据与管理员 root 凭据分开保管。

例如容器网络别名为 `novel-rustfs`、S3 端口为 `9000` 时，在私有 `.env` 设置：

```dotenv
S3_ENABLED=true
S3_ENDPOINT=http://novel-rustfs:9000
S3_BUCKET=novel-world-uploads
S3_REGION=us-east-1
S3_FORCE_PATH_STYLE=true
```

先创建 bucket，再从本地秘密配置填写配套的 `S3_ACCESS_KEY` 与
`S3_SECRET_KEY`；长期应用账户的 `S3_SESSION_TOKEN` 留空。此 HTTP 示例仅用于
同主机私有容器网络；跨主机连接须提供受信任 TLS 边界。不要在容器里使用
`127.0.0.1` 指向另一容器，也不要提交 `.env` 或在命令输出中展示凭据。

应用账户仅需 bucket 上的 `s3:ListBucket`，以及该 bucket 的 `source-files/*`
对象上的 `s3:GetObject`、`s3:PutObject`、`s3:DeleteObject`。`ListBucket` 用于
`HeadBucket` readiness，不能加上只适用于列举对象的前缀条件而阻断 HEAD。
RustFS 服务账户继承父账户权限：给父 IAM 用户附加上述最小权限策略，或在创建
派生服务账户时附带限制性的 session policy；不要把服务账户当作独立 IAM 用户
直接绑定策略。须检查最终生效权限，见
[官方 IAM 说明](https://docs.rustfs.com/en/security-compliance/iam)。

按受管部署流程重建 Novel Service 后，验证其 `/ready`、用应用账户执行
HEAD 和一次私有测试对象的 PUT/GET/DELETE，并确认前缀外对象与其他 bucket
访问被拒绝。不得用真实小说解析来代替存储探针。启用只保留之后上传的原文件，
不会为历史上传补齐源对象；原文件保留不等于自动去重。已保留源对象的恢复、
删除仍需要此 bucket，停用前需明确这些任务的处理方式。PostgreSQL 备份不含
RustFS 数据卷，管理员须单独安排对象存储备份与恢复，见
[备份边界](./docs/BACKUP_RESTORE.md#scope)。

---

## D20 基础规则迁移 0030

0030 改变 Novel Service 的模板写入契约，旧 Novel writer 与新 schema 不兼容。
升级时先关闭入口写入并停止、排空 Novel 和 Narrative 两个服务；不能只停
Narrative。候选 release 必须包含完整的 0021、0024、0025、0030 四个 required
schema barriers；应用完整候选 release 后再启动兼容版本，避免旧 writer 在迁移后写入。现有 v1 profiles 和 sessions 保留原绑定，继续按 v1 读取；
不要重写为 v2。数据库只前向迁移，旧版 rollback 不受支持，故障恢复使用兼容
release 前向修复。此要求是 rollout 契约，不代表该迁移已在生产执行。

## 生产升级与回滚

`v*` Tag workflow 会先运行完整 CI，只发布以 Git SHA 标记的应用镜像。
全部镜像和 Windows/Linux/macOS 客户端构建成功后，同一 GitHub Release
会附加 `release.env`、SBOM、客户端压缩包、`desktop-SHA256SUMS` 和
`release-attestation.json`。发布文件的逐文件 provenance 验证流程见
[`SECURITY.md#release-file-provenance`](SECURITY.md#release-file-provenance)。
`release.env` 只允许版本、代码 SHA、六个应用镜像和三个经源码审批的
基础镜像 digest；不要部署单个镜像或使用 `latest`。

消费者必须使用独立复核取得的 source/signer SHA，并独立取得 trusted roots；不能
只信 `release.env` 或随 release 附带的 root。当前 `release.sh` 仍不会自动执行
provenance 或 deploy-time SBOM admission，相关 H2 证据仍待完成。

PostgreSQL、Redis、Nginx 的 digest 固定在 `docker-compose.yml`。普通应用发布
要求候选与当前 release 的三个 digest 完全一致，总是检查 PostgreSQL，
只在 `CACHE_MODE=redis` 时检查 Redis，
不会重建或降级它们。基础镜像变更必须作为独立基础设施变更，先完成数据库备份、
格式兼容与恢复演练；本脚本会拒绝把它混入应用发布。

Redis 7 到 8 是一次冷切换，不是普通 `release.sh upgrade`。本次切换只适用于
尚无用户的默认 Unix 预览环境。`CACHE_MODE=redis` 的受管安装使用以下一次性
流程；不要 `source` manifest：

```bash
set -euo pipefail
target=/absolute/path/to/redis8-release.env
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
archive_root="$(dirname "$repo_root")/novelworld-release-archive"
install -d -m 700 "$archive_root"
./infra/docker/release.sh validate "$target"
docker compose --env-file .env --env-file .release/current.env --profile redis \
  stop nginx gateway agent-service redis
redis_volume=$(docker inspect --format \
  '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Name}}{{end}}{{end}}' novel-redis)
redis_project=$(docker inspect --format \
  '{{ index .Config.Labels "com.docker.compose.project" }}' novel-redis)
test -n "$redis_volume" && test -n "$redis_project"
test "$(docker volume inspect --format \
  '{{ index .Labels "com.docker.compose.volume" }}' "$redis_volume")" = redis_data
test "$(docker volume inspect --format \
  '{{ index .Labels "com.docker.compose.project" }}' "$redis_volume")" = "$redis_project"
docker compose --env-file .env --env-file .release/current.env --profile redis rm -f redis
archive="$archive_root/redis7.$(date -u +%Y%m%dT%H%M%SZ)"
test ! -e "$archive"
mv .release "$archive"
sync
docker volume rm "$redis_volume"
unset REDIS_IMAGE COMPOSE_PROFILES
docker compose --env-file .env --env-file "$target" --profile redis pull redis
docker compose --env-file .env --env-file "$target" --profile redis up -d --wait redis
./infra/docker/release.sh adopt "$target"
```

仓库外的 `.release` 归档是删除卷前的单向提交点；之后不得恢复其中的 Redis 7 manifest。
`adopt` 会拒绝运行中 Redis 与目标 exact image 不一致的情况。失败时保持入口关闭
并只向 Redis 8 重试。不得使用 `down -v`，PostgreSQL 卷必须保留。
`CACHE_MODE=postgres` 没有运行中的 Redis，只需停入口和应用、归档旧 release
状态并重新 `adopt`。新安装直接采用 Redis 8。任何已有用户、Windows 原地升级
或需保留 Redis 投影的环境都不在本次批准范围内，必须另行设计并验证迁移。

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
包含 prompt、URL、错误正文、用户或小说标识。设置页按当前 Key 从 Prometheus
查询近 30 天增量，以随构建发布、带官网来源和核验日期的价格快照重估已报告
计费类别。中国区人民币、国际区美元分别计算，无需汇率；峰谷/上下文/思考
档位缺少逐次信息时显示区间。未知单价不视为免费；Coding/Token Plan 单列
套餐月费报价，不由累计 token 推算实付费用或剩余额度。

`LLM_PRICING_USD_PER_MILLION` 和 `LLM_PRICING_CNY_PER_MILLION` 可显式覆盖
普通 API 的精确 provider/model 单价，同键冲突或套餐 token 单价使启动失败。
`USD_CNY_RATE` 仅用于旧版点估算字段的可选换算。报价核验、更新流程、未统计
费用和回滚边界见 [LLM 计费估算](./docs/LLM_PRICING.md)。这些设置不改变冻结
Diagnostic 的价格、预算或证据。

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

**Q: 如何启用 Laya (Jev) 行动建议和 D20 裁定？**

A: 两者共用可选的 Laya (Jev) decision 服务，但用途不同：行动建议只分类
玩家显式请求的动作类型；配置成对的 `LAYA_API_URL` 与 `LAYA_API_KEY` 后，
高级 D20 回合还会发送受限的行动/角色上下文进行可行性与难度分类。该上下文
不含小说全文、历史、骰子或结果，仍可能包含玩家背景、能力、库存和目标显示名；
服务商的数据保留由其策略决定。裁定错误、低置信度或上下文超限会回退到模板
DC，服务器校验和骰子仍是权威，语义质量尚未认证。它不替代 DeepSeek 故事生成。
让 Laya 服务与
`narrative-service` 处于同一条私有 Docker 网络，在 `.env` 中填写
`LAYA_API_URL=http://<Laya容器名>:8000` 和 `LAYA_API_KEY`，然后重建并重启
Narrative。地址必须从 Narrative 容器内可达；宿主机的
`127.0.0.1:18881` 在容器内指向 Narrative 自己。只使用私有网络，
不要公开 Narrative 或 Laya 端口。未同时配置 URL 和密钥时，界面不显示
建议入口；Laya 失效时建议功能仍可手动选行动，高级检定则回退到模板 DC。

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
