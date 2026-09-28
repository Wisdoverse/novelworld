BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

ALTER TABLE public.user_world_series ALTER COLUMN background DROP NOT NULL;

CREATE OR REPLACE FUNCTION public.reject_world_series_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    IF (NEW.id, NEW.user_id, NEW.name, NEW.revision, NEW.source_novel_id, NEW.created_at)
       IS DISTINCT FROM
       (OLD.id, OLD.user_id, OLD.name, OLD.revision, OLD.source_novel_id, OLD.created_at)
       OR (OLD.background IS DISTINCT FROM NEW.background AND NOT (
           OLD.background IS NULL AND NEW.background IS NOT NULL
           AND pg_catalog.char_length(NEW.background) BETWEEN 1 AND 2000
       ))
       OR (OLD.source_template IS DISTINCT FROM NEW.source_template AND (
           OLD.source_template IS NULL AND NEW.source_template IS NOT NULL
           AND pg_catalog.jsonb_typeof(NEW.source_template) = 'object'
           AND (NEW.source_template->>'novel_id')::UUID = OLD.source_novel_id
       ) IS NOT TRUE)
    THEN
        RAISE EXCEPTION 'world series definitions are immutable' USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END
$function$;

COMMIT;
