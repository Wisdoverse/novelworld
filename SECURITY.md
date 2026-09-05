# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| main branch | ✅ |

## Reporting a Vulnerability

**Do NOT open a public issue for security vulnerabilities.**

Instead, please report them responsibly:

1. Email: [security contact via GitHub private vulnerability reporting]
2. Or use GitHub's [private vulnerability reporting](https://github.com/schorsch888/novelworld/security/advisories/new)

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We will acknowledge receipt within 48 hours and provide a timeline for resolution.

## Security Measures

### Authentication
- Passwords hashed with bcrypt (cost factor 12) in a bounded blocking pool
- JWT tokens with configurable expiry (default 1 hour)
- Refresh tokens are atomically consumed and rotated in server-side storage
- 401 responses do not leak user existence information

### Data Protection
- All SQL queries use parameterized bindings (no string interpolation)
- File upload accepts TXT, EPUB, and PDF with 10 MiB/20 MiB input limits,
  a 20 MiB extracted-text ceiling, bounded blocking parsers, and EPUB aggregate
  expansion and duplicate-spine checks
- Production Nginx enforces 20 requests/second per client with burst 40; the
  Gateway applies its configurable global backstop after authentication on
  protected routes (default 500 requests/second)
- Novel imports, chats, bcrypt, and provider calls have process-wide admission;
  imports also have a fixed provider-call budget
- Retention and application-layer erasure boundaries are documented in
  [docs/DATA_RETENTION.md](./docs/DATA_RETENTION.md)
- Account export uses JWT-derived identity, internal-token-authenticated service
  fragments, explicit field allowlists, a two-request concurrency ceiling, and
  a 15-minute end-to-end deadline. See
  [docs/ACCOUNT_EXPORT.md](./docs/ACCOUNT_EXPORT.md).

### Infrastructure
- All inter-service communication over internal Docker network
- Only Nginx port (80/443) exposed externally in production
- Database credentials auto-generated on first run
- JWT secret auto-generated (256-bit random)


### Secret Rotation

The three deployment secrets have different rotation contracts:

- **JWT_SECRET** — rotating it invalidates every access token (signature
  check). Persisted refresh tokens are opaque server-side rows, not JWTs:
  they survive a JWT_SECRET rotation, so the rotation procedure wipes them
  explicitly (the same two-step contract the restore path applies). All
  sessions end; users log in again.
- **INTERNAL_SERVICE_TOKEN** — authenticates service-to-service calls.
  Rotating it requires recreating every service together in one compose
  operation; a fleet that mixes old and new tokens breaks internal calls.
- **RUNTIME_CONFIG_KEY** — encrypts the provider configuration saved by the
  first-run web setup. It is **not rotatable in place**: rotating it makes the
  stored configuration undecryptable and the deployment fails hard with no
  redo path, so treat it as fixed for a deployment's lifetime and
  re-provision instead.

Procedure (operator, on the host):

1. Update .env with the new JWT_SECRET and INTERNAL_SERVICE_TOKEN.
2. docker compose up -d --force-recreate (all services together).
3. Wipe persisted sessions:
   docker exec novel-postgres psql -U novel -d novel_world -c "DELETE FROM refresh_tokens"
4. Users log in again.

The e2e secret_rotation_drill.sh verifies this contract: old access and
refresh tokens are rejected, a new login works, and an account export (which
crosses internal-token-authenticated service calls) succeeds after rotation.

### Release Rollback

