# Novel World — 系统架构文档

> 沉浸式 AI 互动小说平台 · Rust 微服务 + PostgreSQL 18 + FSD 前端

---

## 一、整体架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                        Client (Browser)                         │
│              React 19 + TypeScript + FSD Architecture           │
└───────────────────────────┬─────────────────────────────────────┘
                            │ HTTPS
┌───────────────────────────▼─────────────────────────────────────┐
│                    Nginx Reverse Proxy                          │
│              TLS Termination · Static Assets CDN                │
└──────┬──────────────┬──────────────┬──────────────┬────────────┘
       │              │              │              │
┌──────▼──────┐       │              │              │
│   Gateway   │       │              │              │
│  (Axum)     │       │              │              │
│  Port 8080  │       │              │              │
└──────┬──────┘       │              │              │
       │ gRPC / HTTP  │              │              │
┌──────▼──────┐ ┌─────▼──────┐ ┌────▼──────┐ ┌────▼──────┐
│   novel-    │ │   agent-   │ │narrative- │ │  user-    │
│   service   │ │   service  │ │  service  │ │  service  │
│  Port 50051 │ │ Port 50052 │ │Port 50053 │ │Port 50054 │
└──────┬──────┘ └─────┬──────┘ └────┬──────┘ └────┬──────┘
       │              │              │              │
┌──────▼──────────────▼──────────────▼──────────────▼──────────┐
│                    PostgreSQL 18                               │
│   novels · chapters · characters · memories · messages        │
│   reading_progress · story_choices · users                    │
└───────────────────────────────────────────────────────────────┘
       │
┌──────▼──────┐
│    Redis    │  短期记忆缓存 · 会话状态 · SSE 连接管理
│  Port 6379  │
└─────────────┘
```

---

## 二、微服务职责划分

| 服务 | 端口 | 职责 | 核心 Domain |
|---|---|---|---|
| `gateway` | 8080 | API 聚合、JWT 鉴权、限流、路由转发 | — |
| `novel-service` | 50051 | 小说导入解析、章节拆分、角色提取、世界观构建 | Novel, Chapter, Character |
| `agent-service` | 50052 | 角色 Agent 对话、4层记忆金字塔、流式 SSE | Agent, Memory, Conversation |
| `narrative-service` | 50053 | 分支叙事引擎、关键节点生成、世界状态持久化 | NarrativeNode, Choice, WorldState |
| `user-service` | 50054 | 用户注册登录、阅读进度、身份设置 | User, ReadingProgress, Identity |

---

## 三、DDD 分层结构（每个微服务统一）

```
service-name/
├── Cargo.toml
└── src/
    ├── main.rs                    # 启动入口
    ├── domain/                    # 领域层（纯业务逻辑，无框架依赖）
    │   ├── mod.rs
    │   ├── entities/              # 聚合根 & 实体
    │   ├── value_objects/         # 值对象
    │   ├── repositories/          # 仓储接口（trait）
    │   ├── services/              # 领域服务
    │   └── events/                # 领域事件
    ├── application/               # 应用层（用例编排，调用 domain）
    │   ├── mod.rs
    │   ├── commands/              # 写操作命令
    │   ├── queries/               # 读操作查询
    │   └── handlers/              # 命令/查询处理器
    ├── infrastructure/            # 基础设施层（DB、外部 API、缓存）
    │   ├── mod.rs
    │   ├── persistence/           # PostgreSQL 实现（sqlx）
    │   ├── cache/                 # Redis 实现
    │   ├── llm/                   # LLM API 客户端
    │   └── messaging/             # 事件总线（可选）
    └── interface/                 # 接口层（HTTP/gRPC handler）
        ├── mod.rs
        ├── http/                  # REST/SSE handlers (Axum)
        └── grpc/                  # gRPC handlers (tonic)
