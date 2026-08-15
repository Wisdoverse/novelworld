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

-- Chapters are only resumable when they form 1..N in order with readable
-- content; UNIQUE(novel_id, chapter_number) makes count/min/max sufficient.
-- They are additionally authoritative when the novel's advertised
-- total_chapters agrees, which is what enrichment and publication require.
-- Readable means "carries a character outside Unicode White_Space". The
-- explicit character set below mirrors exactly what Rust str::trim() strips,
-- and unlike POSIX [:space:] it is locale-independent: under LC_CTYPE=C the
-- regex class matches no non-ASCII character, so NBSP-only content would
-- diverge from the runtime's own blank check.
WITH chapter_shape AS (
    SELECT chapter.novel_id,
           pg_catalog.count(*)::pg_catalog.int4 AS chapter_count,
           pg_catalog.min(chapter.chapter_number) AS lowest_chapter,
           pg_catalog.max(chapter.chapter_number) AS highest_chapter,
           pg_catalog.bool_and(pg_catalog.btrim(
               chapter.content,
               U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) <> '') AS every_chapter_readable
    FROM public.chapters AS chapter
    GROUP BY chapter.novel_id
),
classified AS (
    SELECT n.id,
           n.status::pg_catalog.text AS novel_status,
           n.created_at,
           n.updated_at,
           shape.novel_id IS NOT NULL AS has_chapters,
           COALESCE(
               shape.lowest_chapter = 1
               AND shape.highest_chapter = shape.chapter_count
               AND shape.every_chapter_readable,
               FALSE
           ) AS chapters_resumable,
           COALESCE(
               shape.lowest_chapter = 1
               AND shape.highest_chapter = shape.chapter_count
               AND shape.every_chapter_readable
               AND n.total_chapters = shape.chapter_count,
               FALSE
           ) AS chapters_authoritative,
           COALESCE(pg_catalog.btrim(
               n.world_summary,
               U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) <> '', FALSE)
               AND COALESCE(pg_catalog.btrim(
                   n.genre,
                   U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
               ) <> '', FALSE)
               AND EXISTS (
                   SELECT 1 FROM public.characters AS extracted
                   WHERE extracted.novel_id = n.id
               ) AS enrichment_present,
           EXISTS (
               SELECT 1 FROM public.canon_story_models AS canon
               WHERE canon.novel_id = n.id
           ) AS canon_present
    FROM public.novels AS n
    LEFT JOIN chapter_shape AS shape ON shape.novel_id = n.id
),
resolved AS (
    SELECT classified.*,
           classified.novel_status = 'ready'
               AND classified.chapters_authoritative
               AND classified.enrichment_present
               AND classified.canon_present AS import_complete
    FROM classified
)
INSERT INTO public.novel_import_jobs (
    novel_id, stage, status, attempt, lease_expires_at,
    failure_code, created_at, updated_at
)
SELECT
    n.id,
    CASE
        WHEN n.import_complete THEN 'completed'
        WHEN n.has_chapters AND NOT n.chapters_resumable THEN 'source'
        WHEN n.chapters_authoritative AND n.enrichment_present THEN 'enriched'
        WHEN n.has_chapters THEN 'chapters'
        ELSE 'source'
    END,
    CASE WHEN n.import_complete THEN 'completed' ELSE 'failed' END,
    0,
    NULL,
    CASE
        WHEN n.import_complete THEN NULL
        WHEN n.has_chapters AND NOT n.chapters_resumable THEN 'legacy_chapters_invalid'
        WHEN n.novel_status = 'ready' THEN 'legacy_incomplete'
        WHEN n.novel_status = 'error' THEN 'legacy_error'
        ELSE 'interrupted_upgrade'
    END,
    n.created_at,
    n.updated_at
FROM resolved AS n
ON CONFLICT (novel_id) DO NOTHING;

