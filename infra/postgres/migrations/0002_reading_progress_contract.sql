-- Replay is atomic. Import acceptance deliberately persists reader defaults
-- before chapters exist; pending/parsing/error records are not corrupted reads.
-- Keep those records unchanged, while ready and unknown legacy states fail closed.
-- Declarative guards keep this entire migration under strict static SQL audit.
BEGIN;
SELECT pg_catalog.set_config('search_path', 'public,pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

-- Cast only the selected text result, never a constant error inside a CASE arm.
SELECT (CASE WHEN EXISTS (
    SELECT 1
    FROM public.reading_progress AS progress
    JOIN public.novels AS novel ON novel.id = progress.novel_id
    WHERE COALESCE(pg_catalog.to_jsonb(novel)->>'status', 'legacy')
              NOT IN ('pending', 'parsing', 'error')
      AND NOT EXISTS (
          SELECT 1 FROM public.chapters AS chapter
          WHERE chapter.novel_id = progress.novel_id
            AND chapter.chapter_number BETWEEN 1 AND novel.total_chapters
      )
) THEN 'cannot repair reading progress: novel has no readable chapter'
  ELSE '1' END)::pg_catalog.int4;

-- Existing named constraints must match before any data or catalog mutation.
SELECT (CASE WHEN EXISTS (
    SELECT 1 FROM pg_catalog.pg_constraint
    WHERE conname = 'reading_progress_current_chapter_check'
      AND conrelid = 'public.reading_progress'::pg_catalog.regclass
      AND (contype <> 'c' OR NOT convalidated
           OR pg_catalog.pg_get_constraintdef(oid) <> 'CHECK ((current_chapter >= 1))')
) THEN 'reading progress chapter constraint has an unexpected definition' ELSE '1' END)::pg_catalog.int4;

SELECT (CASE WHEN EXISTS (
    SELECT 1 FROM pg_catalog.pg_constraint
    WHERE conname = 'reading_progress_identity_fields_check'
      AND conrelid = 'public.reading_progress'::pg_catalog.regclass
      AND (contype <> 'c' OR NOT convalidated
           OR pg_catalog.pg_get_constraintdef(oid) <> 'CHECK ((((reader_identity_type = ''self''::identity_type) AND (reader_character_id IS NULL)) OR ((reader_identity_type = ''character''::identity_type) AND (reader_character_id IS NOT NULL))))')
) THEN 'reading progress identity constraint has an unexpected definition' ELSE '1' END)::pg_catalog.int4;

WITH repairs AS (
    SELECT progress.id,
           COALESCE(
               (
                   SELECT MAX(chapter.chapter_number)
                   FROM public.chapters AS chapter
                   WHERE chapter.novel_id = progress.novel_id
                     AND chapter.chapter_number BETWEEN 1 AND novel.total_chapters
                     AND chapter.chapter_number <= progress.current_chapter
               ),
               (
                   SELECT MIN(chapter.chapter_number)
                   FROM public.chapters AS chapter
                   WHERE chapter.novel_id = progress.novel_id
                     AND chapter.chapter_number BETWEEN 1 AND novel.total_chapters
               )
           ) AS current_chapter
    FROM public.reading_progress AS progress
    JOIN public.novels AS novel ON novel.id = progress.novel_id
    WHERE (progress.current_chapter NOT BETWEEN 1 AND novel.total_chapters
       OR NOT EXISTS (
           SELECT 1
           FROM public.chapters AS chapter
           WHERE chapter.novel_id = progress.novel_id
             AND chapter.chapter_number = progress.current_chapter
       ))
      AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    )
)
UPDATE public.reading_progress AS progress
SET current_chapter = repairs.current_chapter
FROM repairs
WHERE progress.id = repairs.id;

WITH canonical_names AS (
    SELECT id,
           BTRIM(
               name,
               U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) AS name
    FROM public.characters
)
UPDATE public.characters AS character
SET name = canonical.name
FROM canonical_names AS canonical
WHERE character.id = canonical.id
  AND character.name IS DISTINCT FROM canonical.name
  AND char_length(canonical.name) BETWEEN 1 AND 200
  AND canonical.name !~ '[[:cntrl:]]'
  AND EXISTS (
      SELECT 1 FROM public.chapters AS readable
      JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
      WHERE readable.novel_id = character.novel_id
        AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
  );

UPDATE public.reading_progress AS progress
SET reader_identity_type = 'self',
    reader_identity = NULL,
    reader_character_id = NULL
WHERE progress.reader_identity_type = 'character'
  AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    )
  AND NOT EXISTS (
      SELECT 1
      FROM public.characters AS character
      WHERE character.id = progress.reader_character_id
        AND character.novel_id = progress.novel_id
        AND character.first_appearance_chapter IS NOT NULL
        AND character.first_appearance_chapter BETWEEN 1 AND progress.current_chapter
        AND char_length(character.name) BETWEEN 1 AND 200
        AND character.name !~ '[[:cntrl:]]'
        AND character.name = BTRIM(
            character.name,
            U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
        )
  );

UPDATE public.reading_progress AS progress
SET reader_identity = character.name
FROM public.characters AS character
WHERE progress.reader_identity_type = 'character'
  AND progress.reader_character_id = character.id
  AND progress.novel_id = character.novel_id
  AND progress.reader_identity IS DISTINCT FROM character.name
  AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    );

UPDATE public.reading_progress AS progress
SET reader_character_id = NULL
WHERE reader_identity_type = 'self'
  AND reader_character_id IS NOT NULL
  AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    );

UPDATE public.reading_progress AS progress
SET reader_identity = NULLIF(
    BTRIM(
        reader_identity,
        U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
    ),
    ''
)
WHERE reader_identity_type = 'self'
  AND reader_identity IS NOT NULL
  AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    );

UPDATE public.reading_progress AS progress
SET reader_identity = NULL
WHERE reader_identity IS NOT NULL
  AND reader_identity ~ '[[:cntrl:]]'
  AND EXISTS (
        SELECT 1
        FROM public.chapters AS readable
        JOIN public.novels AS readable_novel ON readable_novel.id = readable.novel_id
        WHERE readable.novel_id = progress.novel_id
          AND readable.chapter_number BETWEEN 1 AND readable_novel.total_chapters
    );

-- Missing legacy constraints are installed; matching current ones are rebuilt
-- with the same definitions. Drift has already failed before any UPDATE.
ALTER TABLE public.reading_progress
    DROP CONSTRAINT IF EXISTS reading_progress_current_chapter_check,
    ADD CONSTRAINT reading_progress_current_chapter_check CHECK (current_chapter >= 1);
ALTER TABLE public.reading_progress
    DROP CONSTRAINT IF EXISTS reading_progress_identity_fields_check,
    ADD CONSTRAINT reading_progress_identity_fields_check CHECK (
        (reader_identity_type = 'self' AND reader_character_id IS NULL)
        OR (reader_identity_type = 'character' AND reader_character_id IS NOT NULL)
    );
COMMIT;
