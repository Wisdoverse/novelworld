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

CREATE TABLE novel_import_jobs (
    novel_id         UUID PRIMARY KEY REFERENCES novels(id) ON DELETE CASCADE,
    stage            VARCHAR(16) NOT NULL DEFAULT 'source',
    status           VARCHAR(16) NOT NULL DEFAULT 'pending',
    attempt          BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TIMESTAMPTZ,
    failure_code     VARCHAR(64),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT novel_import_jobs_stage_check
        CHECK (stage IN ('source', 'chapters', 'enriched', 'completed')),
    CONSTRAINT novel_import_jobs_status_check
        CHECK (status IN ('pending', 'in_progress', 'failed', 'completed')),
    CONSTRAINT novel_import_jobs_attempt_check CHECK (attempt >= 0),
    CONSTRAINT novel_import_jobs_failure_code_check CHECK (
        failure_code IS NULL OR char_length(failure_code) BETWEEN 1 AND 64
    ),
    CONSTRAINT novel_import_jobs_state_check CHECK (
        (status = 'pending' AND attempt = 0 AND lease_expires_at IS NULL
            AND failure_code IS NULL AND stage <> 'completed')
        OR (status = 'in_progress' AND attempt >= 1 AND lease_expires_at IS NOT NULL
            AND failure_code IS NULL AND stage <> 'completed')
        OR (status = 'failed' AND lease_expires_at IS NULL
            AND failure_code IS NOT NULL AND stage <> 'completed')
        OR (status = 'completed' AND lease_expires_at IS NULL
            AND failure_code IS NULL AND stage = 'completed')
    )
);

CREATE INDEX idx_novel_import_jobs_recoverable
    ON novel_import_jobs(status, lease_expires_at, created_at)
    WHERE status IN ('pending', 'in_progress');

