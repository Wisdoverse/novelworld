# NovelWorld Agent Instructions

NovelWorld turns novels into interactive worlds. The repository contains five
Rust/Axum runtime services, a React/TypeScript frontend, PostgreSQL, an optional
Redis projection, and optional S3-compatible storage.

`CLAUDE.md` is a symlink to this file. Keep `AGENTS.md` as the canonical local
agent entrypoint and keep that symlink intact.

## Read what the task needs

Use this file as a map. Do not load the full documentation set for every task.

| When changing | Read |
|---|---|
| Supported behavior, claims, or evidence | [`docs/PRODUCT_CONTRACT.md`](docs/PRODUCT_CONTRACT.md), [`SPEC.md`](SPEC.md), and [`docs/SPEC_CONFORMANCE.md`](docs/SPEC_CONFORMANCE.md) |
| Advanced rules, prompt versions, or D20 migration | [`docs/ADVANCED_RULES_PLAN.md`](docs/ADVANCED_RULES_PLAN.md), [`docs/adr/0010-versioned-basic-game-rules.md`](docs/adr/0010-versioned-basic-game-rules.md), and [`DEPLOY.md`](DEPLOY.md) |
| Service, data, or dependency boundaries | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| Tests, CI, review, or contribution workflow | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| Deployment, configuration, upgrade, or rollback | [`DEPLOY.md`](DEPLOY.md), [`.env.example`](.env.example), and [`docs/OPERATIONS.md`](docs/OPERATIONS.md) |
| Security, privacy, or trust boundaries | [`SECURITY.md`](SECURITY.md), [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md), and the relevant lifecycle contract |
| Product direction or roadmap status | [`docs/ROADMAP.md`](docs/ROADMAP.md) and the GitHub Project |
| Provider qualification or a paid Diagnostic | [`docs/QUALIFICATION_POLICY.md`](docs/QUALIFICATION_POLICY.md) and [`docs/adr/0004-durable-diagnostic-budget.md`](docs/adr/0004-durable-diagnostic-budget.md) |
| LLM provider, region, Coding/Token Plan, credential switching, or price estimation | [`docs/LLM_PROVIDERS.md`](docs/LLM_PROVIDERS.md) and [`docs/LLM_PRICING.md`](docs/LLM_PRICING.md) |

[`docs/README.md`](docs/README.md) is the complete documentation index. Runtime
code, migrations, and tests own current behavior; prose describes contracts and
evidence limits. Resolve conflicts conservatively and update the owning source.

## Non-negotiable boundaries

Backend work must preserve the private `single-node-v1` Cloud Native, DDD, and
microservice contract:

- Domain code depends only on domain code and pure data types. Application code
  depends on domain ports. Concrete database, cache, storage, model, password,
  and HTTP adapters belong in infrastructure.
- Runtime services communicate over HTTP and never depend on another runtime
  package. Each business relation has one owner. Runtime SQL may access only
  owner relations or an exact versioned exception.
- PostgreSQL or configured object storage owns authoritative facts. Local state
  is limited to disposable cache, projection, readiness, and bounded admission.
- Runtime configuration and secrets are externalized. Each service keeps
  separate liveness/readiness, JSON tracing, metrics, and graceful signals.
- New or changed dependency calls define deadlines and retry behavior. Retried
  side effects require idempotency, a durable claim/lease, or an explicit
  non-retry contract.

Frontend work must preserve Feature-Sliced Design:

```text
app -> pages -> widgets -> features -> entities -> shared
```

Imports point downward. Cross-layer consumers use a slice's root public API;
they do not enter another slice's private segments. Same-layer slices do not
depend on one another. Root entries only bootstrap `app`. The rule covers all
TypeScript/TSX source, tests, mocks, re-exports, and literal dynamic/module-test
imports. There is no legacy allowlist.

`cargo run --locked -p architecture-check -- check` and `pnpm lint:fsd` are the
blocking structural gates. Their success proves only the invariants they scan.
Do not turn static evidence into runtime, recovery, deployment, scale, or
product evidence.

H3/H4 journey generation uses DeepSeek. Do not select or call OpenAI generation
for that work. Embeddings may use the supported local or external
OpenAI-compatible endpoint; preserve both configurations.

Never commit or publish `.env`, credentials, API keys, raw private provider
responses, or private evidence paths.

## How to work

- Inspect the relevant implementation, callers, contract, and current issue or
  pull request before editing. Prefer the smallest root-cause change and reuse
  existing code and checks.
- For complex work, state deliverables, evidence entrypoints, allowed actions,
  and stopping conditions. For a routine scoped change, proceed directly.
- Within the requested scope, reads, reversible edits, branches/worktrees,
  disposable local tests, formatting, affected checks, commits, pull requests,
  review fixes, and proven-safe post-merge cleanup do not need step-by-step
  approval.
- A paid provider run requires explicit authorization for its exact reviewed
  registration. Public deployment, credential disclosure, account funding, and
  destructive or unrecoverable operations also require explicit authority.
- A frozen or consumed Diagnostic is terminal. Never rerun it, rewrite its
  evidence, refill its budget, or change thresholds to hide a result.
- Preserve unrelated working-tree changes. Before cleanup, prove merge or patch
  equivalence, branch/worktree ownership, and absence of open dependencies.

Complete the requested outcome, not just the first patch. Run the affected
checks from [`CONTRIBUTING.md`](CONTRIBUTING.md), inspect the actual result, fix
failures caused by the change, and repeat the affected checks. Keep independent
review and required CI blocking. Stop exploration when the stated acceptance
evidence is satisfied or a concrete external blocker prevents further progress.

Keep evidence claims separate: source, local tests, CI, built artifacts, live
runtime, deployment, provider execution, qualification, merge, and Project
closure are different facts. State skipped or unavailable evidence plainly.

## Roadmap and GitHub records

- [`docs/ROADMAP.md`](docs/ROADMAP.md) owns direction, invariants, horizon order,
  and exit criteria. GitHub Project fields own live execution status, horizon,
  and priority.
- One roadmap issue represents one independently mergeable outcome. Add active
  roadmap issues and pull requests to the Project and keep their fields current.
- A roadmap pull request links its issue with `Closes #<issue>`. Mark an issue
  `Done` and close it only after its acceptance evidence exists, its final
  commit is merged to `main`, and required CI is green. Structural children may
  close while a live-evidence parent stays open.
- Use the issue body and Project fields as the current execution record. Replace
  superseded status in place; do not add comments for plans, progress, pending
  CI, or facts already visible in linked records.
- Default to zero status comments. Add one concise immutable comment only for a
  consumed external/provider execution or a human decision that cannot be
  represented by fields or the issue body. Preserve unique audit and no-rerun
  evidence before deleting an obsolete comment, and migrate inbound links first.
