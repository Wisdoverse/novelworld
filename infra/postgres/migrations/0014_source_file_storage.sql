CREATE TABLE IF NOT EXISTS public.source_file_deletions (
    object_key      TEXT PRIMARY KEY CHECK (
        object_key LIKE 'source-files/%'
        AND pg_catalog.octet_length(object_key) BETWEEN 1 AND 1024
    ),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    last_error      VARCHAR(500),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now()
);

CREATE INDEX IF NOT EXISTS idx_source_file_deletions_due
    ON public.source_file_deletions(next_attempt_at, object_key);

CREATE OR REPLACE FUNCTION public.queue_source_file_deletion()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    IF OLD.original_file_key LIKE 'source-files/%'
       AND pg_catalog.octet_length(OLD.original_file_key) BETWEEN 1 AND 1024 THEN
        INSERT INTO public.source_file_deletions (object_key)
        VALUES (OLD.original_file_key)
        ON CONFLICT (object_key) DO UPDATE
        SET next_attempt_at = LEAST(
            public.source_file_deletions.next_attempt_at,
            EXCLUDED.next_attempt_at
        );
    END IF;
    RETURN OLD;
END
$function$;

DROP TRIGGER IF EXISTS queue_source_file_deletion ON public.novels;
CREATE TRIGGER queue_source_file_deletion
    AFTER DELETE ON public.novels
    FOR EACH ROW EXECUTE FUNCTION public.queue_source_file_deletion();
