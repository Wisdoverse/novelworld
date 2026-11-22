-- Derived chapter chunks power spoiler-bounded lore retrieval without adding
-- another vector database or API key.
CREATE EXTENSION IF NOT EXISTS "uuid-ossp" WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

-- Very old databases did not retain source text. They remain readable, but
-- naturally produce no lore chunks until the novel is imported again.
ALTER TABLE public.chapters ADD COLUMN IF NOT EXISTS content TEXT;
UPDATE public.chapters SET content = '' WHERE content IS NULL;
ALTER TABLE public.chapters ALTER COLUMN content SET NOT NULL;

CREATE TABLE IF NOT EXISTS public.chapter_chunks (
    id              UUID PRIMARY KEY DEFAULT public.uuid_generate_v4(),
    chapter_id      UUID NOT NULL REFERENCES public.chapters(id) ON DELETE CASCADE,
    chunk_index     INTEGER NOT NULL CHECK (chunk_index >= 0),
    content         TEXT NOT NULL CHECK (content <> ''),
    UNIQUE(chapter_id, chunk_index)
);

-- Backfill only chapters without any chunk, so replay cost does not grow with
-- chunked text. Novel service writes all chunks of a chapter in one transaction.
-- Replay does not repair a stale or partial chunk set.
-- MATERIALIZED keeps all content reads behind the chunk-existence filter.
WITH missing AS MATERIALIZED (
    SELECT chapter.id, chapter.content
    FROM public.chapters AS chapter
    WHERE NOT EXISTS (
        SELECT 1 FROM public.chapter_chunks AS existing
        WHERE existing.chapter_id = chapter.id
    )
)
INSERT INTO public.chapter_chunks (id, chapter_id, chunk_index, content)
SELECT public.uuid_generate_v4(), missing.id,
       ((piece.start_at - 1) / 1050)::INTEGER,
       btrim(substring(missing.content FROM piece.start_at FOR 1200))
FROM missing
CROSS JOIN LATERAL generate_series(1, GREATEST(char_length(missing.content), 1), 1050) AS piece(start_at)
WHERE btrim(substring(missing.content FROM piece.start_at FOR 1200)) <> ''
ON CONFLICT (chapter_id, chunk_index) DO NOTHING;
