# NovelWorld documentation

Use this index to find the owner of a question. Runtime code, migrations, and
tests establish current behavior; these guides define contracts, record decisions,
or describe operational procedures. The GitHub Project owns live roadmap status.

## Choose by task

| Task | Start here |
|---|---|
| Understand supported features and limits | [Product contract](./PRODUCT_CONTRACT.md) |
| Deploy, upgrade, or recover an installation | [Deployment guide](../DEPLOY.md), [operations runbook](./OPERATIONS.md), [backup and restore](./BACKUP_RESTORE.md) |
| Change a service boundary, data owner, or dependency | [Architecture](./ARCHITECTURE.md), then the relevant [ADR](./adr/) |
| Change an API or normative behavior | [Specification](../SPEC.md) and [conformance ledger](./SPEC_CONFORMANCE.md) |
| Review provider configuration, dispatch budgets, or prices | [LLM provider configuration](./LLM_PROVIDERS.md), [pricing guide](./LLM_PRICING.md) |
| Change privacy, retention, export, or security controls | [Security policy](../SECURITY.md), [threat model](./THREAT_MODEL.md), [data retention](./DATA_RETENTION.md), [account export](./ACCOUNT_EXPORT.md) |
| Change import quality or qualification | [Qualification policy](./QUALIFICATION_POLICY.md) and the versioned [extraction-quality policies](#versioned-evidence) |
| Contribute or run checks | [Contributing guide](../CONTRIBUTING.md) |
| Find roadmap direction and exit criteria | [Roadmap](./ROADMAP.md); live status stays in the GitHub Project |

## Contract ownership

- `PRODUCT_CONTRACT.md` describes the currently supported envelope and claims.
- `SPEC.md` is the candidate normative target; `SPEC_CONFORMANCE.md` tracks
  implementation dispositions and evidence. The adjacent
  `SPEC_CONFORMANCE.sha256` is checked by CI.
- `ARCHITECTURE.md` owns service/data boundaries and the private deployment
  profile. Accepted decisions remain in `adr/`; later decisions do not rewrite
  earlier records.
- `OPERATIONS.md` owns health, incident, recovery, and single-node capacity
  procedures. `BACKUP_RESTORE.md` owns the separate backup/restore contract.
- `SECURITY.md` owns reporting and security controls; `THREAT_MODEL.md` owns
  assets, trust boundaries, and threat analysis.

## Versioned evidence

Extraction and H1 policies are versioned records. Preserve their identities,
inputs, thresholds, reports, and failure evidence; do not edit an earlier
version to describe a later decision.

- [Extraction quality v1](./EXTRACTION_QUALITY.md)
- [Structural correction v2](./EXTRACTION_QUALITY_V2.md)
- [Event accounting v3](./EXTRACTION_QUALITY_V3.md)
- [H1 measurement v4](./EXTRACTION_QUALITY_V4.md), with its [prospective examples](./H1_MEASUREMENT_DESIGN.md)
- [Qualification policy and frozen registrations](./QUALIFICATION_POLICY.md)
- [Historical H1 response review](./H1_RESPONSE_EVIDENCE_REVIEW.md)
- [Historical H4 accessibility review](./H4_ACCESSIBILITY_REVIEW.md)
- [Evidence index](./evidence/README.md)

Accepted architecture decisions and the ADR template are discoverable in the
[ADR directory](./adr/). The Chinese-language README and pricing guide remain
available at their existing paths.
