-- One immutable, source-bound rules template is shared by every reader of the
-- same canonical novel model. Generation rows use a fenced expiring lease.
CREATE TABLE IF NOT EXISTS public.novel_game_rule_templates (
    novel_id            UUID NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    canon_model_version INTEGER NOT NULL,
    schema_version      INTEGER NOT NULL,
    prompt_version      VARCHAR(100) NOT NULL,
    status              VARCHAR(16) NOT NULL,
    attempt             BIGINT NOT NULL DEFAULT 1,
    lease_expires_at    TIMESTAMPTZ,
    content             JSONB,
    failure_code        VARCHAR(64),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    completed_at        TIMESTAMPTZ,
    PRIMARY KEY (novel_id, canon_model_version),
    CONSTRAINT novel_game_rule_templates_model_fkey
        FOREIGN KEY (novel_id, canon_model_version)
        REFERENCES public.canon_story_models(novel_id, model_version) ON DELETE CASCADE,
    CONSTRAINT novel_game_rule_templates_version_check CHECK (
        canon_model_version >= 1 AND schema_version >= 1
        AND pg_catalog.char_length(prompt_version) BETWEEN 1 AND 100
    ),
    CONSTRAINT novel_game_rule_templates_status_check
        CHECK (status IN ('generating', 'ready', 'failed')),
    CONSTRAINT novel_game_rule_templates_attempt_check CHECK (attempt >= 1),
    CONSTRAINT novel_game_rule_templates_state_check CHECK (
        (status = 'generating' AND lease_expires_at IS NOT NULL
            AND content IS NULL AND failure_code IS NULL AND completed_at IS NULL)
        OR (status = 'ready' AND lease_expires_at IS NULL
            AND pg_catalog.jsonb_typeof(content) = 'object' AND failure_code IS NULL
            AND completed_at IS NOT NULL)
        OR (status = 'failed' AND lease_expires_at IS NULL
            AND content IS NULL AND failure_code IS NOT NULL
            AND failure_code <> '' AND completed_at IS NULL)
    )
);

CREATE OR REPLACE FUNCTION public.reject_ready_game_rule_template_update()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    IF OLD.status = 'ready' THEN
        RAISE EXCEPTION 'ready game rule templates are immutable' USING ERRCODE = '55000';
    END IF;
    RETURN NEW;
END
$function$;

DROP TRIGGER IF EXISTS reject_ready_game_rule_template_update
    ON public.novel_game_rule_templates;
CREATE TRIGGER reject_ready_game_rule_template_update
    BEFORE UPDATE ON public.novel_game_rule_templates
    FOR EACH ROW EXECUTE FUNCTION public.reject_ready_game_rule_template_update();

-- The authoritative D20 result is stored on the in-progress turn before the
-- provider renders prose, then replayed with the completed ledger entry.
ALTER TABLE public.world_turns
    ADD COLUMN IF NOT EXISTS resolution JSONB;

DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'world_turns_resolution_check'
          AND conrelid = 'public.world_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.world_turns
            ADD CONSTRAINT world_turns_resolution_check
            CHECK (resolution IS NULL OR pg_catalog.jsonb_typeof(resolution) = 'object');
    END IF;
END
$migration$;
