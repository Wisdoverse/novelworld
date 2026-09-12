# ADR 0005: Durable prospective chat summary windows

- Status: Accepted decision; delivery tracked in [#328](https://github.com/Wisdoverse/novelworld/issues/328)
- Date: 2026-09-08

## Context

Chat commits precede derived-memory work. The former detached producer checked
whether the current message count was divisible by twenty and summarized the
latest twenty messages. An interrupted or failed boundary task had no durable
obligation; a later boundary could omit the earlier window completely while
all authoritative chat remained in PostgreSQL.

## Decision

Agent owns this lifecycle on existing `chat_turns`. Its fenced chat-completion
transaction assigns each new valid self turn an immutable per-user/novel/
character sequence. Every tenth turn registers a pending summary and a random
UUIDv4 result ID in that same transaction. Its exact source interval is the ten
sequences ending at that anchor, with twenty paired completed messages. Every
source must prove scope, self identity, role, chapter and persona provenance.
The summary preserves the maximum actual source chapter and persona marker
separately. Neither rewind nor later chat changes that interval.

Migration 0027 leaves historical turns unenrolled. Each scope starts with its
first ten newly committed self turns; no historical summary coverage is
inferred and no partial historical window is backfilled. Character-mode turns
remain unenrolled. Existing valid summaries and chat retain their existing
retrieval/export/deletion rules.

One bounded Agent worker examines ten due anchors per five-second pass,
sequentially, reusing per-user chat admission. Ineligible or occupied principals
are durably deferred thirty seconds. A fifteen-second claimed lease and
monotonic attempt fence elect a known-unsent owner. Expired unsent claims may
resume. Before provider work, exact compare-and-set records `dispatched` with a
330-second lease. An ambiguous dispatch acknowledgement makes no provider call.
Once dispatched, the window never authorizes another logical summary call.

The existing MemorySummary adapter, prompt, 256-token output ceiling and
transport retries remain. An outer 300-second deadline includes runtime
configuration resolution. Successful output must be nonempty after trimming
and at most 4,000 Unicode characters; invalid output is terminal, not repaired
or regenerated. The worker checks current authoritative character ID, shelf
access, self identity, persona visibility and progress before dispatch and
before publication. A post-call eligibility failure is terminal. The final
HTTP check and PG transaction are separate; cross-service linearizability is
not claimed.

The result transaction locks the current unexpired dispatched fence,
revalidates the exact PG sources, inserts the fixed Mid ID without an UPSERT,
and marks the window saved atomically. Stale owners cannot publish. A lost save
acknowledgement leaves either the one saved result or an explicit unresolved
outcome; it never permits regeneration. Dispatched errors, interruption and
expiry retain terminal `unknown` state. Invalid source/output or changed
eligibility retain terminal failure. No automatic reset/retry endpoint exists.
These states retain an operational explanation, not a guarantee of recovery
from arbitrary provider failure or exactly-once HTTP.

Both chat APIs keep their completed-response semantics and a cache-only
post-commit projection, bounded at two seconds. Cache failure or a tombstone
cannot erase pending PG work or authorize a paid call. Shelf detachment leaves
work ineligible until authoritative access returns; account deletion cascades
both windows and memory, including a late-save race. Completed-key replay
neither schedules summaries nor invokes the provider.

Long promotion is optional immediately after the sole proven Mid commit. Its
whole operation is bounded at 305 seconds, including the existing embedding
call and a three-second PG save. It has no application retry or restart
recovery, and failure cannot undo Mid success. Diagnostic embedding refusal
and durable allowance reservation/settlement remain unchanged. Summary
recovery never provisions/refills allowance or releases unknown reservations.

Shutdown stops new scans at the signal and grants the owned worker five
seconds before aborting it. PG state determines later recovery; dispatched
work cannot become unsent because the process stopped.

## Compatibility and evidence limits

The additive migration works with the existing supported release path, which
stops old Agent before migration. Desktop also embeds migration 0027. Rolling
back to an older Agent suspends this new availability guarantee: it may run
its previous best-effort producer and creates unsequenced chat. Pending and
terminal records survive, but no cross-version summary deduplication or
backfill of rollback-period chat is promised. Re-upgrade resumes only eligible
known-unsent windows.

The tests cover prospective enrollment, retained source windows, fencing,
unknown outcomes, lost acknowledgements, scope/progress boundaries and
lifecycle deletion with synthetic model adapters. This is structural evidence,
not live semantic quality, complete four-layer continuity, a qualified provider,
or H4 release-upgrade acceptance. An existing journey that upgrades after
seven chats cannot assume the first new summary arrives at total chat ten;
any new paid fixture requires its own reviewed registration.
