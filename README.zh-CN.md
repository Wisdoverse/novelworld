<div align="center">

# NovelWorld

[English](./README.md) · [简体中文](./README.zh-CN.md)

**读一本小说，与角色对话，亲自决定故事的下一步。**

[![CI](https://github.com/Wisdoverse/novelworld/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Wisdoverse/novelworld/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![阶段：私有预览](https://img.shields.io/badge/阶段-私有预览-orange)](./docs/PRODUCT_CONTRACT.md)

[快速开始](#快速开始) · [功能](#功能) · [平台支持](#平台支持) · [开发](#开发) · [文档](#文档)

</div>

NovelWorld 是一个开源、可自托管的 AI 互动小说平台：导入自己的小说，按章节阅读，与书中提取出的角色聊天，或创建原创玩家进入故事世界，通过行动和选择探索新的故事走向。

> [!NOTE]
> **项目正在快速迭代和测试中。** 当前版本是面向私有环境的单节点预览版，模型质量、故障恢复和无障碍验证仍在推进。支持范围和已知限制请查看[产品契约](./docs/PRODUCT_CONTRACT.md)与[路线图](./docs/ROADMAP.md)。本文面向读者与自托管用户；功能、安装方式或支持范围变化时，应与[英文 README](./README.md)同步更新。

## 为什么选择 NovelWorld？

- **使用自己的书。** 粘贴文本，或导入 TXT、EPUB 和可提取文本的 PDF。
- **从阅读走向互动。** 与角色对话、选择剧情分支，或以原创玩家身份在小说世界中行动。
- **保留旅程。** 已提交的对话、选择和世界回合存储在 PostgreSQL 中，方便继续体验。
- **自行选择部署和模型。** 部署私有 Docker 服务，或尝试便携桌面版；使用 AI 功能时可配置 DeepSeek 或 OpenAI 兼容服务。

## 快速开始

### 服务端：Linux 或 Windows

安装 Git 和带有 Compose v2 的 Docker。Windows 用户请先启动 [Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/)。主机要求见[部署指南](./DEPLOY.md#系统要求)。

克隆仓库：

```bash
git clone https://github.com/Wisdoverse/novelworld.git
cd novelworld
```

**Linux：**

```bash
./start.sh
```

**Windows PowerShell：**

```powershell
.\start.cmd
```

也可以双击项目目录中的 `start.cmd`。

启动器会引导初始数据库配置、生成必要密钥，自动重启一次后构建应用。服务就绪后，打开 **http://localhost**。

1. 创建第一个管理员账号，此步骤不需要模型 API 密钥。
2. 打开「设置」，配置模型服务商、模型和 API 密钥。
3. 导入小说，等待解析完成后开始阅读。
4. 打开角色对话、选择剧情分支，或在已解锁章节创建原创玩家进入世界。

默认使用 PostgreSQL，Redis 为可选组件。预览版应在本机使用；远程访问前需配置加密私有网络或 TLS。升级、备份、可选 Redis 和故障排查见 [DEPLOY.md](./DEPLOY.md)，升级可能需要维护窗口。

### 桌面端：实验性便携版本

从 [GitHub Releases](https://github.com/Wisdoverse/novelworld/releases) 获取对应版本的便携包，解压后运行下表中的程序。包内包含前端、五个 Rust 服务和本地 PostgreSQL，无需 Docker 或外部 NovelWorld 服务端。

| 平台 | 压缩包 | 启动程序 |
|---|---|---|
| Windows 10/11 x64 | `novelworld-windows-x64-portable.zip` | `NovelWorld.exe` |
| Linux x64 | `novelworld-linux-x64-appimage.tar.gz` | 解压后的 AppImage |
| macOS Apple Silicon | `novelworld-macos-arm64-app.zip` | `NovelWorld.app` |
| macOS Intel | `novelworld-macos-x64-app.zip` | `NovelWorld.app` |

这些是未签名的工程构建，数据库迁移只支持向前执行。请保持应用与数据版本兼容，不要使用旧版本打开新版本创建的数据。AI 功能仍需联网，并使用配置的模型服务和密钥。

## 功能

| 使用场景 | 当前功能 |
|---|---|
| 书架与导入 | 单本或限量批量导入；从共享目录添加已解析小说，同时保留各自私有的阅读进度和旅程。 |
| 阅读与翻译 | 按章节阅读、记录进度，按需将当前章节转换为简体中文。 |
| 角色对话 | 流式聊天并继续已保存的对话；可用设定与记忆受服务端记录的阅读进度约束。 |
| 分支故事 | 在分支点选择后续走向，查看已提交的结果。 |
| 开放世界互动 | 在已解锁检查点创建原创玩家，旅行、调查、交谈、结盟或对抗；时间线区分玩家决定与生成文本。 |
| 模型设置 | 部署后配置平台模型服务；登录用户也可选择使用个人加密保存的服务商密钥。 |

<p align="center">
  <img src="./docs/evidence/h4-chat-landscape.png" width="568" alt="NovelWorld 中文阅读界面，在较窄的横屏视口中展示角色聊天面板和消息输入框。" />
</p>

*截图来自合成浏览器测试场景。当前界面使用简体中文。*

### 输入与语言支持

| 输入方式 | 当前接收限制 |
|---|---|
| 粘贴文本 | 5 MiB |
| TXT | 10 MiB；UTF-8、带 BOM 的 UTF-16 或 GBK |
| EPUB 或可提取文本的 PDF | 每个文件 20 MiB；提取后的文本不超过 20 MiB |
| 批量上传 | 最多 5 个文件、合计 40 MiB；仍受单文件限制 |

简体中文和英文具有确定性的结构测试覆盖。生成的叙事过渡目前要求使用中文。不支持扫描版、纯图片 PDF 或受 DRM 保护的文件。成功接收文件不代表模型提取或翻译质量已通过验证，目前没有任何语言与模型组合完成发布资格验证。

### 平台支持

| 部署方式 | Windows | Linux | macOS |
|---|---|---|---|
| Docker 服务端 | `start.cmd` | `./start.sh` | 尚未完成验证 |
| 便携桌面端 | x64 工程构建 | x64 AppImage 工程构建 | Apple Silicon 与 Intel 工程构建 |

完整的兼容性、隐私、来源可见性与恢复边界见[产品契约](./docs/PRODUCT_CONTRACT.md)。使用者需要具有处理导入书籍的权限，并了解模型服务商的数据和计费政策；相关原文片段与对话会发送给配置的模型服务商。

## 架构

浏览器中的 React 应用通过网关访问五个 Rust/Axum 服务。PostgreSQL 保存权威状态，Redis 是可选的缓存投影。

| 层级 | 技术与职责 |
|---|---|
| 前端 | React、TypeScript、Tailwind CSS，采用 Feature-Sliced Design |
| 后端 | 五个 Rust/Axum 服务，采用 DDD 分层并通过 HTTP 通信 |
| 数据 | PostgreSQL 18 与 pgvector；可选 Redis 投影 |
| 模型集成 | OpenAI 兼容请求、SSE 流式聊天和有界重试 |
| 部署 | Docker Compose 服务端或实验性 Tauri 桌面包 |

当前为共享数据库的私有 `single-node-v1` 拓扑。静态架构检查约束代码与数据表归属边界；数据库级隔离、水平扩展与公有云就绪不在当前支持声明内。详见[架构与证据限制](./docs/ARCHITECTURE.md#code-boundaries)。

## 开发

依赖要求、本地配置与按改动范围执行的检查见 [CONTRIBUTING.md](./CONTRIBUTING.md)。常用命令：

```bash
cargo test -p novel-service
cargo run --locked -p architecture-check -- check
```

```bash
cd frontend
pnpm install --frozen-lockfile
pnpm dev
```

Vite 前端运行在 `http://localhost:5173`，需要可用的后端网关。前端架构改动应运行 `pnpm lint:fsd`；其他检查遵循[验证指南](./CONTRIBUTING.md#verification)，CI 是权威合并门禁。

编码代理修改仓库前应阅读 [AGENTS.md](./AGENTS.md)，`CLAUDE.md` 是它的符号链接。

## 文档

| 想了解什么 | 从这里开始 |
|---|---|
| 安装、升级与故障排查 | [部署指南](./DEPLOY.md) |
| 当前能力与限制 | [产品契约](./docs/PRODUCT_CONTRACT.md) |
| 产品方向与进行中的工作 | [路线图](./docs/ROADMAP.md) · [GitHub Projects](https://github.com/Wisdoverse/novelworld/projects) |
| 服务边界与数据归属 | [架构](./docs/ARCHITECTURE.md) |
| 预期行为与实现证据 | [规格](./SPEC.md) · [符合性记录](./docs/SPEC_CONFORMANCE.md) |
| 数据保留、导出与删除 | [数据生命周期](./docs/DATA_RETENTION.md) · [账号导出](./docs/ACCOUNT_EXPORT.md) |
| 全部工程与运维文档 | [文档索引](./docs/README.md) |

## 参与贡献

欢迎提交问题报告、范围明确的修复、文档改进和可复现测试结果。请先阅读[贡献指南](./CONTRIBUTING.md)、搜索[现有 Issue](https://github.com/Wisdoverse/novelworld/issues)，并提供复现步骤。安全漏洞请按 [SECURITY.md](./SECURITY.md) 报告。

## 许可证

[MIT](./LICENSE)
