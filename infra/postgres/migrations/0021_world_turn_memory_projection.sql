-- A completed world-turn response is not declared terminal until its eligible
-- character-memory projection is durably saved or explicitly skipped. This
-- lets exact replays remain available during an Agent outage after a prior ack.
BEGIN;

DO $migration$
DECLARE
    projection_status_exists BOOLEAN;
    projection_completed_at_exists BOOLEAN;
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    SELECT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_attribute
        WHERE attrelid = 'public.world_turns'::pg_catalog.regclass
          AND attname = 'memory_projection_status'
          AND attnum > 0
          AND NOT attisdropped
    ) INTO projection_status_exists;
    SELECT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_attribute
        WHERE attrelid = 'public.world_turns'::pg_catalog.regclass
          AND attname = 'memory_projection_completed_at'
          AND attnum > 0
          AND NOT attisdropped
    ) INTO projection_completed_at_exists;

    IF projection_status_exists IS DISTINCT FROM projection_completed_at_exists THEN
        RAISE EXCEPTION 'world turn memory projection columns are only partially installed';
    END IF;

    IF NOT projection_status_exists THEN
        ALTER TABLE public.world_turns
            ADD COLUMN memory_projection_status VARCHAR(16),
            ADD COLUMN memory_projection_completed_at TIMESTAMPTZ;

        -- A pre-contract completed turn has no trustworthy character-witness
        -- provenance. Mark it honestly terminal instead of requiring a client
        -- to retain an old key or fabricating a structured fact. Non-terminal
        -- legacy rows remain pending for the new writer's normal fencing path.
        UPDATE public.world_turns
        SET memory_projection_status = CASE
                WHEN status = 'completed' THEN 'skipped'
                ELSE 'pending'
            END,
            memory_projection_completed_at = CASE
                WHEN status = 'completed' THEN pg_catalog.clock_timestamp()
                ELSE NULL
            END;

        ALTER TABLE public.world_turns
            ALTER COLUMN memory_projection_status SET DEFAULT 'pending',
            ALTER COLUMN memory_projection_status SET NOT NULL;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM (
            VALUES
                (
                    'memory_projection_status'::pg_catalog.text,
                    'character varying(16)'::pg_catalog.text,
                    TRUE,
                    '''pending''::character varying'::pg_catalog.text
                ),
                (
                    'memory_projection_completed_at',
                    'timestamp with time zone',
                    FALSE,
                    NULL
                )
        ) AS expected(column_name, type_name, is_not_null, default_expression)
        LEFT JOIN pg_catalog.pg_attribute AS attribute
          ON attribute.attrelid = 'public.world_turns'::pg_catalog.regclass
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
        RAISE EXCEPTION 'world turn memory projection columns have an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'world_turns_memory_projection_status_check'
          AND conrelid = 'public.world_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.world_turns
            ADD CONSTRAINT world_turns_memory_projection_status_check
            CHECK (memory_projection_status IN ('pending', 'saved', 'skipped'));
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'world_turns_memory_projection_state_check'
          AND conrelid = 'public.world_turns'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.world_turns
            ADD CONSTRAINT world_turns_memory_projection_state_check
            CHECK (
                (memory_projection_status = 'pending'
                    AND memory_projection_completed_at IS NULL)
                OR (memory_projection_status IN ('saved', 'skipped')
                    AND status = 'completed'
                    AND memory_projection_completed_at IS NOT NULL)
            );
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'world_turns_memory_projection_status_check'
          AND conrelid = 'public.world_turns'::pg_catalog.regclass
          AND contype::pg_catalog.text = 'c'
          AND convalidated
          AND pg_catalog.pg_get_constraintdef(oid) IN (
                'CHECK (((memory_projection_status)::text = ANY ((ARRAY[''pending''::character varying, ''saved''::character varying, ''skipped''::character varying])::text[])))',
                'CHECK (((memory_projection_status)::text = ANY (ARRAY[(''pending''::character varying)::text, (''saved''::character varying)::text, (''skipped''::character varying)::text])))'
          )
    ) THEN
        RAISE EXCEPTION 'world turn memory projection status constraint has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint
        WHERE conname = 'world_turns_memory_projection_state_check'
          AND conrelid = 'public.world_turns'::pg_catalog.regclass
          AND contype::pg_catalog.text = 'c'
          AND convalidated
          AND pg_catalog.pg_get_constraintdef(oid) IN (
                'CHECK (((((memory_projection_status)::text = ''pending''::text) AND (memory_projection_completed_at IS NULL)) OR (((memory_projection_status)::text = ANY ((ARRAY[''saved''::character varying, ''skipped''::character varying])::text[])) AND ((status)::text = ''completed''::text) AND (memory_projection_completed_at IS NOT NULL))))',
                'CHECK (((((memory_projection_status)::text = ''pending''::text) AND (memory_projection_completed_at IS NULL)) OR (((memory_projection_status)::text = ANY (ARRAY[(''saved''::character varying)::text, (''skipped''::character varying)::text])) AND ((status)::text = ''completed''::text) AND (memory_projection_completed_at IS NOT NULL))))'
          )
    ) THEN
        RAISE EXCEPTION 'world turn memory projection state constraint has an unexpected definition';
    END IF;

    IF pg_catalog.to_regclass('public.idx_world_turns_one_in_progress') IS NOT NULL
       AND NOT EXISTS (
            SELECT 1
            FROM pg_catalog.pg_index AS index_definition
            WHERE index_definition.indexrelid =
                      pg_catalog.to_regclass('public.idx_world_turns_one_in_progress')
              AND index_definition.indrelid = 'public.world_turns'::pg_catalog.regclass
              AND index_definition.indisunique
              AND index_definition.indisvalid
              AND index_definition.indisready
              AND index_definition.indislive
              AND pg_catalog.pg_get_indexdef(index_definition.indexrelid) =
                  'CREATE UNIQUE INDEX idx_world_turns_one_in_progress ON public.world_turns USING btree (user_id, novel_id) WHERE (((status)::text = ''in_progress''::text) OR (((status)::text = ''completed''::text) AND ((memory_projection_status)::text = ''pending''::text)))'
       )
    THEN
        DROP INDEX public.idx_world_turns_one_in_progress;
    END IF;

    IF pg_catalog.to_regclass('public.idx_world_turns_one_in_progress') IS NULL THEN
        CREATE UNIQUE INDEX idx_world_turns_one_in_progress
            ON public.world_turns(user_id, novel_id)
            WHERE status = 'in_progress'
               OR (status = 'completed' AND memory_projection_status = 'pending');
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_index AS index_definition
        WHERE index_definition.indexrelid =
                  pg_catalog.to_regclass('public.idx_world_turns_one_in_progress')
          AND index_definition.indrelid = 'public.world_turns'::pg_catalog.regclass
          AND index_definition.indisunique
          AND index_definition.indisvalid
          AND index_definition.indisready
          AND index_definition.indislive
          AND pg_catalog.pg_get_indexdef(index_definition.indexrelid) =
              'CREATE UNIQUE INDEX idx_world_turns_one_in_progress ON public.world_turns USING btree (user_id, novel_id) WHERE (((status)::text = ''in_progress''::text) OR (((status)::text = ''completed''::text) AND ((memory_projection_status)::text = ''pending''::text)))'
    )
    THEN
        RAISE EXCEPTION 'world turn unresolved authority index has an unexpected definition';
    END IF;
END
$migration$;

-- Pre-contract prose remains stored so migration never guesses at witness
-- provenance or irreversibly deletes a colliding legacy memory. Agent prompt
-- consumers quarantine the former producer class (permanent, importance 7,
-- UUID version nibble 4). Account export and explicit deletion retain their
-- normal authority over those rows; no replacement historical fact is invented.

COMMIT;
