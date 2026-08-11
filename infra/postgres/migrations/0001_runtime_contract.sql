-- Bring existing persistent databases in line with the production repositories.
-- Every statement is safe to replay; Compose runs this file on each deployment.

ALTER TABLE public.character_memories ADD COLUMN IF NOT EXISTS novel_id UUID;
ALTER TABLE public.character_memories ADD COLUMN IF NOT EXISTS chapter_number INTEGER;

UPDATE public.character_memories AS memory
SET novel_id = character.novel_id
FROM public.characters AS character
WHERE memory.character_id = character.id
  AND memory.novel_id IS NULL;

DO $migration$
BEGIN
    IF EXISTS (SELECT 1 FROM public.character_memories WHERE novel_id IS NULL) THEN
        RAISE EXCEPTION 'cannot backfill character_memories.novel_id';
    END IF;
END
$migration$;

ALTER TABLE public.character_memories ALTER COLUMN novel_id SET NOT NULL;

DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'character_memories_novel_id_fkey'
          AND conrelid = 'public.character_memories'::regclass
    ) THEN
        ALTER TABLE public.character_memories
            ADD CONSTRAINT character_memories_novel_id_fkey
            FOREIGN KEY (novel_id) REFERENCES public.novels(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'character_memories_novel_id_fkey'
          AND conrelid = 'public.character_memories'::regclass
          AND pg_get_constraintdef(oid) =
              'FOREIGN KEY (novel_id) REFERENCES novels(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'character_memories novel foreign key has an unexpected definition';
    END IF;
END
$migration$;

CREATE INDEX IF NOT EXISTS idx_memories_character_user_novel
    ON public.character_memories(character_id, user_id, novel_id);

ALTER TABLE public.chat_messages ADD COLUMN IF NOT EXISTS reader_identity VARCHAR(200);
ALTER TABLE public.chat_messages ADD COLUMN IF NOT EXISTS chapter_context INTEGER;

DO $migration$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'chat_messages'
          AND column_name = 'chapter_num'
    ) THEN
        IF EXISTS (
            SELECT 1 FROM public.chat_messages
            WHERE chapter_num IS NOT NULL
              AND chapter_context IS NOT NULL
              AND chapter_num <> chapter_context
        ) THEN
            RAISE EXCEPTION 'chat_messages chapter context conflict';
        END IF;

        UPDATE public.chat_messages
        SET chapter_context = chapter_num
        WHERE chapter_context IS NULL;

        ALTER TABLE public.chat_messages DROP COLUMN chapter_num;
    END IF;
END
$migration$;

DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'narrative_nodes_novel_chapter_key'
          AND conrelid = 'public.narrative_nodes'::regclass
    ) THEN
        IF EXISTS (
            SELECT 1
            FROM public.narrative_nodes
            GROUP BY novel_id, chapter_number
            HAVING COUNT(*) > 1
        ) THEN
            RAISE EXCEPTION
                'cannot enforce narrative node uniqueness: duplicate novel/chapter rows exist';
        END IF;

        ALTER TABLE public.narrative_nodes
            ADD CONSTRAINT narrative_nodes_novel_chapter_key
            UNIQUE (novel_id, chapter_number);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'narrative_nodes_novel_chapter_key'
          AND conrelid = 'public.narrative_nodes'::regclass
          AND pg_get_constraintdef(oid) = 'UNIQUE (novel_id, chapter_number)'
    ) THEN
        RAISE EXCEPTION 'narrative node uniqueness constraint has an unexpected definition';
    END IF;
END
$migration$;
