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

服务端可选择下方的直接启动方式，或使用 [Jenkins 服务端部署](#jenkins-服务端部署可选)。
Jenkins 仅是可选的服务器部署入口；本地启动、桌面版和其他部署方式由用户自行选择。

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

## 同一世界剧情来源迁移 0036

0036 新增 Narrative 所有的来源操作记录和可空的回合来源坐标，不重写旧世界
JSON。首次明确进入下一幕后，session 使用 schema v2 和独立 active source context；
原始 entry、角色、规则和旧回合保留。使用现有 managed release 流程停止旧
Narrative，再发布客户端，停止并 drain Novel/Agent，执行 migration 后启动匹配版本。
新 UI 只能配合支持来源扩展的 Novel/Narrative 使用。

release 的五个 barriers 必须齐全。即使 0036 SQL 是 additive，也不能在世界扩展后
恢复旧 Narrative reader；rollback/marked restore 拒绝跨过 0036。保留数据库与操作
key，通过兼容版本向前恢复，不能删除 source_context、重置角色或手写 JSON 回退。
读取进度后退时只隐藏后续派生内容，不能删除已接入来源。
参见 [ADR 0013](docs/adr/0013-same-world-source-progression.md)。

---

## Jenkins 服务端部署（可选）

仓库根目录的 [`Jenkinsfile`](./Jenkinsfile) 在 Linux 私有单节点服务器上构建并自动
部署六个应用镜像。代码、架构、测试和秘密扫描检查由现有 GitHub Actions CI 负责；触发
Jenkins 前，应确认选用的受信任分支或提交已通过 CI。每次成功构建都会运行现有
`start.sh`，随后执行 `infra/ops/health-checks.sh`，确认部署 readiness 和部署后健康状态。
部署依赖 Job 默认 workspace 中预先配置且跨构建保留的 `.env`；缺少时，镜像构建前
即失败关闭。Compose 使用该 `.env` 构建和启动，不使用示例配置。

1. 在目标 Linux 部署服务器安装 Git、Bash、OpenSSL、curl，以及带 BuildKit 的
   Docker 和 Compose v2。Jenkins agent 用户必须能访问该服务器的 Docker daemon。
   Jenkins 主机无需安装 Rust、Node.js、pnpm 或 Python；镜像所需的 Rust、Node.js
   与 pnpm 工具链由 Dockerfiles 在容器内提供。
2. 创建 **Pipeline script from SCM** 作业，选择本仓库的受信任分支，Script Path 为
   `Jenkinsfile`；需要 Jenkins Pipeline、Git 和 Timestamper 插件。此作业必须专用于
   受信任分支，不运行外部 PR。由于流水线使用 `agent any`，所有符合条件的 executor
   都必须位于同一 Linux 部署服务器并使用同一 Docker daemon。Pipeline 使用 Jenkins
   分配给该 Job 的默认 workspace；确保它在该服务器上跨构建保持不变。
   部署前应确认所选受信任分支或提交已通过 GitHub Actions CI；Jenkins 只负责镜像
   构建和部署。
3. 在首次构建前，于 Job 的默认 workspace 内私下配置 `.env`：可复制 `.env.example`，填写有效的
   `POSTGRES_USER`、`POSTGRES_DB` 和满足启动器要求的强 `POSTGRES_PASSWORD`，
   设置 `BOOTSTRAP_L0_COMPLETE=true`，权限设为 `600`。启动器会生成缺少的 L1 根；
   之后应保留这些值，LLM 可稍后在设置页配置。Redis 仍由 `CACHE_MODE` 决定。
   不要启用 SCM 清理未跟踪文件，也不要清理 Job 的默认 workspace；`.env` 中的数据库
   密码和启动根必须跨构建保留。不要把 `.env` 放入 Git、Jenkins 参数、日志或构建归档。
4. 确认已通过 GitHub Actions CI，准备维护窗口和已验证的数据库备份后，运行 Job。
   Jenkins 调用 `start.sh` 先构建镜像；构建失败时旧容器仍保持运行。构建成功后才停止
   旧 writer，再以不重复构建的方式启动完整栈并重放迁移。Compose 等待 readiness 后再
   通过入口与容器健康检查。每次部署会造成停机；配置不完整、构建失败或健康检查失败
   都会使作业失败。

已有源码安装接入前，先在私有 `.env` 中设置与该安装完全相同的
`COMPOSE_PROJECT_NAME`（当前安装为 `novel-world`），并保留原有的
`POSTGRES_USER`、`POSTGRES_DB`、`POSTGRES_PASSWORD`、`BOOTSTRAP_L0_COMPLETE`、
`JWT_SECRET`、`RUNTIME_CONFIG_KEY` 和 `INTERNAL_SERVICE_TOKEN`。这样 PostgreSQL 数据卷
仍由原 Compose project 管理，启动根也保持不变；不要执行数据卷迁移或重新初始化。
新建安装可不设置 `COMPOSE_PROJECT_NAME`，Compose 会沿用 Job 默认 workspace 的名称，
该 workspace 必须跨构建稳定。若 Jenkins agent 在容器中，确保 Docker daemon 可访问
到相同 workspace 路径，使构建上下文和 Compose bind mounts 指向正确位置。
健康检查默认访问 `http://127.0.0.1:80`；若该地址不能从 Jenkins agent 到达，可在私有
`.env` 配置未加引号的 `NGINX_URL=http://...`，指向该 agent 可访问的部署入口。该 URL
只用于 Jenkins 发起的部署后探测。每个服务器仍只配置一个作业，流水线禁止并发运行。
服务端仍默认仅在 localhost 访问，远程访问使用既有加密边界。

此入口沿用直接启动器的源码安装流程，没有自动回滚。遇到失败先排查迁移和健康
状态，不自动重跑或删除数据卷。存在 `.release` 的受管安装在检出前会被拒绝，仍按
下方的不可变 `release.env` 升级、确认与恢复流程处理；不可用源码启动绕过该契约。

---

## D20 基础规则迁移 0030

0030 改变 Novel Service 的模板写入契约，旧 Novel writer 与新 schema 不兼容。
升级时先关闭入口写入并停止、排空 Novel 和 Narrative 两个服务；不能只停
Narrative。候选 release 必须包含完整的 0021、0024、0025、0030、0036 五个 required
schema barriers；应用完整候选 release 后再启动兼容版本，避免旧 writer 在迁移后写入。现有 v1 profiles 和 sessions 保留原绑定，继续按 v1 读取；
不要重写为 v2。数据库只前向迁移，旧版 rollback 不受支持，故障恢复使用兼容
release 前向修复。此要求是 rollout 契约，不代表该迁移已在生产执行。

每次受管部署都会重放标准迁移目录。导入可在章节可用前原子创建默认
`reading_progress`；`current_chapter = 1` 只是元数据，不证明该章节存在。
迁移 0002 对 `pending`、`parsing`、`error` 且在公布章节数内没有有效章节的小说
保留进度及其偏好；`ready` 或状态缺失/未知而没有有效章节时必须失败关闭，且只可
按现有有效章节规范化。升级前先完成并验证 PostgreSQL 备份，不要手动删除或改写
进度记录来绕过迁移错误。当前一键 `start.sh` 会先构建所选 profile 的镜像；若构建失败，
旧容器仍保持运行。构建成功后才执行 `docker compose down`，再用 `up --no-build` 启动完整
栈并重放迁移；它不是只重开入口的命令。

迁移 0019 只在首次引入 `user_novels` 关系时回填上传者书架：它在创建关系之前记录该表
是否已存在，并在事务内锁定 `novels`。若表原已存在（即使为空），重放保留现有书架，
不会把已移出的书重新挂回上传者。此检查用于区分首次采用与重放，不会自动修复任意
历史部分迁移。`tests/integration/tests/legacy_migration.rs` 中的全迁移重放回归覆盖显式移除
后的保留行为；Issue #424 记录最终 CI 与 live 部署证据。

若受管停机后只需重开已经存在的 Nginx 容器，本机 Compose `start --help` 不提供
`--no-deps`；确认只返回一个 Nginx 容器 ID 后再使用：

```bash
nginx_id=$(docker compose ps --all -q nginx)
test -n "$nginx_id" && test "$(printf '%s\n' "$nginx_id" | wc -l)" -eq 1 && docker start "$nginx_id"
```

## 生产升级与回滚

Rust 原生镜像的源码新鲜度属于构建阶段契约，不是部署脚本可补救的事项。
构建必须在现有锁定的 BuildKit Cargo target cache 内只清理 workspace release
产物（`cargo clean --locked --release --workspace`），保留第三方已编译依赖及
registry/layer cache，不清理宿主机共享 Cargo target。历史 git archive 的源码
mtime 可能让 Cargo 复用其他 revision 编译的 workspace 二进制；Docker `RUN`
成功本身不能证明二进制对应当前源码。发布证据必须证明 Cargo 实际编译了变更源码。
对注册候选 release pair，B 的真实 runtime 变更至少须使一个受影响应用镜像的
image ID 和至少一个 filesystem layer 均与基线不同；仅文档/缓存变化或未改变的
runtime 输入不要求每个镜像或二进制都变化。当前 #432 与 #429 的交付顺序是 A→C→B；
只有实际合并的 C 基线和其严格后继 B 候选、且两端都含该保护，才可参与
后续 artifact pair 资格验证。旧 A 基线和失败预览产物须保留为失败证据，且不合格。

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
`novel-service`、`narrative-service` 处于同一条私有 Docker 网络，在 `.env` 中填写
`LAYA_API_URL=http://<Laya容器名>:8000` 和 `LAYA_API_KEY`，然后重建并重启 Novel 与 Narrative，以启用 D20 裁定和系列匹配。地址必须分别从
Novel 和 Narrative 容器内可达；宿主机的
`127.0.0.1:18881` 在容器内指向 Narrative 自己。只使用私有网络，
不要公开 Novel、Narrative 或 Laya 端口。未同时配置 URL 和密钥时，行动
建议入口隐藏，系列识别返回未配置并保留手动关联。系列推荐需用户确认；
Laya 失效时仍可手动选择系列或行动，高级检定回退到模板 DC。系列绑定需要
匹配的 Novel、Narrative 和前端版本；0031 和 0033 经正常受管停机迁移执行，且不新增
发布屏障。0033 允许先创建仅共享背景的系列；D20 规则由用户以后显式生成，
来源书的就绪规则只会固定一次。来源规则未就绪时，系列成员不能用各自的规则
代替。绑定状态产生后，旧版本回滚不受支持。

**Q: 如何独立升级 NovelWorld 的 Laya？**

A: 可叠加 `docker-compose.laya.yml`，使用包含 `laya==0.3.20` 和已审查的
multilingual checkpoint 的不可变镜像。在根 `.env` 设置 `LAYA_IMAGE` 为镜像
digest（本机测试也可用精确 image ID）、`LAYA_MODEL_PATH` 为镜像内固定 revision
的本地 checkpoint 路径、`LAYA_API_URL=http://laya:8000`，保持既有密钥。先验证
镜像包版本、checkpoint 内容和权限；不要使用浮动 `latest`。该 endpoint 不下载
模型，不发布宿主机端口，只加载 multilingual，限制 CPU/内存/线程，并对会截断
state 的请求返回 422；问题头部和选项仍使用上游 token 预算。升级不会自动提升
识别准确率或改变 0.8 弃权阈值。

```bash
docker compose -f docker-compose.yml -f docker-compose.laya.yml --profile laya up -d laya
```

验证健康、鉴权以及系列/行动/D20 协议兼容后，按正常部署流程更新 Novel 与
Narrative 的 URL；保留原共享服务和其他消费者。回滚只恢复这两个消费者原来的
URL 并重建，随后可停止本项目独立 Laya。不要删除共享模型缓存或其他项目容器。

系列识别默认只调用本地 Laya。用户可主动点击 DeepSeek 补判，使用设置页当前
配置的 DeepSeek API 与现有计费路径；其他 provider 或 Diagnostic 绑定会被拒绝。
相同证据/候选/策略/模型复用 PostgreSQL 结果。每条 claim 最多一次实际请求，输出上限为 512 tokens（启用思考时包含思考 tokens），
不做 HTTP 重试、JSON fallback 或修复重发；未知结果也不自动重试。“查询结果”使用只读的 `check_only=true` 模式；证据或
模型配置改变时，也不会把查询变成新的付费请求。0032 缓存迁移
必须先于新版 Novel 启动，缓存不会改已有系列或 D20 绑定。本功能测试和部署不
构成执行新的付费 Diagnostic 的授权。

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

### Community series suggestions (migration 0035)

Apply additive migration 0035 before starting the matching Novel/frontend pair;
normal managed migrations and desktop startup include it. Existing series do
not contribute until their owner opts in. No provider credential or additional
configuration is needed. Rolling back the frontend/API keeps the additive table
and all prior private backgrounds, rules and frozen sessions intact; no down
migration is required. See [ADR 0012](docs/adr/0012-opt-in-community-series.md).
