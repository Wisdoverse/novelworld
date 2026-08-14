DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_type AS type
        JOIN pg_catalog.pg_namespace AS namespace
          ON namespace.oid = type.typnamespace
        WHERE namespace.nspname = 'public' AND type.typname = 'novel_status'
    ) THEN
        CREATE TYPE public.novel_status AS ENUM ('pending', 'parsing', 'ready', 'error');
    END IF;
END
$migration$;

ALTER TABLE public.novels
    ADD COLUMN IF NOT EXISTS world_summary TEXT,
    ADD COLUMN IF NOT EXISTS genre VARCHAR(100),
    ADD COLUMN IF NOT EXISTS status public.novel_status NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS parse_error TEXT,
    ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now();

CREATE TABLE IF NOT EXISTS public.novel_import_jobs (
    novel_id         UUID PRIMARY KEY REFERENCES public.novels(id) ON DELETE CASCADE,
    stage            VARCHAR(16) NOT NULL DEFAULT 'source',
    status           VARCHAR(16) NOT NULL DEFAULT 'pending',
    attempt          BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TIMESTAMPTZ,
    failure_code     VARCHAR(64),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT novel_import_jobs_stage_check
        CHECK (stage IN ('source', 'chapters', 'enriched', 'completed')),
    CONSTRAINT novel_import_jobs_status_check
        CHECK (status IN ('pending', 'in_progress', 'failed', 'completed')),
    CONSTRAINT novel_import_jobs_attempt_check CHECK (attempt >= 0),
    CONSTRAINT novel_import_jobs_failure_code_check CHECK (
        failure_code IS NULL OR pg_catalog.char_length(failure_code) BETWEEN 1 AND 64
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

CREATE INDEX IF NOT EXISTS idx_novel_import_jobs_recoverable
    ON public.novel_import_jobs(status, lease_expires_at, created_at)
    WHERE status IN ('pending', 'in_progress');

WITH classified AS (
    SELECT n.*,
           n.status::pg_catalog.text = 'ready'
               AND n.total_chapters > 0
               AND NULLIF(pg_catalog.btrim(n.world_summary), '') IS NOT NULL
               AND NULLIF(pg_catalog.btrim(n.genre), '') IS NOT NULL
               AND n.total_chapters = (
                   SELECT pg_catalog.count(*)::pg_catalog.int4
                   FROM public.chapters AS complete_chapter
                   WHERE complete_chapter.novel_id = n.id
               )
               AND EXISTS (
                   SELECT 1 FROM public.characters AS complete_character
                   WHERE complete_character.novel_id = n.id
               )
               AND EXISTS (
                   SELECT 1 FROM public.canon_story_models AS canon
                   WHERE canon.novel_id = n.id
               ) AS import_complete
    FROM public.novels AS n
)
INSERT INTO public.novel_import_jobs (
    novel_id, stage, status, attempt, lease_expires_at,
    failure_code, created_at, updated_at
)
SELECT
    n.id,
    CASE
        WHEN n.import_complete THEN 'completed'
        WHEN n.total_chapters > 0
             AND NULLIF(pg_catalog.btrim(n.world_summary), '') IS NOT NULL
             AND NULLIF(pg_catalog.btrim(n.genre), '') IS NOT NULL
             AND n.total_chapters = (
                 SELECT pg_catalog.count(*)::pg_catalog.int4
                 FROM public.chapters AS enriched_chapter
                 WHERE enriched_chapter.novel_id = n.id
             )
             AND EXISTS (SELECT 1 FROM public.characters AS c WHERE c.novel_id = n.id)
            THEN 'enriched'
        WHEN EXISTS (SELECT 1 FROM public.chapters AS ch WHERE ch.novel_id = n.id)
            THEN 'chapters'
        ELSE 'source'
    END,
    CASE WHEN n.import_complete THEN 'completed' ELSE 'failed' END,
    0,
    NULL,
    CASE
        WHEN n.import_complete THEN NULL
        WHEN n.status::pg_catalog.text = 'ready' THEN 'legacy_incomplete'
        WHEN n.status::pg_catalog.text = 'error' THEN 'legacy_error'
        ELSE 'interrupted_upgrade'
    END,
    n.created_at,
    n.updated_at
FROM classified AS n
ON CONFLICT (novel_id) DO NOTHING;

UPDATE public.novels AS n
SET status = 'error'::public.novel_status,
    parse_error = CASE job.failure_code
        WHEN 'legacy_incomplete'
            THEN 'Import data is incomplete after upgrade; retry or re-upload the source'
        WHEN 'legacy_error'
            THEN 'Previous import failed; retry or re-upload the source'
        ELSE 'Import was interrupted by an upgrade; retry or re-upload the source'
    END,
    updated_at = pg_catalog.now()
FROM public.novel_import_jobs AS job
WHERE job.novel_id = n.id
  AND job.failure_code IN ('interrupted_upgrade', 'legacy_incomplete', 'legacy_error')
  AND n.status::pg_catalog.text IN ('pending', 'parsing', 'ready', 'error');
