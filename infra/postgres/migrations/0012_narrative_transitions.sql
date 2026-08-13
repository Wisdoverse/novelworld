ALTER TABLE public.user_choices
    ADD COLUMN IF NOT EXISTS transition JSONB;

UPDATE public.user_choices
SET consequence = '历史选择已提交'
WHERE consequence IS NULL OR consequence = '';

UPDATE public.user_choices
SET transition = pg_catalog.jsonb_build_object(
    'schema_version', 1,
    'prompt_version', 'legacy-prose-v1',
    'canon_model_version', COALESCE((
        SELECT pg_catalog.max(model_version)
        FROM public.canon_story_models
        WHERE novel_id = user_choices.novel_id
    ), 0),
    'canonical_checkpoint_chapter', chapter_number,
    'rendered_narrative', consequence,
    'events', pg_catalog.jsonb_build_array(pg_catalog.jsonb_build_object(
        'summary', '历史选择已提交',
        'actor_character_ids', '[]'::jsonb,
        'location_id', NULL
    )),
    'relationship_changes', '[]'::jsonb,
    'location_changes', '[]'::jsonb,
    'thread_changes', '[]'::jsonb
)
WHERE transition IS NULL;

UPDATE public.user_choices
SET transition = transition || pg_catalog.jsonb_build_object(
    'canon_model_version', COALESCE((
        SELECT pg_catalog.max(model_version)
        FROM public.canon_story_models
        WHERE novel_id = user_choices.novel_id
    ), 0),
    'canonical_checkpoint_chapter', chapter_number
)
WHERE NOT transition ? 'canon_model_version'
   OR NOT transition ? 'canonical_checkpoint_chapter';

ALTER TABLE public.user_choices
    ALTER COLUMN consequence SET NOT NULL,
    ALTER COLUMN transition SET NOT NULL;

ALTER TABLE public.user_choices
    DROP CONSTRAINT IF EXISTS user_choices_consequence_check,
    DROP CONSTRAINT IF EXISTS user_choices_transition_check,
    DROP CONSTRAINT IF EXISTS user_choices_transition_projection_check,
    ADD CONSTRAINT user_choices_consequence_check CHECK (consequence <> ''),
    ADD CONSTRAINT user_choices_transition_check CHECK (
        pg_catalog.jsonb_typeof(transition) = 'object'
        AND transition @> '{"schema_version": 1}'::jsonb
        AND pg_catalog.jsonb_typeof(transition -> 'prompt_version') = 'string'
        AND pg_catalog.jsonb_typeof(transition -> 'canon_model_version') = 'number'
        AND pg_catalog.jsonb_typeof(transition -> 'canonical_checkpoint_chapter') = 'number'
        AND pg_catalog.jsonb_typeof(transition -> 'rendered_narrative') = 'string'
        AND pg_catalog.jsonb_typeof(transition -> 'events') = 'array'
        AND pg_catalog.jsonb_typeof(transition -> 'relationship_changes') = 'array'
        AND pg_catalog.jsonb_typeof(transition -> 'location_changes') = 'array'
        AND pg_catalog.jsonb_typeof(transition -> 'thread_changes') = 'array'
    ),
    ADD CONSTRAINT user_choices_transition_projection_check CHECK (
        transition ->> 'rendered_narrative' = consequence
    );
