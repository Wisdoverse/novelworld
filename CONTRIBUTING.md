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

Prerequisites are current stable Rust (the locked dependency graph currently
requires Rust 1.94.1 or newer), Node.js 22 or newer, pnpm, Docker, and Docker
Compose.

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

Backend changes must preserve the private `single-node-v1` Cloud Native, DDD,
and microservice contract:

- Domain layers do not import application, infrastructure, interface, or
  concrete adapter types. Application code depends on ports, not adapters.
- External systems are reached through domain ports and infrastructure
  adapters.
- Services communicate over HTTP adapters and runtime packages do not depend
  on one another. Runtime SQL is checked against the versioned relation-owner
  manifest.
- Authoritative facts are externalized; process-local state is disposable
  cache/projection/admission only. Each runtime keeps external configuration,
  separate liveness/readiness, JSON tracing, metrics, and graceful signals.
- New or changed dependency calls define their deadline and retry behavior;
  existing unqualified timeout/drain paths are gaps, not reusable defaults.
- SQL uses parameterized bindings; LLM calls use the shared bounded retry
  contract.

`cargo run --locked -p architecture-check -- check` scans the reachable module
graph of all five runtime packages. Production modules and tests nested inside
a DDD layer are checked for layer and SQL violations. Root `cfg(test)`
composition modules are still SQL-scanned but have no DDD layer assignment;
runtime-hook evidence must come from reachable non-test code. The gate blocks
layer inversions, unreviewed domain/application crates and local helpers,
concrete adapter leakage, reviewed non-HTTP/raw transport patterns,
unanalyzable SQL, owner violations, relation/routine inventory drift, cross-owner
view/routine/trigger dependencies, undeclared cross-owner foreign keys, and
missing static runtime hooks. Existing shared-schema debt is exact and visible,
not a general allowlist; adding declared debt is a versioned policy change that
must be justified in review.

Ten existing migrations contain executable `DO` bodies that the conservative
parser intentionally does not interpret. Each is an exact normalized full-file
hash debt; any edit reopens the blocker. New or changed migrations must pass the
strict statement, ownership, view, routine, trigger, and foreign-key audit.

Frontend changes must preserve Feature-Sliced Design:

```text
app -> pages -> widgets -> features -> entities -> shared
```

`pages`, `widgets`, `features`, and `entities` are sliced layers. Imports only
point downward, and an import into one of those slices must address its root
`index.ts`/`index.tsx` public API, such as `@/entities/character`; importing a
private `ui`, `model`, `api`, or other path below another slice is forbidden.
Slices in the same layer cannot import one another. A slice may use relative
paths only for its own internals.

Root entry modules may only bootstrap `app`. The non-sliced `app` and `shared`
layers retain the relative imports needed for their own composition and shared
internals. Those exceptions do not permit an upward import or bypassing a
sliced-layer public API. The architecture check scans every TypeScript/TSX
module under `frontend/src`, including tests and source-side mocks, and treats static
and type-only imports, import types, literal dynamic imports, re-exports,
`require`/import-equals, literal Vitest/Jest module APIs (`mock`, `doMock`,
unmocking, and actual/mock loaders), aliases, and relative paths as dependency
edges. There is no legacy allowlist: any violation fails `pnpm lint:fsd` and
blocks merge.

Server state uses TanStack Query, client state uses Zustand, and HTTP/SSE calls
go through `frontend/src/shared/api/client.ts`.

## Verification

Run the narrowest useful check while iterating, then all affected gates before
review.

### Backend

```bash
cargo fmt --all -- --check
cargo run --locked -p architecture-check -- self-test
cargo run --locked -p architecture-check -- check
cargo check --workspace --exclude integration-tests
cargo test --workspace --exclude integration-tests
cargo clippy --workspace --exclude integration-tests --all-targets -- -D warnings
```

The backend architecture gate is static source evidence. It does not prove
database grants, complete timeout/drain behavior, recovery drills, alerting,
capacity, multi-replica safety, horizontal scaling, or public deployment.
Those claims require their own runtime or migration evidence.

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

`pnpm lint:fsd` proves only the statically detectable import boundary contract.
It does not prove that a feature has the correct semantic owner, that runtime
loading succeeds, or that user-visible behavior is correct; type, unit, build,
and applicable browser gates remain required.

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
