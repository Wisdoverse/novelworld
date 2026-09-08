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
docker compose up -d postgres
cargo build --workspace
cd frontend
pnpm install --frozen-lockfile
pnpm dev
```

Copy `.env.example` to `.env` when running individual services locally and
preseed valid PostgreSQL values plus `BOOTSTRAP_L0_COMPLETE=true`. The root
server launchers perform that L0 guide for interactive installs. Never commit
the resulting secrets or provider credentials.

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

During the current private rapid-iteration phase,
[human acceptance is not required](./docs/QUALIFICATION_POLICY.md#rapid-iteration-acceptance).
Non-author agent review and all affected automated/required CI gates remain
mandatory; no additional human sign-off is needed to deliver a focused change.
This applies throughout the active roadmap goal, including agent-adjudicated
new revisions and bounded Diagnostics under the linked policy's pre-call
identity, budget and evidence controls.
Keep unperformed human checks unverified, and do not label iteration delivery
as formal release qualification or authorization to rerun a frozen model.

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

Push and pull-request CI select jobs from the complete change set:

| Changed area | Selected checks beyond the always-on policy checks |
|---|---|
| Root Markdown, documentation prose/images, issue/PR templates | None |
| Backend services, shared Rust crates, integration tests | Backend, integration, production smoke |
| Frontend | Frontend build/browser, desktop contract, production smoke |
| Tauri shell | Desktop contract and backend (includes Tauri dependency audit) |
| PostgreSQL schema/migrations | Backend, integration, desktop contract, production smoke |
| Launchers | Unix and Windows launcher contracts |
| Shared build/configuration, workflows, or unknown paths | Full suite |

`tools/ci_scope.py` owns the conservative routing rules. Mixed changes take the
union; unavailable Git history falls back to the full suite. Scope self-tests,
specification checksums, and secret scanning always run. The required
`Rust Build & Test` check aggregates policy and selected-job results, so a failed
scope detector or failed selected check blocks merging even when unrelated jobs
are skipped. Manual verification (`make verify`) and reusable release CI run
the full suite. Docker image publication and portable builds retain their
existing release/manual triggers.

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

Changes to diagnostic LLM dispatch, its owner ledger, or lifecycle fixture also
run the offline budget boundary gate on Linux with Docker and OpenSSL:

```bash
cargo build --locked -p user-service -p novel-service -p agent-service -p narrative-service
cargo build --locked -p llm-client --example diagnostic_budget_driver
python3 tests/e2e/diagnostic_budget_lifecycle_test.py
python3 tests/e2e/diagnostic_release_docker_spy_test.py
python3 tests/e2e/diagnostic_journey_test.py
python3 tools/llm-budget/test_verify.py
python3 - <<'PY'
import runpy
import subprocess
fixture = runpy.run_path("tests/e2e/diagnostic_budget_lifecycle.py")
for name in ("PYTHON_IMAGE", "PG_IMAGE", "REGISTRY_IMAGE"):
    subprocess.run(["docker", "pull", fixture[name]], check=True, timeout=180)
PY
python3 tests/e2e/diagnostic_budget_lifecycle.py \
  --owner-binary target/debug/user-service \
  --client-binary target/debug/examples/diagnostic_budget_driver
```

Images are fetched before isolation. The fixture packages the four normal
service binaries (from the same directory as `--owner-binary`) without source
or credentials, uses a temporary loopback-only registry for real repository
digests, and invokes the existing release capability preflight unchanged.
The budget/provider test uses synthetic credentials, real PostgreSQL and
production client constructors on a unique Docker internal network; it never
loads an operator key or forwards to a provider. It removes only its own
containers, anonymous volumes, image references and network, including on
ordinary cancellation (not SIGKILL); shared base images/build cache remain.
On hosts with exhausted Docker address pools,
`--subnet` accepts an unused RFC1918 `/28` after Docker/host-route overlap checks.
The same fixture executes real `release.sh`, Compose config/pull and digest
probes against an explicitly synthetic Git/release-state fixture. Invalid
candidate, current and previous artifacts must fail before any deployment
command is attempted, preserving the ledger, running owner/PostgreSQL and
provisioning marker. A command spy forwards permitted operations to real Docker
and refuses unexpected deployment commands; it never substitutes fake success.
Failed preflight leaves Git at the target metadata commit. A subsequent valid
`preflight` succeeds without restoring that checkout; this is safe reentry,
not automatic checkout recovery.
This proves release/capability refusal, dispatch/accounting and owner recreation
boundaries, not release-built artifacts, a supported base-to-candidate release
upgrade or live-model qualification.

For Vision journey tooling changes, also run
`python3 tests/e2e/live_deepseek_journey.py --self-test`. The separate
`diagnostic_journey_test.py` checks single-start registration, committed source
identity, synthetic receipt reconciliation, cancellation, bounded terminal
handling and public-report privacy without Docker or provider calls. These
checks do not replace the mandatory real release-image cold-adoption/Settings
and lifecycle wiring evidence tracked in #322. No protected operator key is
needed or authorized by these offline gates.

The same lifecycle fixture has a separate `--journey-images` mode for actual
cold-adoption wiring. Its JSON map contains the four paying services plus
`gateway` and `frontend`, all built with the repository's release Dockerfiles.
This Linux-only mode requires `socat` and an explicitly selected unused RFC1918
`/28` subnet for static loopback ingress. Docker requires an explicitly configured
network subnet for a static container IP; an automatically allocated default
pool is insufficient. The fixture rejects overlap with Docker networks or host
routes. The subnet below is an example; select a free subnet on your host.
Use a pre-created mode-0700 output directory outside the checkout:

```bash
python3 tests/e2e/diagnostic_budget_lifecycle.py \
  --journey-images /private/path/six-release-images.json \
  --journey-output /private/path/empty-cold-adopt-evidence \
  --subnet 10.254.241.0/28
