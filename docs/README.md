# NovelWorld Documentation

This page is the entry point for engineering, product, security, and operations
documentation. It identifies the authoritative source for each decision so a
reader does not have to reconcile several partially overlapping documents.

## Start here

| Need | Authoritative document |
|---|---|
| Supported product and deployment envelope | [Product contract](./PRODUCT_CONTRACT.md) |
| Candidate normative behavior | [Specification](../SPEC.md) |
| Current clause-by-clause implementation evidence | [SPEC conformance ledger](./SPEC_CONFORMANCE.md) |
| System boundaries and ownership | [Architecture](./ARCHITECTURE.md) |
| Local or private-preview deployment | [Deployment guide](../DEPLOY.md) |
| Health, monitoring, and recovery entry points | [Operations runbook](./OPERATIONS.md) |
| Security reporting and controls | [Security policy](../SECURITY.md) and [threat model](./THREAT_MODEL.md) |
| Planned work and exit evidence | [Roadmap](./ROADMAP.md) |
| Contribution and review requirements | [Contributing guide](../CONTRIBUTING.md) |

## Source-of-truth model

These documents answer different questions and must not be treated as
interchangeable:

- Runtime code, migrations, and tests prove current behavior.
- `PRODUCT_CONTRACT.md` defines what the current release supports and what it
  does not claim.
- `SPEC.md` defines the candidate target. A normative statement is not evidence
  that the implementation conforms.
- `SPEC_CONFORMANCE.md` records dispositions and evidence for specification
  clauses.
- `ROADMAP.md` defines direction and exit criteria. The GitHub Project owns live
  execution state.

When two sources conflict, do not silently pick the more optimistic statement.
Correct the narrower document in the same change, and update conformance or
roadmap status only when its required evidence exists.

## Document catalog

### Repository guides

| Document | Purpose | Update when |
|---|---|---|
| [README](../README.md) | Product overview, quick start, and top-level navigation | The supported first-run journey or primary documentation entry points change |
| [Deployment guide](../DEPLOY.md) | Private-preview setup, upgrade, rollback, and operator configuration | A deployment command, release contract, port, or configuration requirement changes |
| [Contributing guide](../CONTRIBUTING.md) | Development workflow, verification, and review bar | CI, architecture rules, or review requirements change |
| [Security policy](../SECURITY.md) | Vulnerability reporting and implemented controls | The reporting process or security posture changes |
| [Code of conduct](../CODE_OF_CONDUCT.md) | Community behavior and enforcement | The community policy changes |

### Product and engineering contracts

| Document | Purpose | Update when |
|---|---|---|
| [Product contract](./PRODUCT_CONTRACT.md) | Supported envelope, claims, and responsibility boundaries | A user-visible claim, supported input, or deployment boundary changes |
| [Specification](../SPEC.md) | Candidate normative behavior | The intended cross-component contract changes |
| [SPEC conformance ledger](./SPEC_CONFORMANCE.md) | Evidence and disposition for every normative clause | Implementation evidence or a clause disposition changes |
| [Qualification policy](./QUALIFICATION_POLICY.md) | Release evidence classes and thresholds | A qualification slice, guardrail, or approval rule changes |
| [Roadmap](./ROADMAP.md) | Ordered outcomes, invariants, and exit criteria | Product direction or evidence-gated sequencing changes |

### Architecture and data

| Document | Purpose | Update when |
|---|---|---|
| [Architecture](./ARCHITECTURE.md) | Service ownership, data authority, and dependency rules | A boundary, owner, dependency direction, or consistency model changes |
| [Data retention](./DATA_RETENTION.md) | Retention, deletion, and external data boundaries | Stored data or deletion behavior changes |
| [Account export](./ACCOUNT_EXPORT.md) | Export wire format and completeness contract | Exported fields, ordering, limits, or completion semantics change |
| [Extraction quality](./EXTRACTION_QUALITY.md) | Supported slices and extraction thresholds | A parser slice, metric, judge rubric, or threshold changes |
| [Import budget](./IMPORT_BUDGET.md) | Provider work limits during import | Provider calls, retry behavior, or budget evidence changes |

### Reliability, security, and operations

| Document | Purpose | Update when |
|---|---|---|
| [Deployment profile](./DEPLOYMENT_PROFILE.md) | Explicit topology and responsibility decisions | The supported deployment profile changes |
| [SLO and capacity contract](./SLOS.md) | Measured single-node workload and decision thresholds | Workload, objective, measurement, or topology changes |
| [Operations runbook](./OPERATIONS.md) | Health checks, alerts, and incident playbook index | A probe, alert, dashboard, or recovery command changes |
| [Backup and restore](./BACKUP_RESTORE.md) | Backup artifacts, RPO/RTO, restore, and drills | Persistence, retention, restore procedure, or recovery target changes |
| [Security policy](../SECURITY.md) | Vulnerability reporting and landed security controls | A security control, dependency posture, or response process changes |
| [Threat model](./THREAT_MODEL.md) | Assets, trust boundaries, threats, and mitigations | A data flow, trust boundary, attacker capability, or mitigation changes |

## Documentation standard

Every material document should make the following clear in its opening section
or through this catalog:

1. Audience and decision owned by the document.
2. Scope, non-goals, and supported deployment profile.
3. Authoritative implementation or evidence links.
4. Failure behavior, rollback or recovery boundary, and known gaps where
   relevant.
5. The event that requires the document to be reviewed.

Use concrete commands, versioned contracts, and stable repository-relative
links. Label proposals, targets, and unqualified behavior explicitly. Avoid
manual test counts, screenshots of text, duplicated configuration, and claims
whose only evidence is another prose document.

Documentation is part of the change, not a follow-up task. The pull request
author owns updates while a change is under review; repository maintainers own
the merged corpus. Review on behavioral change rather than a calendar date so
the document and implementation land atomically.

## Design decisions

Use an architecture decision record only for a durable decision that changes a
service boundary, data ownership, trust boundary, public contract, consistency
model, availability target, or irreversible dependency. Routine implementation
choices belong in the pull request.

Copy [the ADR template](./adr/0000-template.md) to
`docs/adr/NNNN-short-title.md`. An ADR must be accepted in the same change that
introduces the decision. Later decisions supersede earlier records; they do not
rewrite history.

Accepted records:

- [ADR 0001: Source-bound shared rules and server-owned D20 resolution](./adr/0001-source-bound-advanced-game-rules.md)
- [ADR 0002: L0-L4 minimal bootstrap and deferred runtime configuration](./adr/0002-minimal-bootstrap-and-deferred-runtime-configuration.md)
- [ADR 0003: H4 private-preview journey qualification](./adr/0003-h4-journey-qualification.md)