-- Databases upgraded by the first revision of this migration recorded ready
-- novels with gapped or blank chapters as completed imports. ON CONFLICT above
-- leaves those rows untouched, so re-classify the ones that never satisfied
-- import completeness. Downgraded rows are 'failed' and cannot re-fire.
-- The readable set mirrors Rust str::trim() and is locale-independent; see the
-- backfill above.
WITH chapter_shape AS (
    SELECT chapter.novel_id,
           pg_catalog.count(*)::pg_catalog.int4 AS chapter_count,
           pg_catalog.min(chapter.chapter_number) AS lowest_chapter,
           pg_catalog.max(chapter.chapter_number) AS highest_chapter,
           pg_catalog.bool_and(pg_catalog.btrim(
               chapter.content,
               U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) <> '') AS every_chapter_readable
    FROM public.chapters AS chapter
    GROUP BY chapter.novel_id
),
classified AS (
    SELECT n.id,
           n.status::pg_catalog.text AS novel_status,
           shape.novel_id IS NOT NULL AS has_chapters,
           COALESCE(
               shape.lowest_chapter = 1
               AND shape.highest_chapter = shape.chapter_count
               AND shape.every_chapter_readable,
               FALSE
           ) AS chapters_resumable,
           COALESCE(
               shape.lowest_chapter = 1
               AND shape.highest_chapter = shape.chapter_count
               AND shape.every_chapter_readable
               AND n.total_chapters = shape.chapter_count,
               FALSE
           ) AS chapters_authoritative,
           COALESCE(pg_catalog.btrim(
               n.world_summary,
               U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) <> '', FALSE)
               AND COALESCE(pg_catalog.btrim(
                   n.genre,
                   U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
               ) <> '', FALSE)
               AND EXISTS (
                   SELECT 1 FROM public.characters AS extracted
                   WHERE extracted.novel_id = n.id
               ) AS enrichment_present,
           EXISTS (
               SELECT 1 FROM public.canon_story_models AS canon
               WHERE canon.novel_id = n.id
           ) AS canon_present
    FROM public.novels AS n
    LEFT JOIN chapter_shape AS shape ON shape.novel_id = n.id
),
resolved AS (
    SELECT classified.*,
           classified.novel_status = 'ready'
               AND classified.chapters_authoritative
               AND classified.enrichment_present
               AND classified.canon_present AS import_complete
    FROM classified
)
UPDATE public.novel_import_jobs AS job
SET stage = CASE
        WHEN n.has_chapters AND NOT n.chapters_resumable THEN 'source'
        WHEN n.chapters_authoritative AND n.enrichment_present THEN 'enriched'
        WHEN n.has_chapters THEN 'chapters'
        ELSE 'source'
    END,
    status = 'failed',
    lease_expires_at = NULL,
    failure_code = CASE
        WHEN n.has_chapters AND NOT n.chapters_resumable THEN 'legacy_chapters_invalid'
        WHEN n.novel_status = 'ready' THEN 'legacy_incomplete'
        WHEN n.novel_status = 'error' THEN 'legacy_error'
        ELSE 'interrupted_upgrade'
    END,
    updated_at = pg_catalog.now()
FROM resolved AS n
WHERE n.id = job.novel_id
  AND job.status = 'completed'
  AND NOT n.import_complete;

-- Compose runs every migration on each deployment, so only touch novels that
-- are not already carrying the converted failure.
UPDATE public.novels AS n
SET status = 'error'::public.novel_status,
    parse_error = repair.message,
    updated_at = pg_catalog.now()
