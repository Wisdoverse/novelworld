CREATE TABLE IF NOT EXISTS public.canon_story_models (
    id              UUID PRIMARY KEY DEFAULT public.uuid_generate_v4(),
    novel_id        UUID NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    model_version   INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL,
    prompt_version  VARCHAR(100) NOT NULL,
    content         JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT canon_story_models_novel_version_key UNIQUE(novel_id, model_version),
    CONSTRAINT canon_story_models_model_version_check CHECK (model_version >= 1),
    CONSTRAINT canon_story_models_schema_version_check CHECK (schema_version >= 1),
    CONSTRAINT canon_story_models_prompt_version_check
        CHECK (char_length(prompt_version) BETWEEN 1 AND 100),
    CONSTRAINT canon_story_models_content_check CHECK (jsonb_typeof(content) = 'object')
);

CREATE OR REPLACE FUNCTION public.reject_canon_story_model_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    RAISE EXCEPTION 'canon story models are immutable' USING ERRCODE = '55000';
END
$function$;

DROP TRIGGER IF EXISTS reject_canon_story_model_update ON public.canon_story_models;
CREATE TRIGGER reject_canon_story_model_update
    BEFORE UPDATE ON public.canon_story_models
    FOR EACH ROW EXECUTE FUNCTION public.reject_canon_story_model_update();
