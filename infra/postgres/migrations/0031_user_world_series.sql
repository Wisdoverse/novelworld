BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

-- A user's immutable, explicitly confirmed background and sourced basic rules.
-- The safe template snapshot has only fixed vocabulary, numeric values and
-- source identity/chapter references. No source text or provider prose is kept.
-- Source FKs intentionally do not cascade: removing a source novel must not
-- rewrite or destroy a player's frozen series binding.
CREATE TABLE IF NOT EXISTS public.user_world_series (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    name TEXT NOT NULL CHECK (pg_catalog.char_length(name) BETWEEN 1 AND 80),
    background TEXT NOT NULL CHECK (pg_catalog.char_length(background) BETWEEN 1 AND 2000),
    revision INTEGER NOT NULL CHECK (revision = 1),
    source_template JSONB NOT NULL CHECK (pg_catalog.jsonb_typeof(source_template) = 'object'),
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT user_world_series_owner_key UNIQUE (id, user_id),
    CONSTRAINT user_world_series_user_fkey FOREIGN KEY (user_id)
        REFERENCES public.users(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS public.user_novel_world_series (
    user_id UUID NOT NULL,
    novel_id UUID NOT NULL,
    series_id UUID NOT NULL,
    PRIMARY KEY (user_id, novel_id),
    CONSTRAINT user_novel_world_series_shelf_fkey FOREIGN KEY (user_id, novel_id)
        REFERENCES public.user_novels(user_id, novel_id) ON DELETE CASCADE,
    CONSTRAINT user_novel_world_series_owner_fkey FOREIGN KEY (series_id, user_id)
        REFERENCES public.user_world_series(id, user_id) ON DELETE CASCADE
);

CREATE OR REPLACE FUNCTION public.reject_world_series_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    RAISE EXCEPTION 'world series definitions are immutable' USING ERRCODE = '55000';
END
$function$;

DROP TRIGGER IF EXISTS reject_world_series_update ON public.user_world_series;
CREATE TRIGGER reject_world_series_update
    BEFORE UPDATE ON public.user_world_series
    FOR EACH ROW EXECUTE FUNCTION public.reject_world_series_update();
COMMIT;
