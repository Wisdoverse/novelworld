-- Durable opt-in diagnostic budget authority owned by user-service.
BEGIN;

SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

CREATE TABLE IF NOT EXISTS public.diagnostic_llm_budgets (
    budget_id              uuid PRIMARY KEY,
    contract               text NOT NULL,
    profile                text NOT NULL,
    profile_sha256         text NOT NULL CHECK (
        profile_sha256 ~ '^[0-9a-f]{64}$'
    ),
    max_attempts           bigint NOT NULL CHECK (max_attempts BETWEEN 0 AND 2000),
    max_tokens             bigint NOT NULL CHECK (max_tokens BETWEEN 0 AND 20000000),
    max_cost_micro_cny     bigint NOT NULL CHECK (max_cost_micro_cny BETWEEN 0 AND 35000000),
    charged_attempts       bigint NOT NULL DEFAULT 0 CHECK (
        charged_attempts BETWEEN 0 AND max_attempts
    ),
    charged_tokens         bigint NOT NULL DEFAULT 0 CHECK (
        charged_tokens BETWEEN 0 AND max_tokens
    ),
    charged_cost_micro_cny bigint NOT NULL DEFAULT 0 CHECK (
        charged_cost_micro_cny BETWEEN 0 AND max_cost_micro_cny
    ),
    expires_at             timestamptz NOT NULL,
    sealed                 boolean NOT NULL DEFAULT false,
    created_at             timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp()
);

CREATE TABLE IF NOT EXISTS public.diagnostic_llm_attempts (
    budget_id                  uuid NOT NULL REFERENCES public.diagnostic_llm_budgets(budget_id),
    attempt_id                 uuid NOT NULL,
    ordinal                    bigint NOT NULL CHECK (ordinal BETWEEN 1 AND 2000),
    operation                  text NOT NULL,
    output_limit               integer NOT NULL CHECK (output_limit BETWEEN 1 AND 8192),
    reservation_tokens         bigint NOT NULL CHECK (reservation_tokens BETWEEN 1 AND 1056768),
    reservation_cost_micro_cny bigint NOT NULL CHECK (reservation_cost_micro_cny BETWEEN 1 AND 3219456),
    settled                    boolean NOT NULL DEFAULT false,
    settlement_model           text,
    input_tokens               bigint CHECK (input_tokens BETWEEN 0 AND 1048576),
    output_tokens              bigint CHECK (
        output_tokens BETWEEN 0 AND output_limit
    ),
    cached_input_tokens        bigint CHECK (
        cached_input_tokens BETWEEN 0 AND input_tokens
    ),
    PRIMARY KEY (budget_id, attempt_id),
    UNIQUE (budget_id, ordinal),
    CHECK (
        (NOT settled
            AND settlement_model IS NULL
            AND input_tokens IS NULL
            AND output_tokens IS NULL
            AND cached_input_tokens IS NULL)
        OR
        (settled
            AND settlement_model IS NOT NULL
            AND input_tokens IS NOT NULL
            AND output_tokens IS NOT NULL)
    )
);

COMMIT;
