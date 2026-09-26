-- Canonical novel content is shared. A user's shelf and world remain private.
-- `novels.user_id` is retained as immutable uploader attribution, but is no
-- longer an ownership foreign key: deleting the uploader must not erase a
-- canonical asset that other users have attached.
BEGIN;
SELECT pg_catalog.set_config('search_path', 'public,pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);
LOCK TABLE public.novels IN SHARE ROW EXCLUSIVE MODE;
-- Capture first adoption before CREATE. An existing shelf table may be empty
-- because readers removed their books; replay must preserve that decision.
SELECT pg_catalog.set_config(
    'novelworld.migrate_0019_first_adoption',
    (pg_catalog.to_regclass('public.user_novels') IS NULL)::pg_catalog.text,
    true
);
ALTER TABLE public.novels
    DROP CONSTRAINT IF EXISTS novels_user_id_fkey;

CREATE TABLE IF NOT EXISTS public.user_novels (
    user_id  UUID NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    novel_id UUID NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    added_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (user_id, novel_id)
);

CREATE INDEX IF NOT EXISTS idx_user_novels_novel
    ON public.user_novels (novel_id);

-- Backfill uploader shelves only when the ownership model is first introduced.
INSERT INTO public.user_novels (user_id, novel_id, added_at)
SELECT n.user_id, n.id, n.created_at
FROM public.novels AS n
JOIN public.users AS u ON u.id = n.user_id
WHERE pg_catalog.current_setting('novelworld.migrate_0019_first_adoption') = 'true'
ON CONFLICT (user_id, novel_id) DO NOTHING;

DROP VIEW IF EXISTS public.user_shelf;
CREATE VIEW public.user_shelf AS
SELECT
    n.id,
    shelf.user_id,
    n.title,
    n.author,
    n.cover_url,
    n.genre,
    n.total_chapters,
    n.status,
    COALESCE(rp.deviation_mode, n.deviation_mode) AS deviation_mode,
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
FROM public.user_novels AS shelf
JOIN public.novels AS n ON n.id = shelf.novel_id
LEFT JOIN public.reading_progress AS rp
    ON rp.novel_id = n.id AND rp.user_id = shelf.user_id;
COMMIT;