```

---

## 四、FSD 前端架构

```
frontend/src/
├── app/                           # 应用层：路由、Provider、全局初始化
│   ├── providers/                 # ThemeProvider, AuthProvider, QueryProvider
│   ├── router/                    # 路由定义（React Router v7）
│   └── styles/                    # 全局样式、宇宙美学主题变量
│
├── pages/                         # 页面层：仅组合 widgets，无业务逻辑
│   ├── home/                      # 首页 Landing
│   ├── bookshelf/                 # 书架页
│   ├── import/                    # 小说导入页
│   ├── reader/                    # 阅读器页（章节+对话）
│   ├── characters/                # 角色卡片页
│   └── profile/                   # 用户设置页
│
├── widgets/                       # 组件块层：独立可复用的 UI 块
│   ├── reader-panel/              # 章节阅读器面板
│   ├── chat-panel/                # 角色对话侧边面板
│   ├── character-card/            # 角色卡片（头像+信息+标签）
│   ├── branch-selector/           # 分支选择弹窗
│   ├── novel-card/                # 书架小说卡片
│   └── navbar/                    # 顶部导航栏
│
├── features/                      # 功能层：具体业务功能切片
│   ├── import-novel/              # 小说导入（上传/粘贴/解析轮询）
│   ├── chat-with-character/       # 与角色对话（流式 SSE）
│   ├── make-story-choice/         # 做出分支选择
│   ├── set-reader-identity/       # 设置读者身份（自己/扮演角色）
│   ├── update-reading-progress/   # 更新阅读进度
│   └── generate-avatar/           # 触发头像生成
│
├── entities/                      # 实体层：业务数据模型 + API 调用
│   ├── novel/                     # Novel 实体（类型、API、store）
│   ├── character/                 # Character 实体
│   ├── chapter/                   # Chapter 实体
│   ├── memory/                    # Memory 实体
│   └── user/                      # User 实体
│
└── shared/                        # 共享层：纯工具，无业务依赖
    ├── ui/                        # 基础 UI 组件（Button, Card, Input...）
    ├── api/                       # API 客户端（fetch wrapper, SSE client）
    ├── lib/                       # 工具函数（cn, formatDate...）
    ├── config/                    # 环境变量、常量
    └── types/                     # 全局类型定义
