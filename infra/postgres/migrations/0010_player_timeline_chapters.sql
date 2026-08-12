ALTER TABLE public.narrative_nodes
    ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE public.narrative_nodes
    DROP CONSTRAINT IF EXISTS narrative_nodes_novel_chapter_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_narrative_nodes_canonical_chapter
    ON public.narrative_nodes(novel_id, chapter_number)
    WHERE user_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_narrative_nodes_player_chapter
    ON public.narrative_nodes(user_id, novel_id, chapter_number)
    WHERE user_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS public.player_chapters (
    id              UUID PRIMARY KEY DEFAULT public.uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    novel_id        UUID NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    chapter_number  INTEGER NOT NULL CHECK (chapter_number >= 1),
    content         TEXT NOT NULL CHECK (content <> ''),
    origin          VARCHAR(20) NOT NULL CHECK (origin IN ('choice', 'continuation')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    UNIQUE(user_id, novel_id, chapter_number)
);

CREATE INDEX IF NOT EXISTS idx_player_chapters_timeline
    ON public.player_chapters(user_id, novel_id, chapter_number DESC);

-- Backfill the divergence chapter for choices committed by the pre-timeline
-- implementation. Only exact, still-valid anchors are accepted; ambiguous
-- legacy rows fail closed and can be repaired through the choice endpoint.
INSERT INTO public.player_chapters (
    id, user_id, novel_id, chapter_number, content, origin, created_at, updated_at
)
SELECT
    public.uuid_generate_v4(),
    choice.user_id,
    choice.novel_id,
    choice.chapter_number,
    pg_catalog.left(
        chapter.content,
        pg_catalog.strpos(chapter.content, node.anchor_quote)
            + pg_catalog.char_length(node.anchor_quote) - 1
    ) || E'\n\n' || choice.consequence,
    'choice',
    choice.created_at,
    pg_catalog.now()
FROM public.user_choices AS choice
JOIN public.narrative_nodes AS node
  ON node.id = choice.node_id
 AND node.novel_id = choice.novel_id
 AND node.chapter_number = choice.chapter_number
JOIN public.chapters AS chapter
  ON chapter.novel_id = choice.novel_id
 AND chapter.chapter_number = choice.chapter_number
WHERE choice.consequence IS NOT NULL
  AND choice.consequence <> ''
  AND node.anchor_quote IS NOT NULL
  AND pg_catalog.strpos(chapter.content, node.anchor_quote) > 0
ON CONFLICT (user_id, novel_id, chapter_number) DO NOTHING;
