-- Store one encrypted personal LLM configuration per user. user-service owns
-- this table; other services resolve it only through the internal HTTP port.
CREATE TABLE IF NOT EXISTS public.user_llm_configs (
    user_id            UUID PRIMARY KEY REFERENCES public.users(id) ON DELETE CASCADE,
    provider           VARCHAR(32) NOT NULL,
    api_url            TEXT NOT NULL,
    model              VARCHAR(200) NOT NULL,
    thinking_enabled   BOOLEAN NOT NULL DEFAULT FALSE,
    api_key_nonce      BYTEA NOT NULL,
    api_key_ciphertext BYTEA NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
