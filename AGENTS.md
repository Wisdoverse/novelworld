# NovelWorld Agent Instructions

NovelWorld turns novels into interactive worlds. It uses Rust, React, PostgreSQL, and optional Redis and S3-compatible storage.
Keep `CLAUDE.md` as a symlink to this file. Keep `AGENTS.md` as the canonical agent entrypoint.

## Read the owner

Use [`docs/README.md`](docs/README.md) to find the current document owner.

| Task | Read |
|---|---|
| Supported behavior, claims, or evidence | [Product contract](docs/PRODUCT_CONTRACT.md), [SPEC](SPEC.md), and [conformance ledger](docs/SPEC_CONFORMANCE.md) |
| Advanced rules, prompt versions, or D20 migration | [Product contract](docs/PRODUCT_CONTRACT.md), [ADR 0010](docs/adr/0010-versioned-basic-game-rules.md), and [DEPLOY](DEPLOY.md) |
| Service, data, or dependency boundaries | [Architecture](docs/ARCHITECTURE.md) and [CONTRIBUTING](CONTRIBUTING.md) |
| Tests, CI, review, or contribution workflow | [CONTRIBUTING](CONTRIBUTING.md) |
| Deployment, configuration, upgrade, or rollback | [DEPLOY](DEPLOY.md), [.env.example](.env.example), and [Operations](docs/OPERATIONS.md) |
| Security, privacy, or trust boundaries | [SECURITY](SECURITY.md), [threat model](docs/THREAT_MODEL.md), and the lifecycle contract |
| Product direction or roadmap status | [Roadmap](docs/ROADMAP.md) and the GitHub Project |
| Provider qualification or a paid Diagnostic | [Qualification policy](docs/QUALIFICATION_POLICY.md) and [ADR 0004](docs/adr/0004-durable-diagnostic-budget.md) |
| LLM provider, region, Coding/Token Plan, credential switching, or price estimation | [Provider guide](docs/LLM_PROVIDERS.md) and [pricing guide](docs/LLM_PRICING.md) |

[`docs/README.md`](docs/README.md) is the complete documentation index. Runtime code, migrations, and tests own current behavior. Contracts define supported behavior and evidence limits.
Update the document that owns each changed behavior. Resolve conflicts conservatively.
Use [English technical-writing rules](CONTRIBUTING.md#english-technical-writing).

## Keep core boundaries

Backend changes preserve the private `single-node-v1` Cloud Native, DDD, and microservice contract.
Domain code depends only on domain code and pure data types. Application code depends on domain ports.
Database, cache, storage, model, password, and HTTP adapters belong in infrastructure.
Runtime services communicate over HTTP and do not depend on other runtime packages. Each business relation has one owner.
Runtime SQL may access owner relations or an exact versioned exception.
PostgreSQL or configured object storage owns authoritative facts. Use local state only for disposable cache, projection, readiness, and bounded admission.
Runtime configuration and secrets stay external. Each service keeps separate liveness and readiness, JSON tracing, metrics, and graceful signals.
New or changed dependency calls define deadlines and retry behavior. Retried side effects require idempotency, a durable claim or lease, or an explicit no-retry contract.

Frontend changes preserve Feature-Sliced Design:
```text
app -> pages -> widgets -> features -> entities -> shared
```
Imports point downward. Consumers use slice root public APIs. Same-layer slices do not depend on each other.
Root entries only bootstrap `app`. There is no legacy allowlist.
Apply these rules to TypeScript/TSX source, tests, mocks, re-exports, and literal dynamic or module-test imports.
`cargo run --locked -p architecture-check -- check` and `pnpm lint:fsd` are blocking structural gates.
A static pass proves only the rules it scans. It does not prove runtime, recovery, deployment, scale, or product behavior.

H3/H4 journey generation uses DeepSeek. Do not use OpenAI generation for those journeys.
Embeddings may use the supported local model or external OpenAI-compatible endpoint. Preserve both.
Write contributor-facing source, configuration, comments, automation, and default UI copy in English.
Keep English and Simplified Chinese UI catalogs complete. Preserve Chinese documentation, translations, literary and parser examples, and story language.
Never commit or publish `.env`, credentials, API keys, raw private provider responses, or private evidence paths.
Paid provider runs require explicit authorization for the exact reviewed registration.
Never rerun a frozen or consumed Diagnostic. Preserve its evidence, budget, and thresholds.
Public deployment, credential disclosure, account funding, and destructive actions require explicit authority.

## Work and records

Follow [work authorization and cleanup rules](CONTRIBUTING.md#work-authorization-stopping-and-cleanup) and [roadmap record rules](CONTRIBUTING.md#roadmap-and-github-records).
Inspect actual results. Keep independent final-head review and required CI blocking.
Report source, local tests, CI, artifacts, runtime, deployment, provider execution, qualification, merge, and Project status as separate facts.
State skipped or unavailable evidence.
