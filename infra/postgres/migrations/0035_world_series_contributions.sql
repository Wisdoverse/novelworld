BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

-- Presence is explicit consent to contribute current canonical-book grouping.
-- No private series definition, provider prose, or cached recommendation is copied.
CREATE TABLE IF NOT EXISTS public.world_series_contributions (
    series_id UUID NOT NULL,
    user_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (series_id, user_id),
    CONSTRAINT world_series_contributions_owner_fkey FOREIGN KEY (series_id, user_id)
        REFERENCES public.user_world_series(id, user_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS user_novel_world_series_pair_lookup
    ON public.user_novel_world_series (novel_id, user_id, series_id);
COMMIT;
