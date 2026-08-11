# NovelWorld Long-Term Engineering Roadmap

This roadmap orders work by risk and evidence, not by feature count or calendar
promises. A horizon starts only when the previous horizon's exit criteria are
measured and met.

## Product and system invariants

These remain true through every horizon:

1. Authenticated identity comes only from Gateway-verified JWT claims. Request
   bodies, paths, and query strings never choose the acting user.
2. A user can access only novels and derived resources they own. Cross-service
   calls carry that identity and fail closed when ownership cannot be proven.
3. Domain code has no infrastructure or HTTP dependencies. Service data is
   reached through its owning service, not another service's tables.
4. A conversation turn is durable only after both messages are stored. A
   narrative choice is idempotent and cannot be attached to a different novel.
5. Spoiler boundaries use server-side reading progress; the browser cannot move
   the model's knowledge boundary independently of persisted progress.
6. Every externally visible contract has a regression check. Required CI gates
   cannot be allowed to fail.
7. Changes are observable and reversible before they are scaled.

## Horizon 0: restore the trust boundary

Current evidence shows resource endpoints that trust caller-supplied `user_id`
and novel reads that do not verify ownership. Fix these before adding features.

- Derive the acting user from `X-User-Id` on every downstream endpoint.
- Enforce novel ownership before returning novels, chapters, characters,
  relationships, parse status, progress, memories, or world state.
- Propagate the acting user on agent-service and narrative-service calls to
  novel-service.
- Reject character/novel and narrative-node/novel mismatches.
- Remove caller-supplied identity fields from browser and API request contracts.
- Put token issuance and chat completion behind domain ports; application code
  must not import concrete JWT or LLM adapters.
- Add focused regression checks for the principal and ownership invariants.

Exit criteria: the protected-resource matrix is owner-scoped, forged identity
fields are rejected, targeted tests pass, and workspace checks introduce no new
failure.

## Horizon 1: make the current product reliable

- Replace string-matched application errors with typed domain/application
  errors and the specified stable API error envelope.
- Bound pagination, message size, chapter position, and all LLM prompt inputs at
  trust boundaries.
- Make chat persistence and narrative choice/world-state mutation atomic or
  explicitly recoverable and idempotent.
- Make setup state durable and truthful across restarts; never imply that an API
  key was saved when it was not.
- Repair the existing formatting baseline, make formatting and Clippy required
  CI gates, and run real PostgreSQL/Redis integration tests in CI.
- Add readiness checks that verify dependencies separately from liveness.

Exit criteria: required CI is green, restart/failure-path tests pass, and the
documented API matches runtime behavior.

## Horizon 2: complete the core reader loop

- Replace the reader's mock branch node with the narrative-service contract.
- Persist and use authenticated reading progress and reader identity.
- Complete history, memory controls, narrative consequence rendering, and
  accessible loading/error/retry states in the existing FSD slices.
- Move ingestion to durable background work with explicit status transitions,
  retry policy, cancellation, and idempotency.
- Store uploaded source files only when retention/reprocessing requirements are
  defined; otherwise keep direct extraction and document the limit.

Exit criteria: upload -> parse -> read -> chat -> choose -> resume works after a
process restart and is covered by an end-to-end test.

## Horizon 3: quality, safety, and cost controls

- Version prompts and structured-output schemas; validate all model output.
- Build offline evaluation sets for extraction quality, character consistency,
  spoiler leakage, memory relevance, and narrative coherence.
- Add per-provider latency, error, retry, token, and cost metrics with budgets.
- Add deletion/export workflows and explicit retention for source text,
  messages, memories, and generated assets.
- Threat-model prompt injection, untrusted files, SSRF-capable provider config,
  model data leakage, and abuse; close findings with regression tests.

Exit criteria: releases meet defined eval, privacy, security, latency, and cost
budgets rather than relying on manual impressions.

## Horizon 4: scale only the measured bottleneck

- Establish SLOs and capacity tests for upload throughput, stream concurrency,
  database latency, Redis memory, and LLM-provider saturation.
- Split PostgreSQL ownership physically only when contention, isolation, or
  independent operations justify migration cost.
- Add a durable queue only when ingestion reliability or measured concurrency
  exceeds the in-process worker design.
- Add replicas, partitioning, CDN/object storage, or orchestration only against a
  measured bottleneck and with a rollback plan.

Exit criteria: load and failure-injection tests demonstrate the target SLOs at
the required traffic and data volume.

## Explicit non-goals until evidence changes

- No Kafka/event bus for synchronous request flows.
- No Kubernetes service mesh or multi-region control plane.
- No second vector database while pgvector meets measured recall and latency.
- No generic plugin framework, workflow engine, or repository abstraction with
  a single consumer.

Each becomes eligible only when a named SLO, compliance requirement, or measured
bottleneck cannot be met by the current platform.
