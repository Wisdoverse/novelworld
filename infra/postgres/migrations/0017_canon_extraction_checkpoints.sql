CREATE TABLE IF NOT EXISTS public.canon_extraction_checkpoints (
    novel_id       UUID NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    model_version  INTEGER NOT NULL,
    prompt_version VARCHAR(100) NOT NULL,
    chapter_number INTEGER NOT NULL,
    chunk_index    INTEGER NOT NULL,
    is_final       BOOLEAN NOT NULL,
    source_content TEXT NOT NULL,
    extraction     JSONB NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (
        novel_id, model_version, prompt_version, chapter_number, chunk_index
    ),
    CONSTRAINT canon_extraction_checkpoints_model_version_check
        CHECK (model_version >= 1),
    CONSTRAINT canon_extraction_checkpoints_prompt_version_check
        CHECK (pg_catalog.char_length(prompt_version) BETWEEN 1 AND 100),
    CONSTRAINT canon_extraction_checkpoints_chapter_number_check
        CHECK (chapter_number >= 1),
    CONSTRAINT canon_extraction_checkpoints_chunk_index_check
        CHECK (chunk_index >= 0),
    CONSTRAINT canon_extraction_checkpoints_source_content_check
        CHECK (pg_catalog.octet_length(source_content) BETWEEN 1 AND 8000),
    CONSTRAINT canon_extraction_checkpoints_extraction_check
        CHECK (pg_catalog.jsonb_typeof(extraction) = 'object')
);
