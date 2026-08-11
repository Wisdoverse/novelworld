CREATE TYPE public.identity_type AS ENUM ('self', 'character');
CREATE TYPE public.deviation_mode AS ENUM ('canon', 'creative', 'remix');

CREATE TABLE public.users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL
);

CREATE TABLE public.novels (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES public.users(id),
    total_chapters INTEGER NOT NULL DEFAULT 0,
    deviation_mode public.deviation_mode NOT NULL DEFAULT 'canon'
);

CREATE TABLE public.chapters (
    id UUID PRIMARY KEY,
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    chapter_number INTEGER NOT NULL,
    UNIQUE (novel_id, chapter_number)
);

CREATE TABLE public.characters (
    id UUID PRIMARY KEY,
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    name TEXT NOT NULL,
    first_appearance_chapter INTEGER
);

CREATE TABLE public.reading_progress (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES public.users(id),
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    current_chapter INTEGER NOT NULL DEFAULT 1,
    reader_identity VARCHAR(200),
    reader_identity_type public.identity_type NOT NULL DEFAULT 'self',
    reader_character_id UUID REFERENCES public.characters(id),
    deviation_mode public.deviation_mode NOT NULL DEFAULT 'canon',
    last_read_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

CREATE TABLE public.character_memories (
    id UUID PRIMARY KEY,
    character_id UUID NOT NULL REFERENCES public.characters(id),
    user_id UUID NOT NULL REFERENCES public.users(id),
    layer TEXT NOT NULL,
    content TEXT NOT NULL
);

CREATE TABLE public.chat_messages (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES public.users(id),
    character_id UUID NOT NULL REFERENCES public.characters(id),
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    role VARCHAR(20) NOT NULL,
    content TEXT NOT NULL,
    chapter_num INTEGER
);

CREATE TABLE public.narrative_nodes (
    id UUID PRIMARY KEY,
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    chapter_number INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    choices JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE public.user_choices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES public.users(id),
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    node_id UUID NOT NULL REFERENCES public.narrative_nodes(id),
    chapter_number INTEGER NOT NULL,
    choice_index INTEGER NOT NULL,
    choice_text TEXT NOT NULL,
    consequence TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE public.world_states (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES public.users(id),
    novel_id UUID NOT NULL REFERENCES public.novels(id),
    state JSONB NOT NULL DEFAULT '{"choices":[],"relationships":{},"world_events":[]}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, novel_id)
);

-- Same-name constraints on other relations prove migration guards are scoped.
CREATE SCHEMA decoy;

CREATE TABLE decoy.memories (
    novel_id UUID,
    CONSTRAINT character_memories_novel_id_fkey
        FOREIGN KEY (novel_id) REFERENCES public.novels(id)
);

CREATE TABLE decoy.nodes (
    novel_id UUID,
    chapter_number INTEGER,
    CONSTRAINT narrative_nodes_novel_chapter_key
        UNIQUE (novel_id, chapter_number)
);

INSERT INTO public.users (id, email, password_hash) VALUES
    ('00000000-0000-0000-0000-000000000001', 'legacy-one@test.invalid', 'changed-password'),
    ('00000000-0000-0000-0000-000000000009', 'legacy-two@test.invalid', 'changed-password');

INSERT INTO public.novels (id, user_id, total_chapters, deviation_mode) VALUES
    (
        '00000000-0000-0000-0000-000000000002',
        '00000000-0000-0000-0000-000000000001',
        7,
        'creative'
    ),
    (
        '00000000-0000-0000-0000-000000000010',
        '00000000-0000-0000-0000-000000000009',
        5,
        'canon'
    );

INSERT INTO public.chapters (id, novel_id, chapter_number) VALUES
    ('00000000-0000-0000-0000-000000000012', '00000000-0000-0000-0000-000000000002', 1),
    ('00000000-0000-0000-0000-000000000013', '00000000-0000-0000-0000-000000000002', 7),
    ('00000000-0000-0000-0000-000000000014', '00000000-0000-0000-0000-000000000010', 1),
    ('00000000-0000-0000-0000-000000000015', '00000000-0000-0000-0000-000000000010', 5);

INSERT INTO public.characters (id, novel_id, name, first_appearance_chapter)
VALUES (
    '00000000-0000-0000-0000-000000000003',
    '00000000-0000-0000-0000-000000000002',
    U&'\0009\0085\00A0Legacy Future Character\00A0\0085\0009',
    7
);

INSERT INTO public.reading_progress (
    id, user_id, novel_id, current_chapter, reader_identity,
    reader_identity_type, reader_character_id
) VALUES
    (
        '00000000-0000-0000-0000-000000000008',
        '00000000-0000-0000-0000-000000000001',
        '00000000-0000-0000-0000-000000000002',
        99,
        U&'\0009\0085\00A0Legacy Future Character\00A0\0085\0009',
        'character',
        '00000000-0000-0000-0000-000000000003'
    ),
    (
        '00000000-0000-0000-0000-000000000011',
        '00000000-0000-0000-0000-000000000009',
        '00000000-0000-0000-0000-000000000010',
        3,
        U&'\00A0Reader\00A0',
        'self',
        NULL
    );

INSERT INTO public.character_memories (id, character_id, user_id, layer, content)
VALUES (
    '00000000-0000-0000-0000-000000000004',
    '00000000-0000-0000-0000-000000000003',
    '00000000-0000-0000-0000-000000000001',
    'permanent',
    'legacy memory'
);

INSERT INTO public.chat_messages (
    id, user_id, character_id, novel_id, role, content, chapter_num
)
VALUES (
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000001',
    '00000000-0000-0000-0000-000000000003',
    '00000000-0000-0000-0000-000000000002',
    'user',
    'legacy chat',
    7
);

INSERT INTO public.narrative_nodes (id, novel_id, chapter_number)
VALUES (
    '00000000-0000-0000-0000-000000000006',
    '00000000-0000-0000-0000-000000000002',
    7
);
