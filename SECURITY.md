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
`current.env` and `previous.env` with a flock-guarded lock, and a rollback
that dies mid-promotion leaves a `rollback.pending` marker that the next
locked operation recovers before doing anything else. (Upgrades promote
atomically between two staged manifests and do not use the marker.)

What the release_state_drill.sh proves locally (no registry required):

- Manifest grammar fails closed: non-digest images, malformed git SHAs and
  versions, unknown, duplicate, empty, or missing keys are all rejected.
- Every upgrade and rollback guard fires before `deploy_manifest`'s secrets
  check (and therefore before any network access): a divergent manifest for
  the current SHA, infrastructure-image changes, malformed rollback targets,
  a missing or mismatched previous release, and concurrent operations (held
  lock) all stop with an actionable error.
- An interrupted rollback recovers its current/previous pair before the next
  command, and a wedged marker with missing files fails closed: the marker
  survives and requires explicit operator clearing.

What stays gated: the image-level deployment (`deploy_manifest`: git checkout
of the release SHA plus `compose pull` of the digest-pinned images) runs only
with a reachable registry. CI validates the manifest grammar and lock
guards; the deployment path itself is not exercised by a local drill.
SBOM generation has since landed (see Dependency Policy); deploy-time SBOM
verification, provenance/attestation, and signing remain gated.
### Dependency Policy

CI runs `cargo audit` against `Cargo.lock` with the live RustSec
advisory database: any newly reported vulnerability fails the build. CI also
runs `gitleaks` over the full commit history: any committed secret fails the
build. `.gitleaks.toml` is the full default rule set plus narrow allowlists for
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

Informational warnings (`ttf-parser` unmaintained, `lru` unsound pop patched
in 0.18.2) remain non-failing because both are transitive through
already-latest upstream releases. Each acknowledged entry is re-reviewed
whenever its dependency chain updates; new advisories are not silently
ignored. JWT signing and verification use the `aws_lc_rs` backend of
jsonwebtoken, so the rsa crate is not part of the tree at all.

CI also runs `cargo deny check licenses sources` with `deny.toml`: every
dependency license must be in the explicitly allowed permissive set (a new
dependency with a license outside the set fails the build and forces a
deliberate review), unlicensed crates are denied, and unknown registry/git
sources are denied. Dependency advisories stay owned by cargo-audit to avoid
maintaining two ignore lists.

Every pushed application image is scanned in the tag pipeline (docker.yml)
with the pinned `aquasec/trivy:0.68.1` for HIGH/CRITICAL vulnerabilities
(--ignore-unfixed, vuln scanner); any finding fails the release. The same
check runs locally via `infra/security/scan-images.sh`. The four base
images in the Dockerfiles are digest-pinned. The digest-pinned
infrastructure images are scanned when they are re-pinned through the
separately approved infrastructure procedure; the current local scan of
the pinned `pgvector/pgvector@sha256:69167330…` (compose `POSTGRES_IMAGE`)
reports 22 findings (21 HIGH, 1 CRITICAL, CVE-2025-68121) inside its
bundled gosu binary, fixed upstream in go 1.24.13 but not yet rebuilt into
the pinned image - tracked for the next infrastructure re-pin. gosu runs
only as the postgres entrypoint's privilege-drop helper, and that path
does not exercise the affected Go TLS session-resumption code.

The release pipeline (docker.yml) generates one CycloneDX 1.6 SBOM per
application image with the pinned trivy release and ships them with the
release artifact, bound to the recorded image digest via `sboms/digests.txt`;
`infra/security/generate-sboms.sh` is the local operator form.

Still-open H2 supply-chain gates: deploy-time SBOM verification,
provenance/attestation, and signature generation for official release
artifacts.

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
