-- Bind every new in-progress chat turn to the exact committed world snapshot
-- used to build its prompt. Legacy terminal rows remain readable/exportable.
BEGIN;

SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

ALTER TABLE public.chat_turns
    ADD COLUMN IF NOT EXISTS world_revision pg_catalog.bytea;

-- A pre-contract in-progress claim cannot prove which world snapshot it used.
-- Fail it so a retry must claim a fresh, current snapshot instead of reclaiming
-- ambiguous work.
UPDATE public.chat_turns
SET status = 'failed',
    lease_expires_at = NULL,
    failure_code = 'causal_revision_unavailable',
    updated_at = pg_catalog.clock_timestamp(),
    completed_at = NULL
WHERE status::pg_catalog.text = 'in_progress'::pg_catalog.text
  AND world_revision IS NULL;

ALTER TABLE public.chat_turns
    DROP CONSTRAINT IF EXISTS chat_turns_world_revision_check;

ALTER TABLE public.chat_turns
    ADD CONSTRAINT chat_turns_world_revision_check CHECK (
        world_revision IS NULL
        OR pg_catalog.octet_length(world_revision) = 32
    );

ALTER TABLE public.chat_turns
    DROP CONSTRAINT IF EXISTS chat_turns_world_revision_state_check;

ALTER TABLE public.chat_turns
    ADD CONSTRAINT chat_turns_world_revision_state_check CHECK (
        status::pg_catalog.text <> 'in_progress'::pg_catalog.text
        OR world_revision IS NOT NULL
    );

COMMIT;
