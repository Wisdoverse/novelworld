-- Make a narrative choice idempotent and bind it to the same novel/chapter as
-- its node. Existing ambiguous or corrupt rows fail closed for operator review.

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF EXISTS (
        SELECT 1
        FROM public.user_choices
        GROUP BY user_id, node_id
        HAVING pg_catalog.count(*) > 1
    ) THEN
        RAISE EXCEPTION 'cannot enforce narrative choice idempotency: duplicate (user_id, node_id) rows';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM public.user_choices AS choice
        JOIN public.narrative_nodes AS node ON node.id = choice.node_id
        WHERE choice.novel_id <> node.novel_id
           OR choice.chapter_number <> node.chapter_number
    ) THEN
        RAISE EXCEPTION 'cannot enforce narrative choice scope: choice novel/chapter differs from its node';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM public.user_choices
        WHERE chapter_number < 1 OR choice_index < 0 OR choice_text = ''
    ) THEN
        RAISE EXCEPTION 'cannot enforce narrative choice bounds: invalid legacy row';
    END IF;
END
$migration$;

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'narrative_nodes_identity_key'
          AND conrelid = 'public.narrative_nodes'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.narrative_nodes
            ADD CONSTRAINT narrative_nodes_identity_key
            UNIQUE(id, novel_id, chapter_number);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_user_node_key'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.user_choices
            ADD CONSTRAINT user_choices_user_node_key UNIQUE(user_id, node_id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_node_scope_fkey'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.user_choices
            ADD CONSTRAINT user_choices_node_scope_fkey
            FOREIGN KEY(node_id, novel_id, chapter_number)
            REFERENCES public.narrative_nodes(id, novel_id, chapter_number)
            ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_chapter_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.user_choices
            ADD CONSTRAINT user_choices_chapter_check CHECK (chapter_number >= 1);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_index_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.user_choices
            ADD CONSTRAINT user_choices_index_check CHECK (choice_index >= 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_text_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
    ) THEN
        ALTER TABLE public.user_choices
            ADD CONSTRAINT user_choices_text_check CHECK (choice_text <> '');
    END IF;
END
$migration$;

DO $migration$
BEGIN
    PERFORM pg_catalog.set_config('search_path', 'public,pg_catalog', true);

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'narrative_nodes_identity_key'
          AND conrelid = 'public.narrative_nodes'::pg_catalog.regclass
          AND contype = 'u'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'UNIQUE (id, novel_id, chapter_number)'
    ) THEN
        RAISE EXCEPTION 'narrative_nodes_identity_key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_user_node_key'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
          AND contype = 'u'
          AND pg_catalog.pg_get_constraintdef(oid) = 'UNIQUE (user_id, node_id)'
    ) THEN
        RAISE EXCEPTION 'user_choices_user_node_key has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_node_scope_fkey'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
          AND contype = 'f'
          AND pg_catalog.pg_get_constraintdef(oid) =
              'FOREIGN KEY (node_id, novel_id, chapter_number) REFERENCES narrative_nodes(id, novel_id, chapter_number) ON DELETE CASCADE'
    ) THEN
        RAISE EXCEPTION 'user_choices_node_scope_fkey has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_chapter_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) = 'CHECK ((chapter_number >= 1))'
    ) THEN
        RAISE EXCEPTION 'user_choices_chapter_check has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_index_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) = 'CHECK ((choice_index >= 0))'
    ) THEN
        RAISE EXCEPTION 'user_choices_index_check has an unexpected definition';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_constraint
        WHERE conname = 'user_choices_text_check'
          AND conrelid = 'public.user_choices'::pg_catalog.regclass
          AND contype = 'c'
          AND pg_catalog.pg_get_constraintdef(oid) = 'CHECK ((choice_text <> ''''::text))'
    ) THEN
        RAISE EXCEPTION 'user_choices_text_check has an unexpected definition';
    END IF;
END
$migration$;
