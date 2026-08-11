-- Repair legacy progress rows before enforcing the application invariants.
-- Safe to replay: normalized rows remain unchanged and constraints are scoped
-- to the target relation.

DO $migration$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM public.reading_progress AS progress
        JOIN public.novels AS novel ON novel.id = progress.novel_id
        WHERE NOT EXISTS (
            SELECT 1
            FROM public.chapters AS chapter
            WHERE chapter.novel_id = progress.novel_id
              AND chapter.chapter_number BETWEEN 1 AND novel.total_chapters
        )
    ) THEN
        RAISE EXCEPTION 'cannot repair reading progress: novel has no readable chapter';
    END IF;
END
$migration$;

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
    WHERE progress.current_chapter NOT BETWEEN 1 AND novel.total_chapters
       OR NOT EXISTS (
           SELECT 1
           FROM public.chapters AS chapter
           WHERE chapter.novel_id = progress.novel_id
             AND chapter.chapter_number = progress.current_chapter
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
  AND canonical.name !~ '[[:cntrl:]]';

UPDATE public.reading_progress AS progress
SET reader_identity_type = 'self',
    reader_identity = NULL,
    reader_character_id = NULL
WHERE progress.reader_identity_type = 'character'
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
  AND progress.reader_identity IS DISTINCT FROM character.name;

UPDATE public.reading_progress
SET reader_character_id = NULL
WHERE reader_identity_type = 'self'
  AND reader_character_id IS NOT NULL;

UPDATE public.reading_progress
SET reader_identity = NULLIF(
    BTRIM(
        reader_identity,
        U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
    ),
    ''
)
WHERE reader_identity_type = 'self'
  AND reader_identity IS NOT NULL;

UPDATE public.reading_progress
SET reader_identity = NULL
WHERE reader_identity IS NOT NULL
  AND reader_identity ~ '[[:cntrl:]]';

DO $migration$
BEGIN
    PERFORM set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reading_progress_current_chapter_check'
          AND conrelid = 'public.reading_progress'::regclass
    ) THEN
        ALTER TABLE public.reading_progress
            ADD CONSTRAINT reading_progress_current_chapter_check
            CHECK (current_chapter >= 1);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reading_progress_current_chapter_check'
          AND conrelid = 'public.reading_progress'::regclass
          AND contype = 'c'
          AND pg_get_constraintdef(oid) = 'CHECK ((current_chapter >= 1))'
    ) THEN
        RAISE EXCEPTION 'reading progress chapter constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reading_progress_identity_fields_check'
          AND conrelid = 'public.reading_progress'::regclass
    ) THEN
        ALTER TABLE public.reading_progress
            ADD CONSTRAINT reading_progress_identity_fields_check
            CHECK (
                (reader_identity_type = 'self' AND reader_character_id IS NULL)
                OR (reader_identity_type = 'character' AND reader_character_id IS NOT NULL)
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reading_progress_identity_fields_check'
          AND conrelid = 'public.reading_progress'::regclass
          AND contype = 'c'
          AND pg_get_constraintdef(oid) =
              'CHECK ((((reader_identity_type = ''self''::identity_type) AND (reader_character_id IS NULL)) OR ((reader_identity_type = ''character''::identity_type) AND (reader_character_id IS NOT NULL))))'
    ) THEN
        RAISE EXCEPTION 'reading progress identity constraint has an unexpected definition';
    END IF;
END
$migration$;
