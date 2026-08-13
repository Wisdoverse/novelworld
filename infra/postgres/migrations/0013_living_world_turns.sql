CREATE TABLE IF NOT EXISTS public.world_turns (
    id                   UUID PRIMARY KEY,
    user_id              UUID NOT NULL,
    novel_id             UUID NOT NULL,
    request_fingerprint  BYTEA NOT NULL,
    action               JSONB NOT NULL,
    expected_turn_number BIGINT NOT NULL,
    status               VARCHAR(16) NOT NULL,
    attempt              BIGINT NOT NULL DEFAULT 1,
    lease_expires_at     TIMESTAMPTZ,
    transition           JSONB,
    result               JSONB,
    failure_code         VARCHAR(64),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    completed_at         TIMESTAMPTZ,
    CONSTRAINT world_turns_world_state_fkey
        FOREIGN KEY(user_id, novel_id)
        REFERENCES public.world_states(user_id, novel_id) ON DELETE CASCADE,
    CONSTRAINT world_turns_request_fingerprint_check
        CHECK (pg_catalog.octet_length(request_fingerprint) = 32),
    CONSTRAINT world_turns_action_check
        CHECK (pg_catalog.jsonb_typeof(action) = 'object'),
    CONSTRAINT world_turns_expected_turn_check CHECK (expected_turn_number >= 0),
    CONSTRAINT world_turns_status_check
        CHECK (status IN ('in_progress', 'completed', 'failed')),
    CONSTRAINT world_turns_attempt_check CHECK (attempt >= 1),
    CONSTRAINT world_turns_state_check CHECK (
        (
            status = 'in_progress'
            AND lease_expires_at IS NOT NULL
            AND transition IS NULL
            AND result IS NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'completed'
            AND lease_expires_at IS NULL
            AND pg_catalog.jsonb_typeof(transition) = 'object'
            AND pg_catalog.jsonb_typeof(result) = 'object'
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
        OR (
            status = 'failed'
            AND lease_expires_at IS NULL
            AND transition IS NULL
            AND result IS NULL
            AND failure_code IS NOT NULL
            AND failure_code <> ''
            AND completed_at IS NULL
        )
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_world_turns_one_in_progress
    ON public.world_turns(user_id, novel_id) WHERE status = 'in_progress';
CREATE INDEX IF NOT EXISTS idx_world_turns_journal
    ON public.world_turns(user_id, novel_id, completed_at DESC)
    WHERE status = 'completed';
