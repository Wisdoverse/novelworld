ALTER TABLE public.runtime_llm_config
    ADD COLUMN IF NOT EXISTS thinking_enabled BOOLEAN NOT NULL DEFAULT FALSE;