```

**FSD 核心规则：**
- 层级只能向下依赖（app → pages → widgets → features → entities → shared）
- 同层切片之间禁止直接导入，通过 public API（index.ts）暴露
- 每个切片内部结构：`ui/`、`model/`、`api/`、`lib/`、`index.ts`

---

## 五、PostgreSQL 18 数据库 Schema

```sql
-- 用户表
CREATE TABLE users (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email       VARCHAR(320) UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    name        VARCHAR(256),
    avatar_url  TEXT,
    role        VARCHAR(32) NOT NULL DEFAULT 'user',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 小说表
CREATE TABLE novels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title           VARCHAR(512) NOT NULL,
    author          VARCHAR(256),
    cover_url       TEXT,
    description     TEXT,
    world_summary   TEXT,
    genre           VARCHAR(128),
    file_key        TEXT,
    total_chapters  INT NOT NULL DEFAULT 0,
    status          VARCHAR(32) NOT NULL DEFAULT 'pending',
    parse_error     TEXT,
    -- 故事偏离度（借鉴 KathaaVerse）
    deviation_mode  VARCHAR(32) NOT NULL DEFAULT 'canon',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 章节表
CREATE TABLE chapters (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    novel_id            UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_number      INT NOT NULL,
    title               VARCHAR(512),
    content             TEXT NOT NULL,
    summary             TEXT,
    is_key_node         BOOLEAN NOT NULL DEFAULT FALSE,
    key_node_description TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(novel_id, chapter_number)
);

-- 角色表
CREATE TABLE characters (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    novel_id                UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    name                    VARCHAR(256) NOT NULL,
    aliases                 TEXT,
    role                    VARCHAR(32) NOT NULL DEFAULT 'supporting',
    description             TEXT,
    personality             TEXT,
    background              TEXT,
    speaking_style          TEXT,
    appearance              TEXT,
    avatar_url              TEXT,
    avatar_status           VARCHAR(32) NOT NULL DEFAULT 'pending',
    system_prompt           TEXT,
    first_appearance_chapter INT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 对话消息表
CREATE TABLE chat_messages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    character_id    UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    role            VARCHAR(16) NOT NULL,  -- 'user' | 'character'
    content         TEXT NOT NULL,
    reader_identity VARCHAR(256),
    chapter_context INT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 角色记忆表（4层记忆金字塔）
CREATE TABLE character_memories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    character_id    UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    memory_type     VARCHAR(32) NOT NULL,  -- 'short'|'mid'|'long'|'permanent'
    content         TEXT NOT NULL,
    importance      INT NOT NULL DEFAULT 5,
    chapter_number  INT,
    embedding       vector(1536),          -- pgvector 长期记忆向量
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 阅读进度表
CREATE TABLE reading_progress (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id                 UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id                UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    current_chapter         INT NOT NULL DEFAULT 1,
    reader_identity         VARCHAR(256),
    reader_identity_type    VARCHAR(16) NOT NULL DEFAULT 'self',
    reader_character_id     UUID REFERENCES characters(id),
    deviation_mode          VARCHAR(32) NOT NULL DEFAULT 'canon',
    last_read_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

-- 故事选择表
CREATE TABLE story_choices (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_id      UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
    choice_index    INT NOT NULL,
    choice_text     TEXT NOT NULL,
    consequence     TEXT,
    chosen_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 世界状态表（parallel-ai-engine 思路：持久化世界状态）
CREATE TABLE world_states (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id    UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    state_data  JSONB NOT NULL DEFAULT '{}',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

-- 索引
CREATE INDEX idx_novels_user_id ON novels(user_id);
CREATE INDEX idx_chapters_novel_id ON chapters(novel_id);
CREATE INDEX idx_characters_novel_id ON characters(novel_id);
CREATE INDEX idx_chat_messages_user_char ON chat_messages(user_id, character_id, novel_id);
CREATE INDEX idx_memories_user_char ON character_memories(user_id, character_id, novel_id);
CREATE INDEX idx_memories_embedding ON character_memories USING ivfflat (embedding vector_cosine_ops);
CREATE INDEX idx_reading_progress_user ON reading_progress(user_id);
CREATE INDEX idx_world_states_user_novel ON world_states(user_id, novel_id);
```

---

## 六、核心技术选型

| 层次 | 技术 | 选型理由 |
|---|---|---|
| Web 框架 | Axum 0.8 | 高性能、类型安全、Tower 中间件生态 |
| 异步运行时 | Tokio 1.x | Rust 事实标准异步运行时 |
| 数据库 ORM | sqlx 0.8 | 编译期 SQL 检查，零成本抽象 |
| gRPC | tonic 0.12 | Rust 官方 gRPC 实现 |
| 序列化 | serde + serde_json | 标准选择 |
| 认证 | jsonwebtoken 9 | JWT RS256 签名 |
| 向量搜索 | pgvector (PostgreSQL 扩展) | 长期记忆语义检索 |
| 缓存 | redis-rs + deadpool-redis | 短期记忆、SSE 连接管理 |
| LLM 客户端 | reqwest (HTTP) | 调用 OpenAI 兼容 API |
| 前端框架 | React 19 + TypeScript | 生态成熟 |
| 前端路由 | React Router v7 | 文件系统路由 |
| 状态管理 | Zustand + TanStack Query | 轻量全局状态 + 服务端状态 |
| 样式 | Tailwind CSS v4 | 原子化 CSS |
| 动画 | Framer Motion | 宇宙美学动效 |
| 构建工具 | Vite 7 | 极速 HMR |
| 容器化 | Docker + Docker Compose | 一键部署 |
| 反向代理 | Nginx | TLS 终止、静态资源 |
