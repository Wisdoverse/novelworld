BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

-- Preserve every existing row, immutable content, and consumed claim. New
-- prompt variants share the same canonical model's cumulative claim budget.
-- Pre-0030 Novel writers cannot use this primary key: deploy compatible
-- Novel/Narrative services together and recover forward after migration.
ALTER TABLE public.novel_game_rule_templates
    DROP CONSTRAINT IF EXISTS novel_game_rule_templates_pkey;
ALTER TABLE public.novel_game_rule_templates
    ADD CONSTRAINT novel_game_rule_templates_pkey
    PRIMARY KEY (novel_id, canon_model_version, prompt_version);

COMMIT;
