-- The old production Compose path installed a public development credential.
-- Delete it only when it owns no product data; otherwise stop for explicit
-- operator remediation instead of cascading user data.

DO $migration$
DECLARE
    seed_id CONSTANT pg_catalog.uuid := '00000000-0000-0000-0000-000000000001';
    seed_hash CONSTANT pg_catalog.text := '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TiGniMnCGkzBMqVbNxoQyJXkBxKi';
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.users
        WHERE id = seed_id AND password_hash = seed_hash
    ) THEN
        IF EXISTS (SELECT 1 FROM public.novels WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.character_memories WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.chat_turns WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.chat_messages WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.user_choices WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.world_states WHERE user_id = seed_id)
            OR EXISTS (SELECT 1 FROM public.reading_progress WHERE user_id = seed_id)
        THEN
            RAISE EXCEPTION
                'known default admin credential owns product data; change its password or migrate ownership before retrying';
        END IF;

        DELETE FROM public.users WHERE id = seed_id AND password_hash = seed_hash;
    END IF;
END
$migration$;