`infra/docker/release.sh` is the release/rollback state machine for the
compromised-release response. Manifests pin every image to an immutable
`@sha256` digest plus a release version and git SHA; the state dir holds
`current.env` and `previous.env` with a flock-guarded lock. Every new
post-client-gate deployment, including normal restore and healthy rollback,
writes the exact target to `schema-transition.pending` before migration.
Restore and rollback first discard an unmarked staged candidate so it cannot
conflict with that authority. New rollback uses the same promotion protocol as
upgrade. `rollback.pending` is accepted only as a compatibility recovery record
from older tooling, and the next locked operation converges it before doing
anything else.
That marker is the only recovery authority until promotion: a transition that
may have crossed application-semantic migration 0021, 0024, or 0025 rolls the
exact target forward and never starts the older writer or reader. Adoption
requires all three barriers, and upgrade, marked restore, and rollback refuse
to cross any one backwards. On the supported managed Docker path, a database
after 0024 must not run a pre-0024 Agent or Novel release, and a database after
0025 must not run a pre-0025 Agent release, except through a separately approved
compatibility procedure. Experimental desktop archives are forward-migration-only;
an older archive must not reuse a post-0025 data directory because that older
binary cannot enforce these barriers. The marker is storage-synced before migration; promotion is
synced before marker removal, and marker removal is synced again. For an
upgrade, the former current manifest is renamed and synced as `previous`
before the new current manifest is installed. In both adoption and upgrade,
the installed target tempfile is included in that pre-rename sync.

What the release_state_drill.sh proves locally (no registry required):

- Manifest grammar fails closed: non-digest images, malformed git SHAs and
  versions, unknown, duplicate, empty, or missing keys are all rejected.
- Every upgrade and rollback guard fires before `deploy_manifest`'s secrets
  check (and therefore before any network access): a divergent manifest for
  the current SHA, infrastructure-image changes, malformed rollback targets,
  a missing or mismatched previous release, and concurrent operations (held
  lock) all stop with an actionable error.
- Redis mode verifies that the running container's immutable image reference
  exactly matches the active release manifest before replacing any application
  service; the state drill rejects a healthy container with a mismatched image.
- A normal restore and a healthy rollback clear an unmarked candidate before
  the schema marker is written, durably promote the exact marked target, and
  clear the marker only after the promoted pair is synced.
- A legacy interrupted rollback recovers its current/previous pair before the
  next command, and a wedged legacy marker with missing files fails closed: the
  marker survives and requires explicit operator clearing.
- An interrupted semantic-barrier adoption/upgrade rolls the exact schema-transition
  manifest forward, accepts a missing downloaded candidate, rejects a
  different candidate, preserves the old current as previous on upgrade, and
  clears the marker only after the health-gated manifest is promoted.

What stays gated: the image-level deployment (`deploy_manifest`: git checkout
of the release SHA plus `compose pull` of the digest-pinned images) runs only
with a reachable registry. CI validates the manifest grammar and lock
guards; the deployment path itself is not exercised by a local drill. The
state drill pins sync/rename order and simulates process crashes, but it does
not inject a real Linux host power loss or prove filesystem directory-entry
durability across one.
SBOM generation has since landed (see Dependency Policy). Release-file
provenance is documented in the Release-file provenance section below;
deploy-time SBOM admission and platform-native signing remain gated.
### Dependency Policy

CI runs `cargo audit` against both `Cargo.lock` files with the live RustSec
advisory database and `pnpm audit --prod --audit-level high` against the
frontend's frozen lockfile. A newly reported Rust vulnerability or
HIGH/CRITICAL advisory in a shipped browser dependency fails the build;
development-only frontend tooling is outside that production-dependency gate.
Dependabot covers Cargo, npm, Dockerfiles, Compose files, and GitHub Actions.
The temporary TypeScript 7/6 npm aliases, pinned Alpine packages in Dockerfile
`RUN` steps, the release workflow's GitHub CLI archive, and immutable scanner
images embedded in shell commands are
verified manually against upstream releases during each dependency-maintenance
change because Dependabot does not parse those forms.
CI also runs `gitleaks` over the full commit history: any committed secret
fails the build.
`.gitleaks.toml` is the full default rule set plus narrow allowlists for
the upstream rule-set examples and two deliberate test fixtures (the CI
`RUNTIME_CONFIG_KEY` smoke placeholder and two static provider model names).
Credential-shaped upstream examples are regex-escaped so the allowlist still
matches historical fixtures without committing complete key-shaped literals.
CI and the self-test `tests/e2e/gitleaks_self_test.sh` scan the full history:
it plants a GitHub-shaped token and asserts the scan fails (a config that
silently lost its rules would pass everything and must not go unnoticed),
then asserts the repository stays clean.
`.cargo/audit.toml` currently carries no vulnerability ignores. Informational
warnings for unmaintained or unsound transitive crates remain visible for
upstream tracking without weakening the vulnerability gate.