```

This mode reuses the internal-network TLS fixture and supported `release.sh`
cold adoption. Only a private synthetic checkout changes Compose network/CA/
proxy wiring. It assigns nginx a checked unused internal IPv4 address and removes
its Docker published port; a host `socat` listener starts on the random loopback
port before adoption, forwarding only to that nginx address on port 80. The
fixture verifies the actual nginx address after adoption and stops the exact
forwarder's process group on success or failure. This avoids relying on Docker
internal-network port publishing without giving product containers public egress.
Runtime images and release implementation are not replaced.
Private output retains forwarding-process recovery metadata and the pre-adoption
Docker inventory. A failed stop gets one bounded cleanup retry; unproven removal
still fails the fixture. Production Compose Smoke runs this gate after the
existing release-image dispatch/refusal matrix, without publishing private output.
Cold-adoption stdout reports only the zero/nonzero case and fixed boolean stage
presence/terminal flags after the journey's terminal handling, including failure;
private reports and identifiers
remain local. These flags do not replace the fixture's blocking assertions.
A second fixed boolean summary recognizes release phase markers and selected
failure classes from at most 1 MiB of the private adoption log. Missing or larger
logs do not produce phase evidence; raw lines and unknown values are never emitted.
Failed adoption also observes the seven fixed application containers before
terminal cleanup: exact name/project ownership precedes ID-addressed log reads.
One 15-second deadline bounds all reads (at most two seconds and 64 KiB per call,
80 log lines per container). Only fixed state fields, health enums and startup
error-word booleans reach stdout; missing containers and unproven observations
are distinct. Error-word matches are hints, not root-cause proof. Observation
failure cannot replace the adoption failure or skip terminal cleanup; cancellation
does not start this observation. No raw container logs are published or retained.
Outer fallback cleanup shares a 60-second deadline, with Docker calls capped at
10 seconds each. CI sends a soft TERM after 15 minutes, reserving 10 minutes for
terminal handling before a hard kill; SIGKILL/host loss still has no cleanup
guarantee and never makes a failed registration resumable.
Exact owned-container removal also removes its anonymous volumes (including
the PostgreSQL image's parent mount). Named PostgreSQL volumes remain governed
by the independent durable-evidence check; no volume pruning is used.
Separate zero/nonzero registrations exercise Settings, the runner's generated
environment, consistent owner/PG checkpoints and restart persistence. Since
these are partial fixtures, the full journey's missing-quality-sample gate
must fail; terminal handling preserves the owned PG volume. After checking
the durable receipts and exact retained-volume ownership, the fixture performs
non-paid cleanup of that volume. Private evidence remains in the output
directory. A single-image cold adoption is not a genuine version upgrade or
a live-model/whole-journey PASS. Run the existing `--runtime-images` matrix
separately as well; cold adoption does not replace its dispatch/ACK-loss cases.

Production Compose Smoke also invokes the same fixture with
`--runtime-images <json-file> --client-binary <path>`: a map from each of the
four service names to its existing local image built by `Dockerfile.rust-service`.
This repeats the full dispatch, owner-recreation and release-refusal matrix
without building replacement service images. Published test-local digests must
retain the original image IDs, and cleanup preserves the source references.
For a narrower capability-only diagnostic, `--capability-images <json-file>`
does not provision a database or start the product. Neither mode proves a
successful different-version upgrade/rollback journey; that remains #230.

### Frontend

```bash
cd frontend
pnpm install --frozen-lockfile
pnpm audit:dependencies
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

The production-shaped development base uses PostgreSQL-backed cache mode and
does not start Redis. To exercise the optional production Redis projection,
set `CACHE_MODE=redis`, a strong URL-safe `REDIS_PASSWORD`, and the matching
`REDIS_URL`, then add `--profile redis`; the root launchers derive these
together and are the supported path. The independent integration Compose file
above always starts its isolated unauthenticated test Redis.

The extended Compose core journey stops Agent during a world turn and checks
durable pending state, the no-overtake barrier, scanner recovery before replay,
and exact replay without another model call. Changes to its script trigger
these drills on PRs. This fixture-based check does not prove live model quality
or release-upgrade recovery. Run it only in the existing isolated CI topology,
not against a personal deployment; the script uses fixed test container names.

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
