## Problem and outcome

<!-- Why is this change needed? State the user/operational outcome and non-goals. -->

Closes #

## Behavior

<!-- Describe externally visible behavior, compatibility impact, and important failure cases. -->

## Risk assessment

<!-- Mark each category Low/Medium/High or N/A with a reason. -->

| Category | Level | Evidence or mitigation |
|---|---|---|
| Security / privacy | | |
| Data integrity / migration | | |
| Reliability / performance / cost | | |
| API / client compatibility | | |
| Accessibility / UX | | |

## Implementation

<!-- Summarize the key design choices. Link an ADR for boundary-level decisions. -->

## Verification

<!-- List exact commands, automated evidence, and manual scenarios. Do not write only "CI passes". -->

| Check | Result or link |
|---|---|
| Unit / contract tests | |
| Integration / browser tests | |
| Negative and failure cases | |
| UI evidence, if applicable | |

## Rollout and recovery

<!-- State rollout steps, success/abort signals, and rollback or forward recovery. Use N/A with a reason for docs-only changes. -->

## Observability

<!-- Name the health check, metric, alert, or log signal that detects regressions. -->

## Documentation

<!-- Link updated contracts/runbooks/ADRs, or explain why behavior documentation is unaffected. -->

## Author checklist

- [ ] The change is one independently mergeable outcome with explicit non-goals.
- [ ] DDD service ownership and FSD import direction remain valid.
- [ ] New behavior has evidence proportional to its risk and blast radius.
- [ ] Secrets, personal data, and provider content are absent from commits and evidence.
- [ ] Migration, rollout, and recovery are compatible with the supported deployment profile.
- [ ] User-visible and contract changes update documentation in this pull request.