FROM (
    SELECT job.novel_id,
           CASE job.failure_code
               WHEN 'legacy_chapters_invalid'
                   THEN 'Imported chapters are unusable after upgrade; re-upload the source'
               WHEN 'legacy_incomplete'
                   THEN 'Import data is incomplete after upgrade; retry or re-upload the source'
               WHEN 'legacy_error'
                   THEN 'Previous import failed; retry or re-upload the source'
               ELSE 'Import was interrupted by an upgrade; retry or re-upload the source'
           END AS message
    FROM public.novel_import_jobs AS job
    WHERE job.failure_code IN (
        'interrupted_upgrade', 'legacy_incomplete',
        'legacy_error', 'legacy_chapters_invalid'
    )
) AS repair
WHERE repair.novel_id = n.id
  AND n.status::pg_catalog.text IN ('pending', 'parsing', 'ready', 'error')
  AND (
      n.status::pg_catalog.text <> 'error'
      OR n.parse_error IS DISTINCT FROM repair.message
  );

-- Legacy databases predate ON DELETE CASCADE on the account deletion graph.
-- Without it DELETE FROM users fails outright and durable import jobs would be
-- orphaned behind a novel that cannot be removed.
DO $migration$
DECLARE
    required_fk RECORD;
    expected_name pg_catalog.text;
    expected_definition pg_catalog.text;
    existing_name pg_catalog.text;
    existing_delete_rule pg_catalog."char";
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    FOR required_fk IN
        SELECT *
        FROM (VALUES
            ('novels', 'user_id', 'users'),
            ('chapters', 'novel_id', 'novels'),
            ('characters', 'novel_id', 'novels'),
            ('character_memories', 'character_id', 'characters'),
            ('character_memories', 'user_id', 'users'),
            ('chat_messages', 'character_id', 'characters'),
            ('chat_messages', 'novel_id', 'novels'),
            ('chat_messages', 'user_id', 'users'),
            ('narrative_nodes', 'novel_id', 'novels'),
            ('user_choices', 'novel_id', 'novels'),
            ('user_choices', 'user_id', 'users'),
            ('world_states', 'novel_id', 'novels'),
            ('world_states', 'user_id', 'users'),
            ('reading_progress', 'novel_id', 'novels'),
            ('reading_progress', 'user_id', 'users')
        ) AS deletion_graph(child, child_column, parent)
    LOOP
        expected_name := required_fk.child || '_' || required_fk.child_column || '_fkey';
        expected_definition := pg_catalog.format(
            'FOREIGN KEY (%I) REFERENCES %I(id) ON DELETE CASCADE',
            required_fk.child_column, required_fk.parent
        );

        SELECT existing.conname, existing.confdeltype
          INTO existing_name, existing_delete_rule
        FROM pg_catalog.pg_constraint AS existing
        WHERE existing.contype = 'f'
          AND existing.conrelid =
                  ('public.' || required_fk.child)::pg_catalog.regclass
          AND existing.confrelid =
                  ('public.' || required_fk.parent)::pg_catalog.regclass
          AND existing.conkey = ARRAY[(
                  SELECT attribute.attnum
                  FROM pg_catalog.pg_attribute AS attribute
                  WHERE attribute.attrelid = existing.conrelid
                    AND attribute.attname = required_fk.child_column
              )];

        IF existing_name IS NOT NULL
           AND (existing_name <> expected_name OR existing_delete_rule <> 'c') THEN
            EXECUTE pg_catalog.format(
                'ALTER TABLE public.%I DROP CONSTRAINT %I',
                required_fk.child, existing_name
            );
            existing_name := NULL;
        END IF;

        IF existing_name IS NULL THEN
            EXECUTE pg_catalog.format(
                'ALTER TABLE public.%I ADD CONSTRAINT %I %s',
                required_fk.child, expected_name, expected_definition
            );
        END IF;

        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_constraint AS verified
            WHERE verified.conname = expected_name
              AND verified.conrelid =
                      ('public.' || required_fk.child)::pg_catalog.regclass
              AND verified.contype = 'f'
              AND pg_catalog.pg_get_constraintdef(verified.oid) = expected_definition
        ) THEN
            RAISE EXCEPTION
                'account deletion foreign key %.% has an unexpected definition',
                required_fk.child, required_fk.child_column;
        END IF;
    END LOOP;
END
$migration$;
