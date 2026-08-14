# NovelWorld Threat Model

## Overview

NovelWorld is a self-hosted, multi-user web application that accepts novels and
turns them into interactive worlds. Its production topology is an Nginx edge,
a React browser client, an Axum Gateway, four data-owning Rust services,
PostgreSQL, Redis, and operator-selected model/image providers. The browser
reaches only Nginx and the Gateway. Downstream services
trust identity headers inserted by a Gateway that has verified a JWT.

The current supported profile is the private self-hosted preview defined in the
[`product contract`](./PRODUCT_CONTRACT.md). Modeling an Internet attacker or a
public edge is defensive analysis, not approval for public hosting.

The primary runtime is:

```text
Browser -> Nginx -> Gateway
                     |-> user-service
                     |-> novel-service -> document parsers -> model/image providers
                     |-> agent-service -> Redis/PostgreSQL -> model/embedding provider
                     `-> narrative-service -> PostgreSQL -> model provider
```

The production code is under `gateway/`, `services/`, `crates/llm-client/`,
`frontend/`, and `infra/`. `tests/` exercises production contracts. The offline
Horizon 3 evaluator and LLM-budget verifier under `tools/` are release controls,
not remotely exposed product services. Developer launchers, Compose files, and
GitHub workflows are part of the build and deployment trust boundary.

### Assets and security objectives

The assets that require protection are:

- Password hashes, access and refresh tokens, JWT signing material, the runtime
  configuration encryption key, provider API keys, database/Redis credentials,
  S3 credentials, and the internal service token.
- Private source novels, chapters, characters, chat messages, character
  memories, reading progress and identity, choices, world state, generated
  player chapters, open-world sessions and turn journals, and account exports.
- Canon provenance and spoiler boundaries. Generated or reader-specific content
  must not silently replace source-backed canon or expose chapters beyond the
  reader's server-side progress.
- PostgreSQL and Redis integrity, durable idempotency records, and atomic chat,
  narrative, and world-turn commits.
- Provider spend and the availability of the edge, parsers, database pools,
  Redis, and model-dependent operations.
- Release provenance: the reviewed Git revision, immutable production image
  digests, migration order, and the required CI gates.

The principal security objectives are authenticated identity, per-user and
per-novel authorization, secret confidentiality, bounded processing of
attacker-controlled content, fail-closed state transitions, output/data
separation, privacy export/deletion completeness, and reproducible releases.

## Threat Model, Trust Boundaries, and Assumptions

### Actors and input control

Attacker-controlled input includes:

- Unauthenticated registration, login, refresh, first-run setup attempts,
  request paths and headers, and public health/metrics traffic.
- Authenticated reader requests, resource UUIDs, progress updates, chat text,
  narrative choices, display metadata, pasted novels, and uploaded TXT, EPUB,
  and PDF bytes.
- Novel text, archive metadata, markup, model prompts derived from it, and all
  model/provider responses. A novel author or provider can intentionally return
  instruction-like text, malformed structured output, URLs, or oversized data.
- Browser-visible persisted content and filenames. A reader can arrange for
  their own content to be rendered, downloaded, streamed, or exported later.

Operator-controlled input includes environment variables, Compose overrides,
TLS and reverse-proxy configuration, database/Redis access, provider selection,
operator-only model settings, provider base URLs supplied through privileged
environment configuration, secrets, backups, logs, and host access. The first
web setup flow creates the initial administrator and accepts only the built-in
provider choices defined by the runtime configuration domain model.

Developer-controlled input includes source and dependency changes, migrations,
GitHub workflow definitions, release scripts, Dockerfiles, lockfiles, test
fixtures, and published image/tag inputs. Pull-request and registry compromise
belongs to this boundary, not to the remote reader boundary.

The external model, image, and embedding providers are independent parties.
Their output is untrusted application data, while their internal storage,
retention, moderation, and compromise are external risks that NovelWorld can
limit but cannot enforce.

### Trust boundaries

1. **Internet to Nginx/Gateway.** Production Compose exposes Nginx only. Nginx
   forwards `/api/` to the Gateway and has SSE-specific proxy behavior. The
   Gateway owns public routing, body forwarding, JWT verification, identity
   header replacement, and error normalization. Nginx owns per-client admission;
   on protected routes the Gateway authenticates before applying its global
   token-bucket backstop. Gateway observation routes are exempt from that global
   bucket, while Nginx API admission still applies; production Nginx does not
   expose metrics. CORS is permissive in the Rust routers; it is not an
   authorization control.

2. **Browser to authenticated API.** Access and opaque refresh tokens are kept
   in browser `localStorage` and bearer headers rather than cookies. This avoids
   ambient-cookie CSRF but makes any genuine same-origin script execution a
   session-confidentiality event. React's normal text escaping is a control;
   URLs, navigation, downloads, Blob handling, and any future raw-HTML use still
   require explicit review. Chat Markdown suppresses model-authored image loads.

3. **Gateway to downstream services.** The Gateway strips the authority to
   choose an acting user from public input by overwriting `X-User-Id` and
   `X-User-Role` with verified JWT claims. Most downstream product routes rely
   on those headers and network placement instead of revalidating JWTs. Internal
   runtime configuration, erasure, and account-export endpoints also require a
   shared service token. Directly publishing a downstream port would cross an
   assumption and could turn forged identity headers into an authorization
   bypass.

4. **Authenticated user to owned resources.** Readers are mutually untrusted.
   Novel, chapter, character, progress, chat, memory, narrative, world-state,
   generated-chapter, open-world, export, and deletion operations must bind both the acting
   user and the target resource. UUID unpredictability is not authorization.
   Agent and narrative services use novel-service HTTP adapters to prove novel
   ownership instead of reading another service's logical data directly.

5. **Uploaded or pasted content to parsing and ingestion.** Multipart framing,
   MIME/extension/magic detection, ZIP structure, XML/HTML parsing, PDF parsing,
   chapter splitting, and model extraction are representation boundaries.
   Current controls include request and format-specific byte limits, EPUB entry
   count, per-entry and aggregate-expanded-byte limits, duplicate-spine
   rejection, bounded extracted text, EPUB spine resolution, parser admission,
   and archive-entry lookup without filesystem extraction. Blocking parser work
   is isolated from asynchronous request workers. These controls do not prove
   safety inside a third-party parser.

6. **Application to model providers.** Provider requests carry valuable API
   keys and private content across a network boundary. Web-managed settings are
   restricted to built-in provider URLs; environment URLs are privileged
   operator configuration. Shared LLM code applies process-wide admission,
   connect/total deadlines, aggregate response limits, retry limits, bounded
   Retry-After handling, and stable provider errors. Prompt delimiters and
   behavioral instructions reduce
   accidental instruction confusion but are not an authorization mechanism.
   Structured responses must pass JSON and domain validation before durable
   state changes; free-form chat remains untrusted display data.
   Open-world prompts label the novel, action, session, and state as untrusted
   data. The model proposes bounded typed changes over IDs in a persisted entry
   snapshot; it cannot select the acting player or commit.

7. **Services to PostgreSQL, Redis, and S3.** All services currently share one
   PostgreSQL deployment, but ownership is logical and enforced by service APIs
   and user-scoped parameterized queries. Redis is a projection/cache, not the
   only durable copy of account data. Database credentials, schema privileges,
   migrations, backups, and a compromised service process are a stronger trust
   boundary than a normal reader. When enabled, novel-service alone writes
   private source objects to S3 using server-generated UUID paths. S3 endpoint,
   bucket, region, and credentials are operator-controlled; readiness fails when
   the configured bucket is unavailable.

8. **Administrator setup and runtime configuration.** Initial setup is public
   only while no administrator exists and persists the administrator, refresh
   token, and optional encrypted model configuration atomically. Later model
   settings require an administrator principal. Provider keys stored in
   PostgreSQL are encrypted with the operator-supplied AES-GCM key. A malicious
   administrator or a host process that can read secrets is already privileged;
   an ordinary reader reaching these operations is not.

9. **Developer/CI to release.** Required CI checks Rust formatting, compilation,
   tests, Clippy, frontend type/lint/test/build, real PostgreSQL/Redis adapters,
   Windows launchers, offline model-quality/budget contracts, and production
   Compose behavior. Release manifests bind the Git SHA and production images
   to digests. GitHub Actions, registries, Rust/npm dependencies, base images,
   and maintainers remain supply-chain dependencies.

### Assumptions and explicit external boundaries

- The production topology does not expose service, PostgreSQL, or Redis ports to
  untrusted networks; Nginx/Gateway is the only public application edge.
- Operators generate strong unique secrets, terminate TLS for non-local use,
  protect the host and `.env`, restrict backups/logs, and do not deliberately
  configure a hostile provider. HTTPS confidentiality is an operator/deployment
  requirement, not supplied by the HTTP-only example Nginx configuration.
- Host-root compromise, a malicious operator, arbitrary database administrator
  access, container-runtime escape, and theft of all process environment are
  outside the normal remote-attacker model. Application behavior that grants
  those capabilities to a lesser actor remains in scope.
- Provider-side retention, training, insider access, outages, and provider-hosted
  image bytes are external. Sending data or secrets to an attacker-selected
  provider origin, logging them locally, or leaking them between users remains
  in scope.
- Operator backups are outside application-layer export/deletion transactions.
  Redis projections and service-local account-export snapshots have the limits
  documented in `docs/DATA_RETENTION.md` and `docs/ACCOUNT_EXPORT.md`.
- Self-authored prompt injection that changes only the attacker's own fictional
  conversation is lower impact. Crossing authorization, spoiler, secret,
  provider-spend, or durable canon/world-state boundaries is security relevant.
- The version footer identifies the clean-main security-review baseline. This
  model also describes the remediation delivered with the same Horizon 3
  change; it does not assert that a particular host has deployed either revision
  or is configured safely.

## Attack Surface, Mitigations, and Attacker Stories

### Edge routing, authentication, and sessions

The public entry points are the explicit Gateway routes in `gateway/src/main.rs`.
`gateway/src/auth.rs` fixes JWT verification to HS256 and validates expiry through
`jsonwebtoken`; the auth middleware replaces identity headers. User-service
hashes passwords with bcrypt cost 12 in a bounded blocking pool, validates
opaque refresh-token shape, stores refresh tokens server-side with expiry, and
atomically consumes and replaces them on refresh. The first-admin transaction
prevents a second setup winner after configuration.

Realistic attacker stories include credential stuffing, expensive bcrypt work,
stolen bearer/refresh tokens, malformed proxy paths, forged identity headers,
and attempts to call protected siblings through wildcard routes. The relevant
controls are route classification, exact principal injection, ownership checks,
bounded bodies, stable error envelopes, refresh-token lifecycle, and admission
limits. CORS, UUIDs, client-side route guards, and global rate limiting alone do
not establish authorization or fair-use isolation.

### Object authorization and service ownership

Every user-controlled resource identifier can be an IDOR source. The sensitive
sinks are reads, writes, model prompts, exports, deletion, and state transitions
over another user's novel-derived graph. `novel-service` owns ownership proofs;
agent/narrative HTTP adapters carry the acting user. Persistence uses bound SQL
parameters and user/novel predicates, while choice commits and chat turns use
transactions and idempotency records.

The audit must compare sibling operations, including list/detail/delete/retry,
chapter/character/relationship/progress, chat/history/memory, branch/world-state,
account export, and erasure. A single checked parent does not prove a later UUID
belongs to that parent. Physical use of one PostgreSQL database is not permission
for cross-service table reads or unscoped records.

### File upload and parsing

`services/novel-service/src/infrastructure/document.rs` accepts only TXT, EPUB,
and PDF and bounds uploaded and extracted bytes. EPUB processing limits entry
count, container/package/chapter sizes, follows the package spine, and reads ZIP
entries into memory without writing archive paths to disk. The HTTP layer limits
the complete request and metadata. Accepted files are written to S3 only after
format validation, under a key that never includes the attacker-controlled
filename. A delayed cleanup intent covers crashes between S3 and PostgreSQL;
novel/account deletion uses a durable outbox so database cascades cannot orphan
objects.

Relevant stories are ZIP bombs with misleading declared sizes, duplicate or
path-like ZIP names, XML/HTML parser edge cases, PDFs that consume excessive CPU
or memory despite a small byte size, invalid UTF-8, decompression amplification,
and content crafted to poison downstream prompts. Extension or MIME agreement
alone is insufficient. Parser errors must be non-secret, work must release
resources, and rejected content must not enter a partial durable state.

### LLM prompts, outputs, and provider networking

Novel text, chat messages, memories, lore, and model output can contain hostile
instructions. Prompt construction in the services labels untrusted sections and
uses a server-side reading-progress boundary. Narrative and canon extraction
parse structured JSON and apply domain validation before transaction commits.
Chat streams distinguish completion from failure and commit both messages before
emitting the durable completion event.

The realistic security stories are not merely a model leaving character. They
are injected content causing cross-user/source disclosure, bypassing server-side
spoiler limits, producing a durable transition outside allowed bounds, leaking
system/secret data present in a prompt, or multiplying provider spend. Model text
cannot directly authorize a database or HTTP operation. Any code that treats it
as a resource ID, URL, role, executable instruction, or already-trusted HTML
creates a stronger boundary and must validate it independently.

Provider base URLs and API keys are sensitive SSRF/exfiltration surfaces.
Built-in web provider configuration is allowlisted in the runtime-config domain;
operator environment overrides are privileged and should not be misreported as
reader-controlled SSRF. The audit must still trace redirects, connection tests,
error bodies, telemetry, and all places a key or private prompt could cross to a
different origin. Provider responses and errors remain untrusted even over TLS.

### Browser rendering and downloads

The React client renders novel, chat, narrative, and provider-returned strings.
React text interpolation escapes markup, and no security conclusion should rely
on that if a value reaches an `href`, navigation API, CSS, raw HTML, or another
interpreter. Provider setup links are application constants and use
`rel="noreferrer"`. Account export creates a Blob URL only after it observes the
versioned completion record and revokes the URL afterward.

A stored XSS would be high impact because browser storage contains both token
classes. Blob content is downloaded rather than executed, but its filename,
MIME, completion marker, and field serialization remain trust boundaries.
Open redirects, `javascript:` URLs, reverse-tabnabbing, formula injection in a
future CSV export, and unsafe Markdown/HTML rendering become relevant only where
the corresponding sink exists.

### Privacy, export, deletion, and logging

Account export is Gateway-composed NDJSON from four service-owned, internal-token
authenticated streams. Explicit DTO allowlists exclude password hashes, tokens,
runtime keys, idempotency fingerprints, embeddings, and internal errors. A final
record is the completeness proof; service-local snapshots are intentionally not
a distributed backup. Export concurrency and elapsed time are bounded.

Account deletion is a service-coordinated application transaction with documented
external limits. Relevant stories include exporting/deleting another principal,
omitting indirectly owned records, including another user's canonical/branch
data, future secret columns leaking through broad serialization, partial success
presented as complete, and sensitive provider bodies or prompts reaching logs.
Error normalization, field allowlists, sentinel-secret tests, and explicit
retention documentation are controls; they must be maintained as schemas grow.
S3 object keys are excluded from export, deletion intents survive account
cascade, and adapter errors must never expose credentials.

### Availability and economic abuse

Nginx applies a per-client API token bucket before the Gateway; on protected
routes the Gateway authenticates before applying its global backstop. Export,
document parsing, bcrypt, imports,
chats, and provider calls use small native admission limits. Imports also reject
plans above a fixed model-call budget, effective chapters advance one generated
chapter per request, and request/parser/model layers enforce byte, deadline,
retry, and token ceilings. World turns allow one active claim per user/novel,
renew a bounded lease, and share provider admission. Readiness checks are cached and Gateway observation
probes do not consume the shared global bucket.

Relevant stories include one actor consuming global admission capacity, many
accounts coordinating expensive uploads or bcrypt/model operations, retry
amplification, slow streaming clients, database transaction/connection pinning,
Redis failure, pathological parsers, and repeated account export. The security
question is whether a plausible lesser actor can cause sustained cross-user
unavailability or unbounded provider cost. A merely slow operation on the
attacker's own request, finite documented capacity, or an operator choosing an
undersized host is not automatically a vulnerability.

### Persistence, internal APIs, and deployment

SQLx bindings mitigate SQL injection, but authorization predicates, transaction
scope, JSONB validation, migration constraints, and service ownership remain
separate controls. Internal endpoints and identity headers rely on the network
boundary and shared secret; logs and public errors must never turn dependency
details into secret disclosure.

Production Compose keeps state services and Rust services on an internal network
and exposes Nginx. Release tooling validates an allowlisted manifest and immutable
image digests. Realistic deployment stories include accidentally publishing an
internal port, weak/reused environment secrets, running without TLS, restoring a
stale or hostile backup, compromised dependencies/actions/registry artifacts,
or bypassing required CI. These are operator/developer boundary failures unless
the application makes the unsafe state silent or remotely reachable by default.

## Severity Calibration (Critical, High, Medium, Low)

Severity is based on the least-privileged realistic attacker, actual
source-to-sink reachability, affected users/assets, prerequisites, and existing
controls. Missing evidence of a live deployment lowers confidence, not an
otherwise proven source vulnerability.

### Critical

Use Critical only for an immediately actionable path to system-wide catastrophic
impact with practical reachability, for example:

- An unauthenticated upload or request leading to arbitrary code execution in a
  production service and access to database/provider/JWT secrets.
- A remotely exploitable signing or setup flaw that lets any internet attacker
  mint administrator tokens or take over an unconfigured and supposedly guarded
  installation after an administrator already exists.
- A release-path compromise in repository-controlled logic that deterministically
  substitutes attacker images for every production service while passing the
  recorded provenance checks.

Do not use Critical for behavior requiring an already malicious host root,
database administrator, or operator who directly supplies the destination and
secrets; those actors already hold equivalent authority.

### High

Use High for likely account/system compromise or broad private-data loss without
the Critical preconditions, for example:

- A normal reader can read or erase another user's novels, chat, memories,
  world state, generated chapters, or full account export.
- Attacker-controlled input selects a provider origin that receives an operator
  API key or private prompts, or a stored XSS steals access and refresh tokens.
- A bounded uploaded document reliably compromises the parser process or a
  public operation enables sustained service-wide/provider-cost exhaustion with
  modest attacker resources.

### Medium

Use Medium where impact is material but constrained, likelihood is lower, or a
meaningful prerequisite narrows the affected population, for example:

- An authenticated user crosses a spoiler or ownership boundary for a limited
  subset of data but cannot take over accounts or reach secrets.
- A parser or retry path causes repeatable cross-user outage or significant
  spend only with large sustained traffic, an optional provider, or unusual
  input conditions.
- A deployment-default weakness exposes operational metadata or weakens an
  internal boundary and becomes exploitable only when an operator also publishes
  a normally private service port.

High-impact issues with medium/unknown likelihood are normally Medium under this
model unless direct evidence establishes stronger reachability.

### Low

Use Low for limited confidentiality/integrity/availability impact, difficult
preconditions, or useful defense-in-depth gaps, for example:

- Non-secret service/version metadata disclosure, low-volume log injection, or
  a short attacker-only request slowdown with no cross-user effect.
- A provider-output validation gap that can corrupt only the attacker's own
  disposable generated text and cannot alter canon, authorization, or spend.
- A security-header or configuration hardening gap that has no executable sink
  in the current React application and needs another independent vulnerability
  before it matters.

Pure style concerns, generic best-practice omissions, prompt text that merely
breaks character, malicious-operator actions with no privilege gain, and
theoretical dependency claims without a reachable affected operation are not
reportable findings.

Repository: target_sha256_e02444a12e6cbb6554e4f496f3a4efb39fcb8a8216b52125fd26d3a2b1ff8f5e
Version: codex-security-snapshot/v1:sha256:edabedabc8c9370dace521b6f8a749a9144d194177cef663ca4a8b1c9ea4b5e5
