-- Store the one operator-selected text model configuration. The API key is
-- encrypted by user-service before it reaches PostgreSQL.
CREATE TABLE IF NOT EXISTS public.runtime_llm_config (
    singleton          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    provider           VARCHAR(32) NOT NULL,
    api_url            TEXT NOT NULL,
    model              VARCHAR(200) NOT NULL,
    api_key_nonce      BYTEA NOT NULL,
    api_key_ciphertext BYTEA NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
