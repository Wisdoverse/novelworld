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

Write identifiers, comments, configuration, contributor documentation, and
default interface copy in English. Keep the English and Simplified Chinese UI
catalogs complete, including accessible labels and local errors. Preserve
Chinese documentation, literary and parser fixtures, and immutable versioned
provider registrations. Interface language does not change source text or the
language of generated content.

## Development setup

Use current stable Rust, Node.js 26, pnpm, Docker, and Docker Compose.
The locked Rust dependencies require version 1.94.1 or newer.

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

### Work authorization, stopping, and cleanup

Inspect the implementation, callers, contract, and current issue or pull request before editing.
Prefer the smallest root-cause change. Reuse existing code and checks.
Proceed directly with routine scoped work.
Within scope, routine work needs no step-by-step approval.
Allowed work includes the following:

- reading files and making reversible edits
- creating branches and worktrees
- running disposable local tests
- formatting files and running affected checks
- commits, pull requests, and review fixes
- proven-safe post-merge branch and worktree cleanup

For complex work, state the deliverable, evidence entrypoints, allowed actions, and stopping condition.
A paid provider run requires explicit authorization for its exact reviewed registration.
Public deployment, credential disclosure, account funding, and destructive or unrecoverable actions require explicit authority.
A frozen or consumed Diagnostic is terminal.
Do not rerun it.
Do not rewrite its evidence.
Do not refill its budget.
Do not change its thresholds to hide a result.
Preserve unrelated working-tree changes.
Before cleanup, prove merge or patch equivalence, branch and worktree ownership, and the absence of open dependencies.
Complete the requested outcome.
Run affected checks.
Inspect their results.
Fix failures caused by the change.
Repeat affected checks.
Keep independent final review and required CI blocking.
Stop when acceptance evidence is met or a concrete external blocker prevents progress.
Report evidence classes separately. State skipped or unavailable evidence.

### Roadmap and GitHub records

The Roadmap owns direction, invariants, horizon order, and exit criteria.
The GitHub Project owns live status, horizon, and priority.
Each roadmap issue represents one independently mergeable outcome.
Add active roadmap issues and pull requests to the Project. Keep their fields current.
Link each roadmap pull request with `Closes #<issue>`.
After acceptance evidence exists, the final commit reaches `main`, and required CI passes, set the Project item to `Done`.
Then close the issue.
A structural child may close while its live-evidence parent stays open.
Use the issue body and Project fields as the current execution record. Replace superseded status in place.
Default to zero status comments. Do not comment on plans, progress, pending CI, or facts already in linked records.
An immutable comment is only for a consumed external/provider execution or a human decision that the issue body and fields cannot represent.
Add one concise immutable comment for that event or decision.
Before deleting an obsolete comment, preserve unique audit and no-rerun evidence.
Before deletion, migrate inbound links.

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

Nine existing migrations contain executable `DO` bodies that the conservative
parser intentionally does not interpret. Each is an exact normalized full-file
hash debt; any edit reopens the blocker. New or changed migrations must pass the
strict statement, ownership, view, routine, trigger, and foreign-key audit.
Migration 0002 is strictly scanned and is not part of that hash-debt count.
The unchanged 0031 and 0033 migrations have two additional exact-file debts
because later migrations replace their historical series trigger functions.
The new 0034 definition remains strictly audited against `init.sql`.

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

