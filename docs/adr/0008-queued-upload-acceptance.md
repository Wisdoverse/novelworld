# ADR 0008: Accept uploads independently of parsing capacity

Status: Accepted for private single-node-v1 iteration

## Context

Long provider parsing held the same admission slot needed to upload another
book. The existing PostgreSQL import jobs already support pending, unclaimed
work, so a busy parser should not reject a new upload.

## Decision

Keep authoritative jobs and fenced claims in PostgreSQL. Use separate bounded,
short-lived upload preparation admission, and attempt parsing admission only
after atomic acceptance. Gateway limits all import bodies before buffering.
Import POST responses use `status: accepted`; current progress comes from
list/status reads. Existing claim ceilings and terminal retry semantics remain.

The browser permits 50 selected files and submits sequential requests bounded
by five files and 40 MiB. Each request is atomic; the entire selection is not.
An unknown response stops submission, refreshes the shelf, and removes unknown
files from the ordinary retry list until the user inspects the shelf. Confirmed
accepted files are never resent automatically. Every batch pins the initiating
Authorization; a token change stops later submissions.

## Consequences and evidence

No new queue broker, service, migration, or Redis dependency is introduced.
Per-owner parsing remains serial and total parsing remains bounded. The 50-file
selection is not a simultaneous-parsing or completion-time guarantee. Existing
recovery scanning is bounded to 100 candidates; multi-owner fairness at larger
backlogs and multi-replica admission remain unqualified.

Repository integration coverage proves unclaimed attempt-zero jobs survive busy
parsing and can be claimed after release; the separate startup-recovery contract
exercises the pending-job scanner. Frontend and browser fixtures exercise
50-file batching, byte limits, partial acceptance, unknown results, and locked
submission controls. These checks do not establish paid provider throughput.

## Alternatives and rollback

Keep upload rejection while parsing is busy: rejected because uploads and provider
work have different lifetimes. Add Redis: unnecessary because the existing
PostgreSQL queue already persists work and fences attempts. Expand one HTTP batch
to 50 files: rejected to retain the existing memory and transaction bounds.

Deploy compatible Gateway, Novel Service, and browser images after required CI.
Rollback those images together if upload acceptance or progress regresses. No
schema migration is required; existing pending jobs remain recoverable under the
prior worker. Old browsers ignore the response status and still poll list/status.
