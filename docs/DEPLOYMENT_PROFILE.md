# Deployment Profile Decisions

Version: **`deployment-profile-v1`**. The supported deployment profile is the
private self-hosted preview defined in [`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md)
(deployment envelope and responsibility boundary); PRODUCT_CONTRACT remains the
authority for what is supported now. This document records the H2 boundary
decisions for that profile and approves nothing beyond it.

## Profile statement

- **Supported:** operator-run, single-node, private self-hosted deployment
  with users operator-admitted via network isolation (no application invite
  gate) and operator-terminated TLS.
- **Not supported:** internet-hosted/public operation. The public edge is
  treated as defensive analysis only ([`THREAT_MODEL.md`](./THREAT_MODEL.md)),
  and the ROADMAP gates any internet-hosted claim.

## Boundary decisions

1. **TLS — operator duty.** The deployment terminates TLS at an
   operator-provided edge in front of the compose stack (PRODUCT_CONTRACT
   responsibility boundary). The shipped nginx profile serves plain HTTP
   with baseline security headers; it is not a TLS substitute.
2. **Registration verification / invites — not applicable, deliberately not
   built.** Users exist because the operator creates them (first-run admin
   setup) or because an operator-admitted person registers against the
   operator's own deployment; admission is the private network boundary,
   not an application gate. No email verification, invite tokens, or
   public signup (the schema carries an always-false `email_verified`
   field that no flow ever sets). Reopens only for a public profile.
3. **Content-safety boundary — not built, not applicable.** There is no
   public submission or generation surface; every user is operator-invited.
   Moderation, complaints/takedown, and reporting machinery do not exist and
   must not be assumed (PRODUCT_CONTRACT responsibility boundary). Reopens
   only for a public profile.
4. **Provider boundary — operator-configured.** Any OpenAI-compatible
   provider URL/model/key may be configured by the operator; the first-run
   setup offers preset providers. Per-principal quotas, global spend
   ceilings, and kill switches are deferred to a public profile and are not
   built for the private one.
5. **Privacy, consent, retention — operator duty with implemented
   boundaries.** What leaves the deployment (novel excerpts, prompts, chat
   content, provider calls) is disclosed in [`DATA_RETENTION.md`](./DATA_RETENTION.md)
   and [`SECURITY.md`](../SECURITY.md) (LLM Security); export and erasure obligations are
   implemented and drilled (ACCOUNT_EXPORT.md, DATA_RETENTION.md). Provider
   contract review and user consent are operator duties, not implemented
   features. Data minimization has no separate implemented control beyond
   the retention and erasure paths and must not be claimed.
6. **Software supply chain — implemented boundaries.** Dependency
   vulnerability gate (cargo-audit), committed-secret scanning (gitleaks),
   license/source policy (cargo-deny), container image scanning (trivy), and
   digest-pinned release manifests with a rollback state machine
   (release.sh). See [`SECURITY.md`](../SECURITY.md) 'Dependency Policy' and 'Release Rollback'.
   CycloneDX SBOMs are generated per release (docker.yml) and locally via
   `infra/security/generate-sboms.sh`, digest-bound; deploy-time SBOM
   verification, provenance/attestation, and signing remain open
   release-infrastructure work.
7. **Incident response — existing procedures.** Secret rotation
   ([`SECURITY.md`](../SECURITY.md) and its e2e drill), the bad-release edge drill, the release/rollback
   state-machine drill, and the provider-outage drill (fail-closed import,
   bounded source-free errors, settings non-disclosure, recovery retry) are
   implemented and verified locally. Provider credential rotation against a
   live provider and the remaining incident scenarios stay open H2 work.

## Reopening criteria

Selecting a public or internet-hosted profile reopens: registration
verification/invites, per-principal and per-operation quotas, global spend
ceilings and kill switches, moderation/complaints/takedown, an enforceable
public content-safety boundary, and provider qualification per
[`QUALIFICATION_POLICY.md`](./QUALIFICATION_POLICY.md) — the ROADMAP H2 scope
owns those decisions.
