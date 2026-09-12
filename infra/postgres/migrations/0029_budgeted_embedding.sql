BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

ALTER TABLE public.diagnostic_llm_attempts
    DROP CONSTRAINT IF EXISTS diagnostic_llm_attempts_output_limit_check;

ALTER TABLE public.diagnostic_llm_attempts
    ADD CONSTRAINT diagnostic_llm_attempts_output_limit_check
    CHECK (output_limit BETWEEN 0 AND 8192);

COMMIT;
