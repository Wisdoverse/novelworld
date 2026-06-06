-- ═══════════════════════════════════════════════════════════════════════════
-- NovelWorld Database Schema — PostgreSQL 18
-- 设计原则：
--   - 所有 ID 使用 UUID v7（时序有序，适合分布式）
--   - 所有时间戳使用 TIMESTAMPTZ（带时区）
--   - 使用 JSONB 存储半结构化数据（记忆、世界状态、角色特征）
--   - 使用 pg_trgm 扩展支持全文模糊搜索
-- ═══════════════════════════════════════════════════════════════════════════

-- 扩展
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";
CREATE EXTENSION IF NOT EXISTS "vector";  -- pgvector，用于记忆语义搜索

-- ─── 枚举类型 ──────────────────────────────────────────────────────────────

CREATE TYPE novel_status AS ENUM ('pending', 'parsing', 'ready', 'error');
CREATE TYPE deviation_mode AS ENUM ('canon', 'creative', 'remix');
CREATE TYPE character_role AS ENUM ('protagonist', 'antagonist', 'supporting', 'minor');
CREATE TYPE avatar_status AS ENUM ('pending', 'generating', 'ready', 'error');
CREATE TYPE memory_layer AS ENUM ('short', 'mid', 'long', 'permanent');
CREATE TYPE identity_type AS ENUM ('self', 'character');
CREATE TYPE user_role AS ENUM ('user', 'admin');

-- ─── 用户表 ────────────────────────────────────────────────────────────────

CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email         VARCHAR(320) NOT NULL UNIQUE,
    password_hash VARCHAR(256) NOT NULL,
    name          VARCHAR(100),
    avatar_url    TEXT,
    role          user_role NOT NULL DEFAULT 'user',
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_sign_in  TIMESTAMPTZ
);

CREATE INDEX idx_users_email ON users(email);

-- ─── 小说表 ────────────────────────────────────────────────────────────────

