ALTER TABLE public.canon_extraction_checkpoints
    DROP CONSTRAINT canon_extraction_checkpoints_source_content_check;

ALTER TABLE public.canon_extraction_checkpoints
    ADD CONSTRAINT canon_extraction_checkpoints_source_content_check
    CHECK (pg_catalog.octet_length(source_content) BETWEEN 1 AND 16000);