The remaining root-lock informational warning (`ttf-parser` unmaintained)
stays non-failing because it is transitive through an already-latest upstream
release. The formerly warned `lru` chain is now on patched 0.18.3. Each
acknowledged entry is re-reviewed whenever its dependency chain updates; new
advisories are not silently ignored. JWT signing and verification use the
`aws_lc_rs` backend of jsonwebtoken, so the rsa crate is not part of the tree
at all.

The independent desktop lock has one narrower reviewed exception:
`RUSTSEC-2024-0429` affects `glib::VariantStrIter` in `glib 0.18.5`, which is
present only in the experimental Linux desktop graph through Tauri 2's GTK3
stack. Neither NovelWorld nor any crate in the resolved graph calls its sole
public constructor, `Variant::array_iter_str`; Windows and macOS do not resolve
`glib`. GTK 0.18 requires `glib ^0.18`, so the patched `glib >=0.20` cannot be
selected until Tauri completes its GTK4/WebKitGTK 6 migration. CI audits the
desktop lock with `--deny unsound` and an exact ignore for this advisory, so a
different unsound advisory still fails. Dependabot tracks the nested manifest
weekly, and desktop dependency PRs are excluded from automatic merge. Re-review
this exception on any Tauri/GTK/Wry graph change or direct use of the affected
API, and remove it when the supported upstream stack is fixed.

CI also runs `cargo deny check licenses sources` with `deny.toml`: every
dependency license must be in the explicitly allowed permissive set (a new
dependency with a license outside the set fails the build and forces a
deliberate review), unlicensed crates are denied, and unknown registry/git
sources are denied. Dependency advisories stay owned by cargo-audit to avoid
maintaining two ignore lists.

Every pushed application image is scanned in the tag pipeline (docker.yml)
with the pinned `aquasec/trivy:0.74.0` for HIGH/CRITICAL vulnerabilities
(--ignore-unfixed, vuln scanner); any finding fails the release. The same
check runs locally via `infra/security/scan-images.sh`. The four base
images in the Dockerfiles are digest-pinned. The digest-pinned
infrastructure images are scanned when they are re-pinned through the
separately approved infrastructure procedure; the current local scan of
the pinned `pgvector/pgvector:pg18@sha256:2ba9ca5f…` (compose `POSTGRES_IMAGE`)
reports 22 findings (21 HIGH, 1 CRITICAL, CVE-2025-68121) inside its
bundled gosu binary, fixed upstream in go 1.24.13 but not yet rebuilt into
the pinned image - tracked for the next infrastructure re-pin. gosu runs
only as the postgres entrypoint's privilege-drop helper, and that path
does not exercise the affected Go TLS session-resumption code.

The 2026-08-31 re-pin moved every Compose image to the newest artifact in its
supported release channel. Prometheus scans clean. The pinned Redis 8.10.1
image reports two HIGH package rows for one OpenSSL QUIC-server issue
(`CVE-2026-14456`); NovelWorld runs ordinary Redis TCP without TLS or QUIC and
does not publish its production port. The pinned Python 3.14.7 drill image
reports two HIGH rows from pip's vendored SBOM (`GHSA-6v7p-g79w-8964` and
`CVE-2025-47273`), but the vulnerable msgpack extension and setuptools
package-index code are absent and the read-only mock imports only the standard
library. The official edge Nginx image and the Grafana, MinIO, pgvector, and
Python drill
images still contain upstream-owned findings. They are not application-release
images and are not reported as clean or silently ignored: each stays
digest-pinned and role-bounded, `docker-compose` Dependabot tracks all of them,
and every new artifact must be re-scanned before re-pinning. The shipped
frontend runtime installs Alpine's fixed OpenSSL packages on the same Nginx
base and passes the application-image gate.

