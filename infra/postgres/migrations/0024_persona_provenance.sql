-- Preserve pre-contract chat and derived memories for export while requiring
-- server-validated persona provenance on every new online path.
BEGIN;

SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

ALTER TABLE public.chat_turns
    ADD COLUMN IF NOT EXISTS persona_source_chapter_high_water pg_catalog.int4;

ALTER TABLE public.chat_turns
    DROP CONSTRAINT IF EXISTS chat_turns_persona_source_chapter_high_water_check;

ALTER TABLE public.chat_turns
    ADD CONSTRAINT chat_turns_persona_source_chapter_high_water_check CHECK (
        persona_source_chapter_high_water IS NULL
        OR (
            persona_source_chapter_high_water >= 1
            AND persona_source_chapter_high_water <= chapter_context
        )
    );

ALTER TABLE public.character_memories
    ADD COLUMN IF NOT EXISTS persona_source_chapter_high_water pg_catalog.int4;

ALTER TABLE public.character_memories
    DROP CONSTRAINT IF EXISTS character_memories_persona_source_chapter_high_water_check;

ALTER TABLE public.character_memories
    ADD CONSTRAINT character_memories_persona_source_chapter_high_water_check CHECK (
        persona_source_chapter_high_water IS NULL
        OR (
            (
                layer::pg_catalog.text = 'mid'::pg_catalog.text
                OR layer::pg_catalog.text = 'long'::pg_catalog.text
            )
            AND chapter_number IS NOT NULL
            AND persona_source_chapter_high_water >= 1
            AND persona_source_chapter_high_water <= chapter_number
        )
    );

COMMIT;