Native Rust release images require source-to-binary freshness inside the
existing locked BuildKit cache. Before release compilation, clean only workspace
release artifacts with `cargo clean --locked --release --workspace`; retain
third-party compiled dependencies and registry/layer caches, and never prune a
shared host Cargo target. Historical git-archive source mtimes can otherwise
make Cargo reuse workspace binaries from another source revision, so a completed
Docker `RUN` is not proof that changed source was compiled. Require build evidence
that Cargo compiled the changed source. For a registered release pair, a genuine
B runtime change must yield at least one affected application image with both a
different image ID and at least one different filesystem layer. Docs/cache-only
changes and unchanged runtime inputs need not change every image or binary.
The historical C→B Diagnostic path used merged A (#428),
cache-freshness C (#432), and remote-profile B (#429); eligibility required an
actual merged C baseline and strict-descendant B candidate with the guard on
both sides.
For the exact prospective V6 source pair and its artifact, prestart, scan,
budget, and authorization gates, follow the owning
[Qualification Policy](docs/QUALIFICATION_POLICY.md). Source eligibility is
prospective only; it does not transfer historical evidence, renew a frozen or
consumed registration, or claim live or formal qualification.

A dependency-only frontend pair also requires different full maps of compiled
and served assets, plus a changed corresponding frontend filesystem layer. The
historical C→B artifacts and Frozen records retain their original identities
and results.

For Vision journey tooling changes, also run
`python3 tests/e2e/live_deepseek_journey.py --self-test`. CI runs
`diagnostic_journey_test.py` with the digest-pinned `test-postgres` service and
destroys its volume afterward. The PostgreSQL selector test mutates public rows
and therefore requires `NW_H4_TEST_POSTGRES_DISPOSABLE=1`; never point it at a
shared database. The suite checks single-start registration, committed source
identity, synthetic receipt reconciliation, cancellation, bounded terminal
handling, public-report privacy, and latest-Canon player-entry selection without
a provider call. These checks do not replace the mandatory real release-image
cold-adoption/Settings and lifecycle wiring evidence tracked in #322. No
protected operator key is needed or authorized by these gates.

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
TEST_DATABASE_URL=postgres://test:test@localhost:25432/novelworld_test \
  cargo test --locked -p novel-service --test community_series_http -- --ignored
TEST_DATABASE_URL=postgres://test:test@localhost:25432/novelworld_test \
  cargo test --locked -p novel-service --test world_source_http -- --ignored
TEST_DATABASE_URL=postgres://test:test@localhost:25432/novelworld_test \
  cargo test --locked -p novel-service --test import_metrics -- --ignored
docker compose -f docker-compose.test.yml down -v
```

The production-shaped development base uses PostgreSQL-backed cache mode and
does not start Redis. To exercise the optional production Redis projection,
set `CACHE_MODE=redis`, a strong URL-safe `REDIS_PASSWORD`, and the matching
`REDIS_URL`, then add `--profile redis`; the root launchers derive these
together and are the supported path. The independent integration Compose file
above always starts its isolated unauthenticated test Redis.

### Disposable PostgreSQL recovery drill

The backup/restore drill uses a local E2E stub for model, embedding, and image
requests. It needs no external provider credentials. The drill runs destructive
`down -v` steps, so use a disposable checkout with no private `.env`, synthetic
secrets, and a Compose project that owns no retained data. Select unused
loopback ports before starting.

```bash
set -euo pipefail
suffix="$(date -u +%Y%m%d%H%M%S)-$$"
export COMPOSE_FILE=docker-compose.yml:docker-compose.e2e.yml
export COMPOSE_PROJECT_NAME="nw-recovery-$suffix"
export CONTAINER_PREFIX="nw-recovery-$suffix"
export NGINX_HTTP_BIND=127.0.0.1 NGINX_HTTP_PORT=28080
export E2E_GATEWAY_PORT=28081 E2E_LLM_STUB_PORT=28082
export E2E_API_URL="http://127.0.0.1:${NGINX_HTTP_PORT}/api"
export E2E_LLM_STUB_URL="http://127.0.0.1:${E2E_LLM_STUB_PORT}"
export POSTGRES_IMAGE='pgvector/pgvector:pg18@sha256:2ba9ca5f2e7daa0f0e7723cba1ee9167bab54efd3640516a44ac1a928dd67e7a'
export POSTGRES_PASSWORD="$(openssl rand -hex 32)"
export JWT_SECRET="$(openssl rand -hex 32)"
export RUNTIME_CONFIG_KEY="$(openssl rand -hex 32)"
export INTERNAL_SERVICE_TOKEN="$(openssl rand -hex 32)"
export LLM_API_KEY="$(openssl rand -hex 16)"
# Optional cross-image restore: use an immutable digest already cached locally.
# export E2E_RESTORE_POSTGRES_IMAGE='registry/image@sha256:<digest>'
docker image inspect "$POSTGRES_IMAGE" >/dev/null
[ -z "${E2E_RESTORE_POSTGRES_IMAGE:-}" ] || docker image inspect "$E2E_RESTORE_POSTGRES_IMAGE" >/dev/null
docker compose up -d --wait
tests/e2e/backup_restore_drill.sh
docker compose down -v
```

Run the backup/restore drill only in this owned disposable project. Stop and
preserve the stack if any command fails. The command
does not prove image compatibility, deployment adoption, or the RTO target;
those claims require the corresponding completed native evidence.

The required Compose core journey admits source chapter 2 into an existing
source-1 world without provider work, commits a subsequent event outcome,
checks exact source replay after service restart, and stops Agent during a world turn and checks
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

Use the [documentation index](./docs/README.md) to find the owner. Update the
existing contract or procedure that owns a behavior instead of copying it into a
new guide. State each material document’s audience, owned decision, profile,
evidence, and failure boundary; update behavioral documentation in the same PR.
Keep normative targets, current support, and evidence distinct; link to code,
tests, migrations, and versioned records rather than repeating them. Use
repository-relative links and concrete commands. Do not promote a target or
roadmap item to current support without its required evidence.

Use an [ADR](./docs/adr/0000-template.md) only for durable service-boundary,
ownership, trust, public-contract, consistency, availability, or dependency
decisions. Land an accepted ADR with its implementation; routine choices stay
in the PR.

### English technical writing

Use [ASD-STE100 Issue 9](https://www.asd-ste100.org/about_STE.html) as a writing
reference. Read its [FAQ](https://www.asd-ste100.org/STE_faq.html). Use the
[downloads page](https://www.asd-ste100.org/STE_downloads.html) to request the full
standard and dictionary.

Apply these project rules to English technical documents, comments, default interface
instructions, and GitHub records. Preserve Chinese documentation, translations,
literary and parser examples, and story-generation language.

- Use at most 20 words per instruction sentence and 25 words per descriptive sentence.
- Use short sentences and active voice. Give each sentence one purpose.
- Write procedures as commands. State required conditions before actions. Give
  each step one action.
- Use fixed technical terms. Define each term when you first use it. Use an
  approved word only with its approved meaning and part of speech.
- Use American English. Include the articles and words that complete each sentence.
- State what failed and what the reader should do next in each error message.
- Keep API names, protocol values, and commands exact. Do not paraphrase them.
- State what evidence proves and what it does not prove. Do not call skipped
  evidence a pass.

Before you claim full ASD-STE100 compliance, do a review against the full standard
and dictionary. Review existing text before you claim that it obeys ASD-STE100.
Keep all repository safety, architecture, approval, budget, test, and release rules.

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
