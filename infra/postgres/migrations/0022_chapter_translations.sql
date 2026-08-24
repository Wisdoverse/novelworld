-- Source-bound chapter translations are shared across readers. A lease fences
-- provider work across replicas; failed attempts observe a retry cooldown.
BEGIN;

SELECT pg_catalog.set_config('search_path', 'pg_catalog', true);

CREATE TABLE IF NOT EXISTS public.chapter_translations (
    chapter_id          pg_catalog.uuid NOT NULL,
    source_sha256       pg_catalog.bytea NOT NULL,
    profile             pg_catalog.varchar(64) NOT NULL,
    status              pg_catalog.varchar(16) NOT NULL,
    attempt             pg_catalog.int8 NOT NULL DEFAULT 1,
    lease_expires_at    pg_catalog.timestamptz,
    retry_after_at      pg_catalog.timestamptz,
    translated_content  pg_catalog.text,
    failure_code        pg_catalog.varchar(64),
    created_at          pg_catalog.timestamptz NOT NULL DEFAULT pg_catalog.now(),
    updated_at          pg_catalog.timestamptz NOT NULL DEFAULT pg_catalog.now(),
    completed_at        pg_catalog.timestamptz,
    CONSTRAINT chapter_translations_pkey
        PRIMARY KEY (chapter_id, source_sha256, profile),
    CONSTRAINT chapter_translations_chapter_id_fkey
        FOREIGN KEY (chapter_id) REFERENCES public.chapters(id) ON DELETE CASCADE,
    CONSTRAINT chapter_translations_source_sha256_check
        CHECK (pg_catalog.octet_length(source_sha256) = 32),
    CONSTRAINT chapter_translations_profile_check
        CHECK (pg_catalog.char_length(profile) BETWEEN 1 AND 64),
    CONSTRAINT chapter_translations_status_check
        CHECK (status IN ('translating', 'ready', 'failed')),
    CONSTRAINT chapter_translations_attempt_check CHECK (attempt >= 1),
    CONSTRAINT chapter_translations_state_check CHECK (
        (
            status = 'translating'
            AND lease_expires_at IS NOT NULL
            AND retry_after_at IS NULL
            AND translated_content IS NULL
            AND failure_code IS NULL
            AND completed_at IS NULL
        )
        OR (
            status = 'ready'
            AND lease_expires_at IS NULL
            AND retry_after_at IS NULL
            AND translated_content IS NOT NULL
            AND translated_content <> ''
            AND failure_code IS NULL
            AND completed_at IS NOT NULL
        )
        OR (
            status = 'failed'
            AND lease_expires_at IS NULL
            AND retry_after_at IS NOT NULL
            AND translated_content IS NULL
            AND failure_code IS NOT NULL
            AND failure_code <> ''
            AND completed_at IS NULL
        )
    )
);