CREATE TABLE novels (
    id               UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title            VARCHAR(500) NOT NULL,
    author           VARCHAR(200),
    cover_url        TEXT,
    description      TEXT,
    world_summary    TEXT,                    -- AI 生成的世界观摘要
    genre            VARCHAR(100),
    total_chapters   INTEGER NOT NULL DEFAULT 0,
    status           novel_status NOT NULL DEFAULT 'pending',
    parse_error      TEXT,
    deviation_mode   deviation_mode NOT NULL DEFAULT 'canon',
    original_file_key TEXT,                  -- S3 存储键
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_novels_user_id ON novels(user_id);
CREATE INDEX idx_novels_status ON novels(status);
CREATE INDEX idx_novels_title_trgm ON novels USING gin(title gin_trgm_ops);

-- ─── 章节表 ────────────────────────────────────────────────────────────────

CREATE TABLE chapters (
    id                     UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    novel_id               UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_number         INTEGER NOT NULL,
    title                  VARCHAR(500),
    content                TEXT NOT NULL,
    summary                TEXT,             -- AI 生成的章节摘要
    is_key_node            BOOLEAN NOT NULL DEFAULT FALSE,
    key_node_description   TEXT,             -- 关键节点描述（用于分支选择）
    word_count             INTEGER NOT NULL DEFAULT 0,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(novel_id, chapter_number)
);

CREATE INDEX idx_chapters_novel_id ON chapters(novel_id);
CREATE INDEX idx_chapters_key_node ON chapters(novel_id, is_key_node) WHERE is_key_node = TRUE;

-- ─── 角色表 ────────────────────────────────────────────────────────────────

CREATE TABLE characters (
    id                       UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    novel_id                 UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    name                     VARCHAR(200) NOT NULL,
    aliases                  TEXT[] NOT NULL DEFAULT '{}',
    role                     character_role NOT NULL DEFAULT 'supporting',
    description              TEXT,
    personality              TEXT,           -- 性格特征（用于 Agent system prompt）
    background               TEXT,           -- 背景故事
    speaking_style           TEXT,           -- 说话风格（用于 Agent system prompt）
    appearance               TEXT,           -- 外貌描述（用于头像生成）
    avatar_url               TEXT,
    avatar_status            avatar_status NOT NULL DEFAULT 'pending',
    first_appearance_chapter INTEGER,
    traits                   JSONB NOT NULL DEFAULT '{}',  -- 扩展特征
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_characters_novel_id ON characters(novel_id);
CREATE INDEX idx_characters_role ON characters(novel_id, role);
CREATE INDEX idx_characters_name_trgm ON characters USING gin(name gin_trgm_ops);

-- ─── 角色记忆表（4层金字塔）────────────────────────────────────────────────

CREATE TABLE character_memories (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    character_id  UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    layer         memory_layer NOT NULL,
    content       TEXT NOT NULL,
    importance    SMALLINT NOT NULL DEFAULT 5 CHECK (importance BETWEEN 1 AND 10),
    -- pgvector 语义向量（1536维，OpenAI text-embedding-3-small）
    embedding     vector(1536),
    access_count  INTEGER NOT NULL DEFAULT 0,
    last_accessed TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,               -- 短期记忆有过期时间
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_memories_character_user ON character_memories(character_id, user_id);
CREATE INDEX idx_memories_layer ON character_memories(character_id, user_id, layer);
CREATE INDEX idx_memories_importance ON character_memories(character_id, user_id, importance DESC);
-- 向量相似度索引（HNSW，适合高维向量）
CREATE INDEX idx_memories_embedding ON character_memories
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- ─── 对话历史表 ────────────────────────────────────────────────────────────

CREATE TABLE chat_messages (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    character_id UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id     UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    role         VARCHAR(20) NOT NULL CHECK (role IN ('user', 'character')),
    content      TEXT NOT NULL,
    chapter_num  INTEGER,                    -- 对话发生时的章节
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_chat_messages_character_user ON chat_messages(character_id, user_id, created_at DESC);
CREATE INDEX idx_chat_messages_novel_user ON chat_messages(novel_id, user_id);

-- ─── 叙事节点表 ────────────────────────────────────────────────────────────

CREATE TABLE narrative_nodes (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    novel_id       UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_number INTEGER NOT NULL,
    description    TEXT NOT NULL,
    choices        JSONB NOT NULL DEFAULT '[]',  -- NarrativeChoice[]
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_narrative_nodes_novel ON narrative_nodes(novel_id, chapter_number);

-- ─── 用户选择记录表 ────────────────────────────────────────────────────────

CREATE TABLE user_choices (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    node_id         UUID NOT NULL REFERENCES narrative_nodes(id) ON DELETE CASCADE,
    chapter_number  INTEGER NOT NULL,
    choice_index    INTEGER NOT NULL,
    choice_text     TEXT NOT NULL,
    consequence     TEXT,                    -- AI 生成的后果描述
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_choices_user_novel ON user_choices(user_id, novel_id, chapter_number);

-- ─── 世界状态表 ────────────────────────────────────────────────────────────

CREATE TABLE world_states (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id   UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    state      JSONB NOT NULL DEFAULT '{"choices":[],"relationships":{},"world_events":[]}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

CREATE INDEX idx_world_states_user_novel ON world_states(user_id, novel_id);

-- ─── 阅读进度表 ────────────────────────────────────────────────────────────

CREATE TABLE reading_progress (
    id                     UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id                UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id               UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    current_chapter        INTEGER NOT NULL DEFAULT 1,
    reader_identity        VARCHAR(200),     -- 读者自定义身份名
    reader_identity_type   identity_type NOT NULL DEFAULT 'self',
    reader_character_id    UUID REFERENCES characters(id),  -- 扮演的角色
    deviation_mode         deviation_mode NOT NULL DEFAULT 'canon',
    last_read_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

CREATE INDEX idx_reading_progress_user ON reading_progress(user_id, last_read_at DESC);

-- ─── Character Relationship Graph ────────────────────────────────────────

CREATE TABLE character_relationships (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    novel_id          UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    from_character_id UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    to_character_id   UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    relationship_type VARCHAR(50) NOT NULL,
    description       TEXT,
    strength          SMALLINT NOT NULL DEFAULT 50 CHECK (strength BETWEEN 0 AND 100),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_char_rel_novel ON character_relationships(novel_id);
CREATE INDEX idx_char_rel_from ON character_relationships(from_character_id);
CREATE INDEX idx_char_rel_to ON character_relationships(to_character_id);

-- ─── 刷新令牌表 ──────────────────────────────────────────────────────────

CREATE TABLE refresh_tokens (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token         VARCHAR(256) NOT NULL UNIQUE,
    expires_at    TIMESTAMPTZ NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_token ON refresh_tokens(token);

-- ─── 触发器：自动更新 updated_at ──────────────────────────────────────────

CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER trg_novels_updated_at
    BEFORE UPDATE ON novels
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER trg_characters_updated_at
    BEFORE UPDATE ON characters
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- ─── 视图：用户书架（含进度）─────────────────────────────────────────────

CREATE VIEW user_shelf AS
SELECT
    n.id,
    n.user_id,
    n.title,
    n.author,
    n.cover_url,
    n.genre,
    n.total_chapters,
    n.status,
    n.deviation_mode,
    n.created_at,
    n.updated_at,
    rp.current_chapter,
    rp.last_read_at,
    rp.reader_identity,
    rp.reader_identity_type,
    CASE WHEN n.total_chapters > 0
         THEN ROUND((rp.current_chapter::NUMERIC / n.total_chapters) * 100, 1)
         ELSE 0
    END AS progress_pct
FROM novels n
LEFT JOIN reading_progress rp ON rp.novel_id = n.id AND rp.user_id = n.user_id;

-- ─── 函数：语义记忆搜索 ───────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION search_memories(
    p_character_id UUID,
    p_user_id      UUID,
    p_embedding    vector(1536),
    p_limit        INTEGER DEFAULT 10,
    p_layer        memory_layer DEFAULT NULL
)
RETURNS TABLE (
    id          UUID,
    layer       memory_layer,
    content     TEXT,
    importance  SMALLINT,
    similarity  FLOAT
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        m.id,
        m.layer,
        m.content,
        m.importance,
        1 - (m.embedding <=> p_embedding) AS similarity
    FROM character_memories m
    WHERE m.character_id = p_character_id
      AND m.user_id = p_user_id
      AND (p_layer IS NULL OR m.layer = p_layer)
      AND (m.expires_at IS NULL OR m.expires_at > NOW())
    ORDER BY m.embedding <=> p_embedding
    LIMIT p_limit;
END;
$$ LANGUAGE plpgsql;