The release pipeline (docker.yml) generates one CycloneDX 1.6 SBOM per
application image with the pinned trivy release and ships them with the
release artifact, bound to the recorded image digest via the generated
`sboms/digests.txt` sidecar;
`infra/security/generate-sboms.sh` is the local operator form.

For registry releases, successful per-run build and vulnerability-scan digest
records drive both the release manifest and SBOM generation; the pipeline does
not resolve mutable SHA tags again. The local generator keeps its explicit
image-ID fallback. Local commands require GNU `timeout`: pulls are bounded to
10 minutes and scans to 15 minutes, each with an additional 30-second
termination grace; no command is retried automatically. A timeout does not
guarantee that daemon work has fully stopped.

#### Release-file provenance

The release workflow's provenance outcome covers the existing flat release
files: `release.env`, six CycloneDX SBOMs, `digests.txt`, four desktop
archives, and `desktop-SHA256SUMS`. After the required quality, image, manifest,
and desktop builds succeed, the pinned `actions/attest` v4.2.2 action produces
native provenance for these exact file subjects and writes the Sigstore bundle
as `release-attestation.json`. A `workflow_dispatch` exercises signing and
verification without publishing a GitHub Release; tag publication remains
blocked until the required checks and native verification of every file pass.

Consumers must obtain the expected source and signer SHA from an independently
reviewed workflow run or operator record, never only from `release.env`. Verify
each consumed file separately with GitHub CLI 2.100.0 or newer, for example:

```bash
expected_sha="$REVIEWED_SOURCE_SHA"
/path/to/gh attestation verify release.env \
  --hostname github.com \
  --repo Wisdoverse/novelworld \
  --signer-workflow Wisdoverse/novelworld/.github/workflows/docker.yml \
  --source-digest "$expected_sha" \
  --signer-digest "$expected_sha" \
  --deny-self-hosted-runners \
  --bundle release-attestation.json
```

Run the same command separately for every SBOM, digest sidecar, desktop
archive, and checksum file. For offline verification, obtain trusted roots
separately with `gh attestation trusted-root` and pass the resulting file with
`--custom-trusted-root`; never trust a root downloaded alongside the release.
A valid signature establishes file content and producing workflow identity;
it does not establish release qualification.

Deploy-time SBOM verification remains a separate gate. Platform-native Windows
code signing, Apple signing/notarization, human review, and release
qualification remain outside this file-provenance outcome.

### Provider Incidents

`tests/e2e/provider_outage_drill.sh` verifies the fail-closed behavior when
the LLM provider disappears: an import terminates with the bounded
`processing_failed` code and a source-free public message, services stay
healthy, a baseline import is untouched, and a retry succeeds once the
provider returns. The settings API never returns key material (only the
`api_key_configured` boolean). Rotating a provider credential against a
live provider remains gated on a real provider and is recorded as such in
DEPLOYMENT_PROFILE.md.

### LLM Security
- User input passed to LLM prompts includes behavioral constraints
- System prompts instruct models to stay in character and refuse harmful content
- Shared provider requests have a 10-second connect timeout, 5-minute total
  deadline, 1 MiB JSON response ceiling, bounded Retry-After, and normalized
  provider errors
- Chat rendering suppresses model-authored Markdown image requests
- This is defense-in-depth — prompt injection is not fully preventable

### Known Limitations
- Refresh tokens stored in plaintext (not hashed) — acceptable for self-hosted
- No CSRF protection (API-only, no cookie auth)
- LLM prompt injection cannot be fully mitigated at the application layer
- Provider logs, provider-hosted generated images, and operator backups are
  outside the application-layer erasure transaction
- Account export snapshots are service-local and sequential, not a globally
  atomic database backup
