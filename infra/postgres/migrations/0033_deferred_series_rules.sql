BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
SELECT pg_catalog.set_config('statement_timeout', '30s', true);
SELECT pg_catalog.set_config('lock_timeout', '5s', true);

DROP TRIGGER IF EXISTS reject_world_series_update ON public.user_world_series;
ALTER TABLE public.user_world_series
    ADD COLUMN IF NOT EXISTS source_novel_id UUID;
UPDATE public.user_world_series
SET source_novel_id = (source_template->>'novel_id')::UUID
WHERE source_novel_id IS NULL AND source_template IS NOT NULL;
ALTER TABLE public.user_world_series
    ALTER COLUMN source_novel_id SET NOT NULL,
    ALTER COLUMN source_template DROP NOT NULL;

CREATE OR REPLACE FUNCTION public.reject_world_series_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    IF OLD.source_template IS NULL
       AND NEW.source_template IS NOT NULL
       AND pg_catalog.jsonb_typeof(NEW.source_template) = 'object'
       AND (NEW.source_template->>'novel_id')::UUID = OLD.source_novel_id
       AND (NEW.id, NEW.user_id, NEW.name, NEW.background, NEW.revision,
            NEW.source_novel_id, NEW.created_at)
           IS NOT DISTINCT FROM
           (OLD.id, OLD.user_id, OLD.name, OLD.background, OLD.revision,
            OLD.source_novel_id, OLD.created_at)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'world series definitions are immutable' USING ERRCODE = '55000';
END
$function$;

CREATE TRIGGER reject_world_series_update
    BEFORE UPDATE ON public.user_world_series
    FOR EACH ROW EXECUTE FUNCTION public.reject_world_series_update();

COMMIT;
