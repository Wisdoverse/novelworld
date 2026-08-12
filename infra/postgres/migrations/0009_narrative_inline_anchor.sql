ALTER TABLE public.narrative_nodes
    ADD COLUMN IF NOT EXISTS anchor_quote TEXT;

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'narrative_nodes_anchor_quote_length_check'
          AND conrelid = 'public.narrative_nodes'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.narrative_nodes
            ADD CONSTRAINT narrative_nodes_anchor_quote_length_check
            CHECK (
                anchor_quote IS NULL
                OR pg_catalog.char_length(anchor_quote) BETWEEN 1 AND 1000
            );
    END IF;
END
$migration$;