-- S3 删除 outbox 不使用外键，确保账户级联删除小说后仍保留清理证据。
CREATE TABLE source_file_deletions (
    object_key      TEXT PRIMARY KEY CHECK (
        object_key LIKE 'source-files/%'
        AND octet_length(object_key) BETWEEN 1 AND 1024
    ),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error      VARCHAR(500),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_source_file_deletions_due
    ON source_file_deletions(next_attempt_at, object_key);

CREATE OR REPLACE FUNCTION queue_source_file_deletion()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    IF OLD.original_file_key LIKE 'source-files/%'
       AND pg_catalog.octet_length(OLD.original_file_key) BETWEEN 1 AND 1024 THEN
        INSERT INTO public.source_file_deletions (object_key)
        VALUES (OLD.original_file_key)
        ON CONFLICT (object_key) DO UPDATE
        SET next_attempt_at = LEAST(
            public.source_file_deletions.next_attempt_at,
            EXCLUDED.next_attempt_at
        );
    END IF;
    RETURN OLD;
END
$function$;

CREATE TRIGGER queue_source_file_deletion
    AFTER DELETE ON novels
    FOR EACH ROW EXECUTE FUNCTION queue_source_file_deletion();

-- ─── 删除凭证（backup-restore-v1）────────────────────────────────────────────
-- 用户或小说删除时，在同一事务内写入只含 UUID 的删除凭证；不使用外键，
-- 因此账户级联无法删掉自己的删除证据。迁移路径上的重放依赖这两张表。

CREATE TABLE erasure_records (
    subject_type       VARCHAR(8) NOT NULL,
    subject_id         UUID NOT NULL,
    -- 保留归属用户，使 source-files/{user_id}/{novel_id} 在小说行消失后仍可重建。
    user_id            UUID NOT NULL,
    erased_at          TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    -- 删除时由触发器观察 OLD.original_file_key：只有权威删除能看到对象是否存在，
    -- 该事实随记录一起流转，重放无需依赖部署状态推测。
    had_source         BOOLEAN NOT NULL DEFAULT FALSE,
    -- 每条记录一次的来源对象重新入队记账（自消费的 outbox 不能充当记账）。
    -- 记账与 outbox 行同事务写入，转储又是同一快照，因此恢复后保留记账不会漏删对象，
    -- 重复次数为零，仍在策略允许的「每次恢复至多重复一次」范围内。
    source_requeued_at TIMESTAMPTZ,
    CONSTRAINT erasure_records_pkey PRIMARY KEY (subject_type, subject_id),
    CONSTRAINT erasure_records_subject_type_check
        CHECK (subject_type IN ('user', 'novel'))
);

CREATE TABLE restore_attestations (
    id                 UUID PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    subject_id         UUID NOT NULL,
    decision           VARCHAR(8) NOT NULL,
    window_start       TIMESTAMPTZ NOT NULL,
    window_end         TIMESTAMPTZ NOT NULL,
    artifact_inventory TEXT NOT NULL,
    operator_identity  TEXT NOT NULL,
    -- 决定会使安装失去管理员时，操作者显式指定的留存账户在此标记为 true。
    designated_admin   BOOLEAN NOT NULL DEFAULT FALSE,
    recorded_at        TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT restore_attestations_decision_check
        CHECK (decision IN ('retain', 'erase'))
);

CREATE OR REPLACE FUNCTION record_user_erasure()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    INSERT INTO public.erasure_records (subject_type, subject_id, user_id)
    VALUES ('user', OLD.id, OLD.id)
    ON CONFLICT (subject_type, subject_id) DO NOTHING;
    RETURN OLD;
END
$function$;

-- had_source 只能从 false 升为 true：记录可能先于主体行被写入（恢复流程先写决定），
-- 而只有删除本身能看到来源对象是否存在。
CREATE OR REPLACE FUNCTION record_novel_erasure()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    INSERT INTO public.erasure_records (subject_type, subject_id, user_id, had_source)
    VALUES ('novel', OLD.id, OLD.user_id,
            -- COALESCE is core grammar, not a schema-resolved function.
            COALESCE(OLD.original_file_key LIKE 'source-files/%', FALSE))
    ON CONFLICT (subject_type, subject_id) DO UPDATE
    SET had_source = public.erasure_records.had_source OR EXCLUDED.had_source;
    RETURN OLD;
END
$function$;

CREATE TRIGGER record_user_erasure
    AFTER DELETE ON users
    FOR EACH ROW EXECUTE FUNCTION record_user_erasure();

CREATE TRIGGER record_novel_erasure
    AFTER DELETE ON novels
    FOR EACH ROW EXECUTE FUNCTION record_novel_erasure();

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

-- 章节检索投影：小块比整章更适合相关性排序，也为以后可选的向量检索保留清晰边界。
CREATE TABLE chapter_chunks (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    chapter_id      UUID NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
    chunk_index     INTEGER NOT NULL CHECK (chunk_index >= 0),
    content         TEXT NOT NULL CHECK (content <> ''),
    UNIQUE(chapter_id, chunk_index)
);

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
    novel_id      UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    layer         memory_layer NOT NULL,
    content       TEXT NOT NULL,
    importance    SMALLINT NOT NULL DEFAULT 5 CHECK (importance BETWEEN 1 AND 10),
    chapter_number INTEGER,
    -- pgvector 语义向量（1536维，OpenAI text-embedding-3-small）
    embedding     vector(1536),
    access_count  INTEGER NOT NULL DEFAULT 0,
    last_accessed TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,               -- 短期记忆有过期时间
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_memories_character_user ON character_memories(character_id, user_id);
CREATE INDEX idx_memories_character_user_novel ON character_memories(character_id, user_id, novel_id);
CREATE INDEX idx_memories_layer ON character_memories(character_id, user_id, layer);
CREATE INDEX idx_memories_importance ON character_memories(character_id, user_id, importance DESC);
-- 向量相似度索引（HNSW，适合高维向量）
CREATE INDEX idx_memories_embedding ON character_memories
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- ─── 对话历史表 ────────────────────────────────────────────────────────────

CREATE TABLE chat_turns (
    id                     UUID PRIMARY KEY,
    user_id                UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    character_id           UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    novel_id               UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    request_fingerprint    BYTEA NOT NULL,
    chapter_context        INTEGER NOT NULL,
    reader_identity        VARCHAR(200),
    reader_identity_type   identity_type NOT NULL,
    reader_character_id    UUID REFERENCES characters(id) ON DELETE CASCADE,
    deviation_mode         deviation_mode NOT NULL,
    status                 VARCHAR(16) NOT NULL,
    attempt                BIGINT NOT NULL DEFAULT 1,
    lease_expires_at       TIMESTAMPTZ,
    failure_code           VARCHAR(64),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    completed_at           TIMESTAMPTZ,
    CONSTRAINT chat_turns_request_fingerprint_check
        CHECK (pg_catalog.octet_length(request_fingerprint) = 32),
    CONSTRAINT chat_turns_chapter_context_check CHECK (chapter_context >= 1),
    CONSTRAINT chat_turns_status_check
        CHECK (status IN ('in_progress', 'completed', 'failed')),
    CONSTRAINT chat_turns_attempt_check CHECK (attempt >= 1),
    CONSTRAINT chat_turns_identity_fields_check CHECK (
        (reader_identity_type = 'self' AND reader_character_id IS NULL)
        OR (
            reader_identity_type = 'character'
            AND reader_character_id IS NOT NULL
            AND reader_identity IS NOT NULL
        )
    ),
    CONSTRAINT chat_turns_state_check CHECK (
        (
            status = 'in_progress'
            AND lease_expires_at IS NOT NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'completed'
            AND lease_expires_at IS NULL
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
        OR (
            status = 'failed'
            AND lease_expires_at IS NULL
            AND failure_code IS NOT NULL
            AND failure_code <> ''
            AND completed_at IS NULL
        )
    )
);

CREATE UNIQUE INDEX idx_chat_turns_one_in_progress
    ON chat_turns(user_id, character_id, novel_id)
    WHERE status = 'in_progress';

CREATE TABLE chat_messages (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    character_id UUID NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id     UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    role         VARCHAR(20) NOT NULL CHECK (role IN ('user', 'character')),
    content      TEXT NOT NULL,
    reader_identity VARCHAR(200),
    chapter_context INTEGER,                 -- 对话发生时的章节
    turn_id      UUID REFERENCES chat_turns(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_chat_messages_character_user ON chat_messages(character_id, user_id, created_at DESC);
CREATE INDEX idx_chat_messages_novel_user ON chat_messages(novel_id, user_id);
CREATE UNIQUE INDEX idx_chat_messages_turn_role_unique
    ON chat_messages(turn_id, role)
    WHERE turn_id IS NOT NULL;

-- ─── 叙事节点表 ────────────────────────────────────────────────────────────

CREATE TABLE narrative_nodes (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id        UUID REFERENCES users(id) ON DELETE CASCADE,
    novel_id       UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_number INTEGER NOT NULL,
    description    TEXT NOT NULL,
    anchor_quote   TEXT,
    choices        JSONB NOT NULL DEFAULT '[]',  -- NarrativeChoice[]
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT narrative_nodes_identity_key UNIQUE(id, novel_id, chapter_number),
    CONSTRAINT narrative_nodes_anchor_quote_length_check
        CHECK (anchor_quote IS NULL OR char_length(anchor_quote) BETWEEN 1 AND 1000)
);

CREATE INDEX idx_narrative_nodes_novel ON narrative_nodes(novel_id, chapter_number);
CREATE UNIQUE INDEX idx_narrative_nodes_canonical_chapter
    ON narrative_nodes(novel_id, chapter_number) WHERE user_id IS NULL;
CREATE UNIQUE INDEX idx_narrative_nodes_player_chapter
    ON narrative_nodes(user_id, novel_id, chapter_number) WHERE user_id IS NOT NULL;

-- ─── 用户选择记录表 ────────────────────────────────────────────────────────

CREATE TABLE user_choices (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    node_id         UUID NOT NULL,
    chapter_number  INTEGER NOT NULL,
    choice_index    INTEGER NOT NULL,
    choice_text     TEXT NOT NULL,
    consequence     TEXT NOT NULL,           -- transition 的读者可见投影
    transition      JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT user_choices_user_node_key UNIQUE(user_id, node_id),
    CONSTRAINT user_choices_node_scope_fkey
        FOREIGN KEY(node_id, novel_id, chapter_number)
        REFERENCES narrative_nodes(id, novel_id, chapter_number) ON DELETE CASCADE,
    CONSTRAINT user_choices_chapter_check CHECK (chapter_number >= 1),
    CONSTRAINT user_choices_index_check CHECK (choice_index >= 0),
    CONSTRAINT user_choices_text_check CHECK (choice_text <> ''),
    CONSTRAINT user_choices_consequence_check CHECK (consequence <> ''),
    CONSTRAINT user_choices_transition_check CHECK (
        jsonb_typeof(transition) = 'object'
        AND transition @> '{"schema_version": 1}'::jsonb
        AND jsonb_typeof(transition -> 'prompt_version') = 'string'
        AND jsonb_typeof(transition -> 'canon_model_version') = 'number'
        AND jsonb_typeof(transition -> 'canonical_checkpoint_chapter') = 'number'
        AND jsonb_typeof(transition -> 'rendered_narrative') = 'string'
        AND jsonb_typeof(transition -> 'events') = 'array'
        AND jsonb_typeof(transition -> 'relationship_changes') = 'array'
        AND jsonb_typeof(transition -> 'location_changes') = 'array'
        AND jsonb_typeof(transition -> 'thread_changes') = 'array'
    ),
    CONSTRAINT user_choices_transition_projection_check CHECK (
        transition ->> 'rendered_narrative' = consequence
    )
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

-- ─── 玩家时间线章节表 ──────────────────────────────────────────────────────

-- 原著 chapters 永远保持不可变。玩家第一次作出选择后，当前章锚点之后以及
-- 所有后续章节都写入这张按用户隔离的投影表。
CREATE TABLE player_chapters (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    chapter_number  INTEGER NOT NULL CHECK (chapter_number >= 1),
    content         TEXT NOT NULL CHECK (content <> ''),
    origin          VARCHAR(20) NOT NULL CHECK (origin IN ('choice', 'continuation')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id, chapter_number)
);

CREATE INDEX idx_player_chapters_timeline
    ON player_chapters(user_id, novel_id, chapter_number DESC);

-- ─── 开放世界回合账本 ────────────────────────────────────────────────────

CREATE TABLE world_turns (
    id                   UUID PRIMARY KEY,
    user_id              UUID NOT NULL,
    novel_id             UUID NOT NULL,
    request_fingerprint  BYTEA NOT NULL,
    action               JSONB NOT NULL,
    expected_turn_number BIGINT NOT NULL,
    status               VARCHAR(16) NOT NULL,
    attempt              BIGINT NOT NULL DEFAULT 1,
    lease_expires_at     TIMESTAMPTZ,
    transition           JSONB,
    result               JSONB,
    failure_code         VARCHAR(64),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at         TIMESTAMPTZ,
    CONSTRAINT world_turns_world_state_fkey
        FOREIGN KEY(user_id, novel_id)
        REFERENCES world_states(user_id, novel_id) ON DELETE CASCADE,
    CONSTRAINT world_turns_request_fingerprint_check
        CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT world_turns_action_check CHECK (jsonb_typeof(action) = 'object'),
    CONSTRAINT world_turns_expected_turn_check CHECK (expected_turn_number >= 0),
    CONSTRAINT world_turns_status_check
        CHECK (status IN ('in_progress', 'completed', 'failed')),
    CONSTRAINT world_turns_attempt_check CHECK (attempt >= 1),
    CONSTRAINT world_turns_state_check CHECK (
        (
            status = 'in_progress'
            AND lease_expires_at IS NOT NULL
            AND transition IS NULL
            AND result IS NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'completed'
            AND lease_expires_at IS NULL
            AND jsonb_typeof(transition) = 'object'
            AND jsonb_typeof(result) = 'object'
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
        OR (
            status = 'failed'
            AND lease_expires_at IS NULL
            AND transition IS NULL
            AND result IS NULL
            AND failure_code IS NOT NULL
            AND failure_code <> ''
            AND completed_at IS NULL
        )
    )
);

CREATE UNIQUE INDEX idx_world_turns_one_in_progress
    ON world_turns(user_id, novel_id) WHERE status = 'in_progress';
CREATE INDEX idx_world_turns_journal
    ON world_turns(user_id, novel_id, completed_at DESC)
    WHERE status = 'completed';

-- ─── 不可变原著世界模型 ────────────────────────────────────────────────────

CREATE TABLE canon_story_models (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    novel_id        UUID NOT NULL REFERENCES novels(id) ON DELETE CASCADE,
    model_version   INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL,
    prompt_version  VARCHAR(100) NOT NULL,
    content         JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT canon_story_models_novel_version_key UNIQUE(novel_id, model_version),
    CONSTRAINT canon_story_models_model_version_check CHECK (model_version >= 1),
    CONSTRAINT canon_story_models_schema_version_check CHECK (schema_version >= 1),
    CONSTRAINT canon_story_models_prompt_version_check
        CHECK (char_length(prompt_version) BETWEEN 1 AND 100),
    CONSTRAINT canon_story_models_content_check CHECK (jsonb_typeof(content) = 'object')
);

CREATE FUNCTION reject_canon_story_model_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    RAISE EXCEPTION 'canon story models are immutable' USING ERRCODE = '55000';
END
$function$;

CREATE TRIGGER reject_canon_story_model_update
    BEFORE UPDATE ON canon_story_models
    FOR EACH ROW EXECUTE FUNCTION reject_canon_story_model_update();

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
    UNIQUE(user_id, novel_id),
    CONSTRAINT reading_progress_current_chapter_check CHECK (current_chapter >= 1),
    CONSTRAINT reading_progress_identity_fields_check CHECK (
        (reader_identity_type = 'self' AND reader_character_id IS NULL)
        OR (reader_identity_type = 'character' AND reader_character_id IS NOT NULL)
    )
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

-- ─── 运行时模型设置（user-service 所有）──────────────────────────────────

CREATE TABLE runtime_llm_config (
    singleton          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    provider           VARCHAR(32) NOT NULL,
    api_url            TEXT NOT NULL,
    model              VARCHAR(200) NOT NULL,
    thinking_enabled   BOOLEAN NOT NULL DEFAULT FALSE,
    api_key_nonce      BYTEA NOT NULL,
    api_key_ciphertext BYTEA NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

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
