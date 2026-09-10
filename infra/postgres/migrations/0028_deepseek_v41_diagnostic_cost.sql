BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
ALTER TABLE public.diagnostic_llm_attempts
    DROP CONSTRAINT IF EXISTS diagnostic_llm_attempts_reservation_cost_micro_cny_check,
    ADD CONSTRAINT diagnostic_llm_attempts_reservation_cost_micro_cny_check
        CHECK (reservation_cost_micro_cny BETWEEN 1 AND 4292608);
COMMIT;
