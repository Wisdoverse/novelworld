# Contributing to NovelWorld

NovelWorld accepts focused changes that preserve its product, architecture,
security, and evidence contracts. Read the [documentation index](./docs/README.md)
and [agent instructions](./AGENTS.md) before changing behavior.

## Before you start

- Search existing issues and pull requests before opening new work.
- Roadmap work starts from one approved roadmap issue and must preserve its
  scope, non-goals, invariants, acceptance evidence, dependencies, and rollback.
- Report vulnerabilities through the private process in
  [SECURITY.md](./SECURITY.md), never a public issue.
- Keep one independently mergeable outcome per pull request.

## Development setup

Prerequisites are Rust 1.78 or newer, Node.js 22 or newer, pnpm, Docker, and
Docker Compose.

```bash
git clone https://github.com/<you>/novelworld.git
cd novelworld
docker compose up -d postgres redis
cargo build --workspace
cd frontend
pnpm install --frozen-lockfile
pnpm dev
```

Copy `.env.example` to `.env` when running services locally. Never commit the
resulting secrets or provider credentials.

## Change workflow

1. Create a short-lived branch from current `main`.
2. State the problem and acceptance evidence before implementation.
3. Make the smallest change that satisfies the accepted outcome.
4. Add tests at the lowest layer that proves the behavior; add integration or
   browser coverage when the contract crosses a process or user journey.
5. Update affected contracts, runbooks, and threat boundaries in the same
   commit series.
6. Run the relevant local gates, open a pull request, and respond to review with
   new evidence rather than unsupported claims.

Use an [architecture decision record](./docs/adr/0000-template.md) when changing
a service boundary, data ownership, trust boundary, public contract,
consistency model, availability target, or irreversible dependency. Routine
implementation choices stay in the pull request.

## Engineering constraints

Backend changes must preserve DDD and service ownership:

- Domain layers do not import infrastructure or interface types.
- External systems are reached through domain ports and infrastructure
  adapters.
- Services communicate over HTTP and never read another service's tables.
- SQL uses parameterized bindings; LLM calls use the shared bounded retry
  contract.

Frontend changes must preserve Feature-Sliced Design:

```text
app -> pages -> widgets -> features -> entities -> shared
```

Imports only point downward. Server state uses TanStack Query, client state uses
Zustand, and HTTP/SSE calls go through `frontend/src/shared/api/client.ts`.

## Verification

Run the narrowest useful check while iterating, then all affected gates before
review.

### Backend

```bash
cargo fmt --all -- --check
cargo check --workspace --exclude integration-tests
cargo test --workspace --exclude integration-tests
cargo clippy --workspace --exclude integration-tests --all-targets -- -D warnings
```

### Frontend

```bash
cd frontend
pnpm install --frozen-lockfile
pnpm type-check
pnpm lint
pnpm lint:fsd
pnpm test
pnpm build
```

Run `pnpm exec playwright test` for user-flow, responsive, or accessibility
changes. Run the PostgreSQL/Redis integration suite when changing persistence,
migrations, caching, or cross-service data contracts:

```bash
docker compose -f docker-compose.test.yml up -d --wait test-postgres test-redis
docker compose -f docker-compose.test.yml run --rm test-migrate
cargo test -p integration-tests
docker compose -f docker-compose.test.yml down -v
```

The authoritative required gate is [CI](./.github/workflows/ci.yml). To dispatch
that exact workflow for a clean, pushed commit and wait for the result, run:

```bash
make verify
```

This command requires authenticated GitHub CLI access and intentionally creates
one workflow run. It fails closed for dirty, detached, untracked, or unpushed
checkouts.

## Review standard

A pull request is ready for review when it explains:

- the user or operational problem and what is deliberately out of scope;
- externally visible behavior and compatibility impact;
- security, privacy, data, reliability, accessibility, and cost risks;
- automated and manual evidence, including important negative cases;
- rollout, abort signals, rollback or forward recovery;
- monitoring or logs used to detect failure;
- documentation changed, or why no document is affected.

Use `N/A` with a reason instead of omitting a category. Screenshots are required
for visible UI changes. Database migrations must be forward compatible with the
release and rollback procedure in [DEPLOY.md](./DEPLOY.md).

Reviewers block changes that violate architecture boundaries, weaken a contract
without an approved replacement, claim unsupported behavior, or lack evidence
proportional to risk. `Done` means the final commit is merged to `main` and the
required CI is green.

## Documentation

Follow the [documentation standard](./docs/README.md#documentation-standard).
Use repository-relative links and concrete commands. Code, migrations, and
tests prove behavior; prose must not promote a target or roadmap item to current
support without its required evidence.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```text
feat: add source-bounded lore retrieval
fix: commit chat turns before emitting done
docs: clarify restore evidence
refactor: consolidate provider retry policy
test: cover import lease recovery
chore: update approved dependencies
```

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](./LICENSE).
