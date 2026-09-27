BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

-- An advisory decision is separate from explicit immutable series confirmation.
-- Claims are never reclaimed: a lost provider outcome cannot be billed again.
CREATE TABLE IF NOT EXISTS public.series_match_decisions (
    user_id UUID NOT NULL,
    novel_id UUID NOT NULL,
    method TEXT NOT NULL CHECK (method IN ('laya', 'deepseek')),
    evidence_key TEXT NOT NULL CHECK (evidence_key ~ '^[0-9a-f]{64}$'),
    claim_token UUID NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMPTZ,
    result JSONB CHECK (result IS NULL OR pg_catalog.jsonb_typeof(result) = 'object'),
    PRIMARY KEY (user_id, novel_id, method, evidence_key),
    CONSTRAINT series_match_decisions_shelf_fkey FOREIGN KEY (user_id, novel_id)
        REFERENCES public.user_novels(user_id, novel_id) ON DELETE CASCADE,
    CHECK ((completed_at IS NULL) = (result IS NULL))
);
COMMIT;
