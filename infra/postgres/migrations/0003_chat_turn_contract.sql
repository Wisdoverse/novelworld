-- Add a privacy-preserving idempotency ledger for durable chat turns.
-- Legacy messages remain unlinked: their turn boundaries cannot be recovered safely.

CREATE TABLE IF NOT EXISTS public.chat_turns (
    id                     pg_catalog.uuid PRIMARY KEY,
    user_id                pg_catalog.uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    character_id           pg_catalog.uuid NOT NULL REFERENCES public.characters(id) ON DELETE CASCADE,
    novel_id               pg_catalog.uuid NOT NULL REFERENCES public.novels(id) ON DELETE CASCADE,
    request_fingerprint    pg_catalog.bytea NOT NULL,
    chapter_context        pg_catalog.int4 NOT NULL,
    reader_identity        pg_catalog.varchar(200),
    reader_identity_type   public.identity_type NOT NULL,
    reader_character_id    pg_catalog.uuid REFERENCES public.characters(id) ON DELETE CASCADE,
    deviation_mode         public.deviation_mode NOT NULL,
    status                 pg_catalog.varchar(16) NOT NULL,
    attempt                pg_catalog.int8 NOT NULL DEFAULT 1,
    lease_expires_at       pg_catalog.timestamptz,
    failure_code           pg_catalog.varchar(64),
    created_at             pg_catalog.timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at             pg_catalog.timestamptz NOT NULL DEFAULT pg_catalog.now(),
    completed_at           pg_catalog.timestamptz,
    CONSTRAINT chat_turns_request_fingerprint_check
        CHECK (pg_catalog.octet_length(request_fingerprint) = 32),
    CONSTRAINT chat_turns_chapter_context_check CHECK (chapter_context >= 1),
    CONSTRAINT chat_turns_status_check
        CHECK (status IN ('in_progress', 'completed', 'failed')),
    CONSTRAINT chat_turns_attempt_check CHECK (attempt >= 1),
    CONSTRAINT chat_turns_identity_fields_check CHECK (
        (reader_identity_type = 'self' AND reader_character_id IS NULL)
        OR (
            reader_identity_type = 'character'
            AND reader_character_id IS NOT NULL
            AND reader_identity IS NOT NULL
        )
    ),
    CONSTRAINT chat_turns_state_check CHECK (
        (
            status = 'in_progress'
            AND lease_expires_at IS NOT NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'completed'
            AND lease_expires_at IS NULL
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
        OR (
            status = 'failed'
            AND lease_expires_at IS NULL
            AND failure_code IS NOT NULL
            AND failure_code <> ''
            AND completed_at IS NULL
        )
    )
);

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF EXISTS (
        SELECT 1
        FROM (
            VALUES
                ('id'::pg_catalog.text, 'uuid'::pg_catalog.text, TRUE, NULL::pg_catalog.text),
                ('user_id', 'uuid', TRUE, NULL),
                ('character_id', 'uuid', TRUE, NULL),
                ('novel_id', 'uuid', TRUE, NULL),
                ('request_fingerprint', 'bytea', TRUE, NULL),
                ('chapter_context', 'integer', TRUE, NULL),
                ('reader_identity', 'character varying(200)', FALSE, NULL),
                ('reader_identity_type', 'identity_type', TRUE, NULL),
                ('reader_character_id', 'uuid', FALSE, NULL),
                ('deviation_mode', 'deviation_mode', TRUE, NULL),
                ('status', 'character varying(16)', TRUE, NULL),
                ('attempt', 'bigint', TRUE, '1'),
                ('lease_expires_at', 'timestamp with time zone', FALSE, NULL),
                ('failure_code', 'character varying(64)', FALSE, NULL),
                ('created_at', 'timestamp with time zone', TRUE, 'now()'),
                ('updated_at', 'timestamp with time zone', TRUE, 'now()'),
                ('completed_at', 'timestamp with time zone', FALSE, NULL)
        ) AS expected(column_name, type_name, is_not_null, default_expression)
        LEFT JOIN pg_catalog.pg_attribute AS attribute
          ON attribute.attrelid = 'public.chat_turns'::pg_catalog.regclass
         AND attribute.attname = expected.column_name
         AND attribute.attnum > 0
         AND NOT attribute.attisdropped
        LEFT JOIN pg_catalog.pg_attrdef AS column_default
          ON column_default.adrelid = attribute.attrelid
         AND column_default.adnum = attribute.attnum
        WHERE attribute.attname IS NULL
           OR pg_catalog.format_type(attribute.atttypid, attribute.atttypmod)
                <> expected.type_name
           OR attribute.attnotnull IS DISTINCT FROM expected.is_not_null
           OR attribute.attidentity <> ''
           OR attribute.attgenerated <> ''
           OR pg_catalog.pg_get_expr(column_default.adbin, column_default.adrelid)
                IS DISTINCT FROM expected.default_expression
    ) THEN
        RAISE EXCEPTION 'chat turns columns have an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_pkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_pkey PRIMARY KEY (id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_user_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_character_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_character_id_fkey
            FOREIGN KEY (character_id) REFERENCES public.characters(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_novel_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_novel_id_fkey
            FOREIGN KEY (novel_id) REFERENCES public.novels(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_reader_character_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_reader_character_id_fkey
            FOREIGN KEY (reader_character_id) REFERENCES public.characters(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_request_fingerprint_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_request_fingerprint_check
            CHECK (pg_catalog.octet_length(request_fingerprint) = 32);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_chapter_context_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_chapter_context_check CHECK (chapter_context >= 1);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_status_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_status_check
            CHECK (status IN ('in_progress', 'completed', 'failed'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_attempt_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_attempt_check CHECK (attempt >= 1);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_identity_fields_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_identity_fields_check CHECK (
                (reader_identity_type = 'self' AND reader_character_id IS NULL)
                OR (
                    reader_identity_type = 'character'
                    AND reader_character_id IS NOT NULL
                    AND reader_identity IS NOT NULL
                )
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_state_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_turns
            ADD CONSTRAINT chat_turns_state_check CHECK (
                (
                    status = 'in_progress'
                    AND lease_expires_at IS NOT NULL
                    AND failure_code IS NULL
                    AND completed_at IS NULL
                )
                OR (
                    status = 'completed'
                    AND lease_expires_at IS NULL
                    AND failure_code IS NULL
                    AND completed_at IS NOT NULL
                )
                OR (
                    status = 'failed'
                    AND lease_expires_at IS NULL
                    AND failure_code IS NOT NULL
                    AND failure_code <> ''
                    AND completed_at IS NULL
                )
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_pkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'p'
          AND pg_catalog.pg_get_constraintdef(oid) = 'PRIMARY KEY (id)'
    ) THEN
        RAISE EXCEPTION 'chat turns primary key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_user_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'chat turns user foreign key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_character_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (character_id) REFERENCES characters(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'chat turns character foreign key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_novel_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (novel_id) REFERENCES novels(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'chat turns novel foreign key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_reader_character_id_fkey'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (reader_character_id) REFERENCES characters(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'chat turns reader character foreign key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_request_fingerprint_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'CHECK ((octet_length(request_fingerprint) = 32))'
    ) THEN
        RAISE EXCEPTION 'chat turns request fingerprint constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_chapter_context_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) = 'CHECK ((chapter_context >= 1))'
    ) THEN
        RAISE EXCEPTION 'chat turns chapter constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_status_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          -- Two spellings of one constraint. PostgreSQL deparses
          -- CHECK (status IN (...)) over a varchar column as the first form,
          -- and re-parses that deparsed text — which is exactly what restoring
          -- a pg_dump artifact does — into the second. Accepting both keeps a
          -- restored deployment migratable without loosening drift detection:
          -- only one of them can ever match a given constraint.
          AND pg_catalog.pg_get_constraintdef(oid) IN (
              'CHECK (((status)::text = ANY ((ARRAY[''in_progress''::character varying, ''completed''::character varying, ''failed''::character varying])::text[])))',
              'CHECK (((status)::text = ANY (ARRAY[(''in_progress''::character varying)::text, (''completed''::character varying)::text, (''failed''::character varying)::text])))'
          )
    ) THEN
        RAISE EXCEPTION 'chat turns status constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_attempt_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) = 'CHECK ((attempt >= 1))'
    ) THEN
        RAISE EXCEPTION 'chat turns attempt constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_identity_fields_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'CHECK ((((reader_identity_type = ''self''::identity_type) AND (reader_character_id IS NULL)) OR ((reader_identity_type = ''character''::identity_type) AND (reader_character_id IS NOT NULL) AND (reader_identity IS NOT NULL))))'
    ) THEN
        RAISE EXCEPTION 'chat turns identity constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_turns_state_check'
          AND conrelid = 'public.chat_turns'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'CHECK (((((status)::text = ''in_progress''::text) AND (lease_expires_at IS NOT NULL) AND (failure_code IS NULL) AND (completed_at IS NULL)) OR (((status)::text = ''completed''::text) AND (lease_expires_at IS NULL) AND (failure_code IS NULL) AND (completed_at IS NOT NULL)) OR (((status)::text = ''failed''::text) AND (lease_expires_at IS NULL) AND (failure_code IS NOT NULL) AND ((failure_code)::text <> ''''::text) AND (completed_at IS NULL))))'
    ) THEN
        RAISE EXCEPTION 'chat turns state constraint has an unexpected definition';
    END IF;
END
$migration$;

DO $migration$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM public.chat_turns
        WHERE status = 'in_progress'
        GROUP BY user_id, character_id, novel_id
        HAVING pg_catalog.count(*) > 1
    ) THEN
        RAISE EXCEPTION
            'cannot enforce one in-progress chat turn: duplicate conversations exist';
    END IF;
END
$migration$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_turns_one_in_progress
    ON public.chat_turns(user_id, character_id, novel_id)
    WHERE status = 'in_progress';

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_index AS index_definition
        JOIN pg_catalog.pg_class AS index_relation
          ON index_relation.oid = index_definition.indexrelid
        JOIN pg_catalog.pg_am AS access_method
          ON access_method.oid = index_relation.relam
        WHERE index_relation.relnamespace = 'public'::pg_catalog.regnamespace
          AND index_relation.relname = 'idx_chat_turns_one_in_progress'
          AND index_definition.indrelid = 'public.chat_turns'::pg_catalog.regclass
          AND index_definition.indisunique
          AND index_definition.indisvalid
          AND index_definition.indisready
          AND index_definition.indnkeyatts = 3
          AND index_definition.indnatts = 3
          AND access_method.amname = 'btree'
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid, 1, true) = 'user_id'
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid, 2, true) =
              'character_id'
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid, 3, true) = 'novel_id'
          AND pg_catalog.pg_get_expr(
                  index_definition.indpred,
                  index_definition.indrelid
              ) = '((status)::text = ''in_progress''::text)'
    ) THEN
        RAISE EXCEPTION 'chat turns one-in-progress index has an unexpected definition';
    END IF;
END
$migration$;

ALTER TABLE public.chat_messages ADD COLUMN IF NOT EXISTS turn_id pg_catalog.uuid;

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_attribute AS attribute
        WHERE attribute.attrelid = 'public.chat_messages'::pg_catalog.regclass
          AND attribute.attname = 'turn_id'
          AND attribute.attnum > 0
          AND NOT attribute.attisdropped
          AND pg_catalog.format_type(attribute.atttypid, attribute.atttypmod) = 'uuid'
          AND NOT attribute.attnotnull
          AND attribute.attidentity = ''
          AND attribute.attgenerated = ''
          AND NOT EXISTS (
              SELECT 1
              FROM pg_catalog.pg_attrdef AS column_default
              WHERE column_default.adrelid = attribute.attrelid
                AND column_default.adnum = attribute.attnum
          )
    ) THEN
        RAISE EXCEPTION 'chat messages turn column has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_messages_turn_id_fkey'
          AND conrelid = 'public.chat_messages'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.chat_messages
            ADD CONSTRAINT chat_messages_turn_id_fkey
            FOREIGN KEY (turn_id) REFERENCES public.chat_turns(id) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'chat_messages_turn_id_fkey'
          AND conrelid = 'public.chat_messages'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (turn_id) REFERENCES chat_turns(id) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'chat messages turn foreign key has an unexpected definition';
    END IF;
END
$migration$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_messages_turn_role_unique
    ON public.chat_messages(turn_id, role)
    WHERE turn_id IS NOT NULL;

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_index AS index_definition
        JOIN pg_catalog.pg_class AS index_relation
          ON index_relation.oid = index_definition.indexrelid
        JOIN pg_catalog.pg_am AS access_method
          ON access_method.oid = index_relation.relam
        WHERE index_relation.relnamespace = 'public'::pg_catalog.regnamespace
          AND index_relation.relname = 'idx_chat_messages_turn_role_unique'
          AND index_definition.indrelid = 'public.chat_messages'::pg_catalog.regclass
          AND index_definition.indisunique
          AND index_definition.indisvalid
          AND index_definition.indisready
          AND index_definition.indnkeyatts = 2
          AND index_definition.indnatts = 2
          AND access_method.amname = 'btree'
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid, 1, true) = 'turn_id'
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid, 2, true) = 'role'
          AND pg_catalog.pg_get_expr(index_definition.indpred, index_definition.indrelid) =
              '(turn_id IS NOT NULL)'
    ) THEN
        RAISE EXCEPTION 'chat messages turn/role index has an unexpected definition';
    END IF;
END
$migration$;