DO $translation_contract$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM (
            VALUES
                ('chapter_id', 'uuid', TRUE, NULL::pg_catalog.text),
                ('source_sha256', 'bytea', TRUE, NULL::pg_catalog.text),
                ('profile', 'character varying(64)', TRUE, NULL::pg_catalog.text),
                ('status', 'character varying(16)', TRUE, NULL::pg_catalog.text),
                ('attempt', 'bigint', TRUE, '1'),
                ('lease_expires_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text),
                ('retry_after_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text),
                ('translated_content', 'text', FALSE, NULL::pg_catalog.text),
                ('failure_code', 'character varying(64)', FALSE, NULL::pg_catalog.text),
                ('created_at', 'timestamp with time zone', TRUE, 'now()'),
                ('updated_at', 'timestamp with time zone', TRUE, 'now()'),
                ('completed_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text)
        ) AS expected(attname, type_name, is_not_null, default_expression)
        LEFT JOIN pg_catalog.pg_attribute AS actual
          ON actual.attrelid = 'public.chapter_translations'::pg_catalog.regclass
         AND actual.attname = expected.attname
         AND actual.attnum > 0
         AND NOT actual.attisdropped
        LEFT JOIN pg_catalog.pg_attrdef AS actual_default
          ON actual_default.adrelid = actual.attrelid
         AND actual_default.adnum = actual.attnum
        WHERE actual.attname IS NULL
           OR pg_catalog.format_type(actual.atttypid, actual.atttypmod)
                  IS DISTINCT FROM expected.type_name
           OR actual.attnotnull IS DISTINCT FROM expected.is_not_null
           OR pg_catalog.pg_get_expr(actual_default.adbin, actual_default.adrelid)
                  IS DISTINCT FROM expected.default_expression
    ) THEN
        RAISE EXCEPTION 'chapter translations columns have unexpected definitions';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint AS translation_pk
        WHERE translation_pk.conrelid =
                  'public.chapter_translations'::pg_catalog.regclass
          AND translation_pk.contype::pg_catalog.text = 'p'
          AND translation_pk.convalidated
          AND NOT translation_pk.condeferrable
          AND NOT translation_pk.condeferred
          AND translation_pk.conkey = ARRAY[
                (SELECT attnum FROM pg_catalog.pg_attribute
                 WHERE attrelid = translation_pk.conrelid AND attname = 'chapter_id'),
                (SELECT attnum FROM pg_catalog.pg_attribute
                 WHERE attrelid = translation_pk.conrelid AND attname = 'source_sha256'),
                (SELECT attnum FROM pg_catalog.pg_attribute
                 WHERE attrelid = translation_pk.conrelid AND attname = 'profile')
          ]
    ) THEN
        RAISE EXCEPTION 'chapter translations primary key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint AS translation_fk
        WHERE translation_fk.conrelid =
                  'public.chapter_translations'::pg_catalog.regclass
          AND translation_fk.confrelid = 'public.chapters'::pg_catalog.regclass
          AND translation_fk.contype::pg_catalog.text = 'f'
          AND translation_fk.confdeltype::pg_catalog.text = 'c'
          AND translation_fk.convalidated
          AND translation_fk.conkey = ARRAY[(
                SELECT attnum FROM pg_catalog.pg_attribute
                WHERE attrelid = translation_fk.conrelid AND attname = 'chapter_id'
          )]
          AND translation_fk.confkey = ARRAY[(
                SELECT attnum FROM pg_catalog.pg_attribute
                WHERE attrelid = translation_fk.confrelid AND attname = 'id'
          )]
    ) THEN
        RAISE EXCEPTION 'chapter translations chapter foreign key has an unexpected definition';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM (
            VALUES
                ('chapter_translations_source_sha256_check',
                 'CHECK ((octet_length(source_sha256) = 32))'),
                ('chapter_translations_profile_check',
                 'CHECK (((char_length((profile)::text) >= 1) AND (char_length((profile)::text) <= 64)))'),
                ('chapter_translations_attempt_check',
                 'CHECK ((attempt >= 1))'),
                ('chapter_translations_state_check',
                 'CHECK (((((status)::text = ''translating''::text) AND (lease_expires_at IS NOT NULL) AND (retry_after_at IS NULL) AND (translated_content IS NULL) AND (failure_code IS NULL) AND (completed_at IS NULL)) OR (((status)::text = ''ready''::text) AND (lease_expires_at IS NULL) AND (retry_after_at IS NULL) AND (translated_content IS NOT NULL) AND (translated_content <> ''''::text) AND (failure_code IS NULL) AND (completed_at IS NOT NULL)) OR (((status)::text = ''failed''::text) AND (lease_expires_at IS NULL) AND (retry_after_at IS NOT NULL) AND (translated_content IS NULL) AND (failure_code IS NOT NULL) AND ((failure_code)::text <> ''''::text) AND (completed_at IS NULL))))')
        ) AS expected(conname, definition)
        LEFT JOIN pg_catalog.pg_constraint AS actual
          ON actual.conrelid = 'public.chapter_translations'::pg_catalog.regclass
         AND actual.conname = expected.conname
        WHERE actual.oid IS NULL
           OR actual.contype::pg_catalog.text <> 'c'
           OR NOT actual.convalidated
           OR pg_catalog.pg_get_constraintdef(actual.oid, FALSE) <> expected.definition
    ) THEN
        RAISE EXCEPTION 'chapter translations check constraints have unexpected definitions';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint AS status_check
        WHERE status_check.conrelid =
                  'public.chapter_translations'::pg_catalog.regclass
          AND status_check.conname = 'chapter_translations_status_check'
          AND status_check.contype::pg_catalog.text = 'c'
          AND status_check.convalidated
          AND pg_catalog.pg_get_constraintdef(status_check.oid, FALSE) IN (
                'CHECK (((status)::text = ANY ((ARRAY[''translating''::character varying, ''ready''::character varying, ''failed''::character varying])::text[])))',
                'CHECK (((status)::text = ANY (ARRAY[(''translating''::character varying)::text, (''ready''::character varying)::text, (''failed''::character varying)::text])))'
          )
    ) THEN
        RAISE EXCEPTION 'chapter translations status constraint has an unexpected definition';
    END IF;
END
$translation_contract$;

COMMIT;
