-- Narrative source admission preserves sealed origins and serializes with turns.
BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
ALTER TABLE public.world_turns ADD COLUMN IF NOT EXISTS expected_source_chapter integer;
ALTER TABLE public.world_turns DROP CONSTRAINT IF EXISTS world_turns_expected_source_check;
ALTER TABLE public.world_turns ADD CONSTRAINT world_turns_expected_source_check
    CHECK (expected_source_chapter IS NULL OR expected_source_chapter >= 1);
CREATE TABLE IF NOT EXISTS public.world_source_operations (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    novel_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (pg_catalog.octet_length(request_fingerprint) = 32),
    expected_turn_number BIGINT NOT NULL CHECK (expected_turn_number >= 0),
    previous_source_chapter INTEGER NOT NULL CHECK (previous_source_chapter >= 1),
    source_chapter INTEGER NOT NULL CHECK (source_chapter = previous_source_chapter + 1),
    source_context JSONB NOT NULL CHECK (pg_catalog.jsonb_typeof(source_context) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT world_source_operations_world_state_fkey FOREIGN KEY (user_id, novel_id)
        REFERENCES public.world_states(user_id, novel_id) ON DELETE CASCADE,
    UNIQUE (user_id, novel_id, source_chapter)
);
COMMIT;
