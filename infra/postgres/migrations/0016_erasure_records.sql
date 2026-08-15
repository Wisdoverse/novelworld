-- Erasure records and replay for backup-restore-v1 (SPEC 12.4.1, 12.4.2).
--
-- Deleting a user or a novel writes a durable, UUID-only record in the same
-- transaction as the authoritative delete, through AFTER DELETE row triggers.
-- Row-level triggers cover every deletion path without service coordination,
-- including the per-novel records produced by an account cascade. The records
-- carry no content, no email, and no derived data, and no foreign keys, so an
-- account or novel cascade can never remove the evidence of its own deletion —
-- the same reason public.source_file_deletions has no foreign keys.

CREATE TABLE IF NOT EXISTS public.erasure_records (
    subject_type       VARCHAR(8) NOT NULL,
    subject_id         UUID NOT NULL,
    -- The owning user, so the deterministic retained-source object key
    -- source-files/{user_id}/{novel_id} stays reconstructible after the novel
    -- row is gone. Equal to subject_id for user records.
    user_id            UUID NOT NULL,
    erased_at          TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    -- Whether the deleted novel actually held a retained source object, seen by
    -- the trigger on OLD.original_file_key. Only the authoritative delete can
    -- observe this, and it is an immutable fact about the subject that travels
    -- with the record, so replay never has to guess from deployment state.
    had_source         BOOLEAN NOT NULL DEFAULT FALSE,
    -- Durable per-record bookkeeping for the exactly-once source re-queue. The
    -- self-consuming source_file_deletions outbox cannot serve as bookkeeping.
    -- A restored dump keeps its stamps: the stamp and the outbox row are written
    -- in one transaction and the dump is one snapshot, so a restored artifact
    -- either carries both (the cleanup worker re-drains, idempotently) or
    -- neither-because-already-deleted. Keeping them therefore costs zero repeats
    -- and stays inside the policy's at-most-one-repeat-per-restore allowance.
    source_requeued_at TIMESTAMPTZ,
    CONSTRAINT erasure_records_pkey PRIMARY KEY (subject_type, subject_id),
    CONSTRAINT erasure_records_subject_type_check
        CHECK (subject_type IN ('user', 'novel'))
);

-- Attest-or-erase decisions recorded by the scripted disaster restore. No
-- foreign keys: an erase decision deletes the subject it describes.
CREATE TABLE IF NOT EXISTS public.restore_attestations (
    id                 UUID PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    subject_id         UUID NOT NULL,
    decision           VARCHAR(8) NOT NULL,
    window_start       TIMESTAMPTZ NOT NULL,
    window_end         TIMESTAMPTZ NOT NULL,
    artifact_inventory TEXT NOT NULL,
    operator_identity  TEXT NOT NULL,
    -- True on the account the operator designated as administrator because the
    -- decisions would otherwise have left the installation without one.
    designated_admin   BOOLEAN NOT NULL DEFAULT FALSE,
    recorded_at        TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT restore_attestations_decision_check
        CHECK (decision IN ('retain', 'erase'))
);

-- Replay re-fires these triggers against rows it deletes. The first record for
-- a subject wins: erased_at stays the original deletion time so a restore can
-- never move a deletion fact forward.
CREATE OR REPLACE FUNCTION public.record_user_erasure()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    INSERT INTO public.erasure_records (subject_type, subject_id, user_id)
    VALUES ('user', OLD.id, OLD.id)
    ON CONFLICT (subject_type, subject_id) DO NOTHING;
    RETURN OLD;
END
$function$;

-- had_source only ever moves from false to true: a record can be written before
-- the subject row is deleted — the restore writes decision records first — and
-- the delete is the only writer that can see whether a source object existed.
CREATE OR REPLACE FUNCTION public.record_novel_erasure()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
BEGIN
    INSERT INTO public.erasure_records (subject_type, subject_id, user_id, had_source)
    VALUES ('novel', OLD.id, OLD.user_id,
            -- COALESCE is core grammar, not a schema-resolved function.
            COALESCE(OLD.original_file_key LIKE 'source-files/%', FALSE))
    ON CONFLICT (subject_type, subject_id) DO UPDATE
    SET had_source = public.erasure_records.had_source OR EXCLUDED.had_source;
    RETURN OLD;
END
$function$;

DROP TRIGGER IF EXISTS record_user_erasure ON public.users;
CREATE TRIGGER record_user_erasure
    AFTER DELETE ON public.users
    FOR EACH ROW EXECUTE FUNCTION public.record_user_erasure();

DROP TRIGGER IF EXISTS record_novel_erasure ON public.novels;
CREATE TRIGGER record_novel_erasure
    AFTER DELETE ON public.novels
    FOR EACH ROW EXECUTE FUNCTION public.record_novel_erasure();

-- ─── Erasure replay ────────────────────────────────────────────────────────
--
-- Everything below runs on every deployment, before any service starts, and is
-- idempotent: against an already-clean database it is a no-op apart from the
-- bounded bookkeeping in the third statement. The triggers above only cover
-- future deletions; deletions that happened before this migration are
-- unknowable and are deliberately not backfilled.

DELETE FROM public.users
 WHERE id IN (
     SELECT subject_id FROM public.erasure_records WHERE subject_type = 'user'
 );

DELETE FROM public.novels
 WHERE id IN (
     SELECT subject_id FROM public.erasure_records WHERE subject_type = 'novel'
 );

-- The two statements above let the existing cascades and deletion triggers
-- re-apply downstream cleanup, including queue_source_file_deletion for rows
-- that were actually present. This statement covers the remaining case: a
-- record whose subject row is not in this database at all, where the retained
-- source key can only be reconstructed from the record's UUIDs. It runs at most
-- once per record per database lineage.
--
-- The gate is the record's own had_source, recorded by the delete that could
-- still see the key, so replay never guesses from deployment state. A deployment
-- that never enabled S3 writes no had_source record and therefore enqueues no
-- speculative key — which also means it can never trip novel-service's refusal
-- to start while source_file_deletions is non-empty and S3 is disabled. A
-- had_source record restored into a lineage whose S3 is switched off does queue
-- a real pending deletion and does stop that deployment, which is exactly the
-- documented "disabling S3 while stored objects or pending deletions exist is
-- rejected" contract: an object awaiting erasure is not silently dropped.
-- Bookkeeping is stamped only when the re-queue actually happened.
WITH requeued AS (
    UPDATE public.erasure_records AS record
       SET source_requeued_at = pg_catalog.now()
     WHERE record.subject_type = 'novel'
       AND record.had_source
       AND record.source_requeued_at IS NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.novels AS subject
            WHERE subject.id = record.subject_id
       )
    RETURNING 'source-files/' || record.user_id::pg_catalog.text
              || '/' || record.subject_id::pg_catalog.text AS object_key
)
INSERT INTO public.source_file_deletions (object_key)
SELECT object_key FROM requeued
ON CONFLICT (object_key) DO NOTHING;

-- Deletion-path invariant: interactive final-account deletion clears the
-- runtime configuration so the installation returns to first-run setup
-- (services/user-service/src/infrastructure/persistence/pg_user_repo.rs).
-- Replay and attest-or-erase preserve it. The erasure-record predicate keeps a
-- database that simply has not been set up yet untouched: no user was erased,
-- so replay did not leave it empty.
DELETE FROM public.runtime_llm_config
 WHERE NOT EXISTS (SELECT 1 FROM public.users)
   AND EXISTS (
       SELECT 1 FROM public.erasure_records WHERE subject_type = 'user'
   );
