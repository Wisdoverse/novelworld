-- Prospective self-chat windows only. Historical rows remain unenrolled.
BEGIN;
SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);
ALTER TABLE public.chat_turns
    ADD COLUMN IF NOT EXISTS summary_sequence       BIGINT,
    ADD COLUMN IF NOT EXISTS summary_state          VARCHAR(16) NOT NULL DEFAULT 'none',
    ADD COLUMN IF NOT EXISTS summary_memory_id      UUID,
    ADD COLUMN IF NOT EXISTS summary_claim_attempt  BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS summary_lease_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS summary_next_attempt_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS summary_failure_code   VARCHAR(64);
ALTER TABLE public.chat_turns
    DROP CONSTRAINT IF EXISTS chat_summary_sequence_check,
    DROP CONSTRAINT IF EXISTS chat_summary_state_check;
ALTER TABLE public.chat_turns
    ADD CONSTRAINT chat_summary_sequence_check CHECK (
        summary_sequence IS NULL OR (
            summary_sequence > 0 AND status = 'completed'
            AND reader_identity_type = 'self' AND reader_character_id IS NULL
            AND persona_source_chapter_high_water IS NOT NULL
        )
    ),
    ADD CONSTRAINT chat_summary_state_check CHECK (
        summary_claim_attempt >= 0 AND (
            (summary_state = 'none'
                AND (summary_sequence IS NULL OR summary_sequence % 10 <> 0)
                AND summary_memory_id IS NULL AND summary_claim_attempt = 0
                AND summary_lease_expires_at IS NULL
                AND summary_next_attempt_at IS NULL AND summary_failure_code IS NULL)
            OR (
                summary_sequence IS NOT NULL AND summary_sequence % 10 = 0
                AND summary_memory_id IS NOT NULL AND (
                    (summary_state = 'pending' AND summary_lease_expires_at IS NULL
                        AND summary_next_attempt_at IS NOT NULL AND summary_failure_code IS NULL)
                    OR (summary_state IN ('claimed', 'dispatched') AND summary_claim_attempt > 0
                        AND summary_lease_expires_at IS NOT NULL
                        AND summary_next_attempt_at IS NULL AND summary_failure_code IS NULL)
                    OR (summary_state = 'saved' AND summary_claim_attempt > 0
                        AND summary_lease_expires_at IS NULL
                        AND summary_next_attempt_at IS NULL AND summary_failure_code IS NULL)
                    OR (summary_state IN ('failed', 'unknown') AND summary_claim_attempt > 0
                        AND summary_lease_expires_at IS NULL AND summary_next_attempt_at IS NULL
                        AND summary_failure_code IS NOT NULL
                        AND summary_failure_code IN ('source_invalid', 'output_invalid',
                            'eligibility_changed', 'dispatch_unknown'))
                )
            )
        )
    );
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_summary_sequence
    ON public.chat_turns(user_id, novel_id, character_id, summary_sequence)
    WHERE summary_sequence IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_chat_summary_due
    ON public.chat_turns(summary_next_attempt_at, id)
    WHERE summary_state = 'pending';
CREATE INDEX IF NOT EXISTS idx_chat_summary_leases
    ON public.chat_turns(summary_lease_expires_at, id)
    WHERE summary_state IN ('claimed', 'dispatched');
COMMIT;
