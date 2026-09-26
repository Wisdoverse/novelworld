# ADR 0007: Regional LLM API and subscription presets

Status: Accepted for configuration compatibility; live qualification is separate.

## Context

Settings only offered DeepSeek/OpenAI and the shared adapter always appended `/v1`.
Domestic providers use different API bases, region accounts and subscription Keys.
Reusing a Key during a provider switch could disclose it to another service.

## Decision

Keep the existing shared OpenAI-compatible adapter and User Service credential owner.
Use fixed official HTTPS presets with distinct provider IDs for each region/API/plan.
Allow bounded editable model IDs for new providers. Require a new Key on a variant
switch, including variants sharing a URL. Append resource paths directly to explicit
API bases; retain root-URL `/v1` behavior. Add optional-consumer provider metadata to
internal configuration contract 2; immutable Diagnostic bindings remain unchanged.
Display actual subscription-use limitations. See [provider guide](../LLM_PROVIDERS.md).

## Consequences

No new adapters, runtime packages, relations or dependencies are needed. Existing
DeepSeek IDs remain readable. Older config consumers may label new variants as
`environment` until upgraded. Custom base prefixes must include the API version.
Offline endpoint, credential, request and browser checks do not qualify paid provider
behavior or permit subscriptions beyond their terms. H3/H4 journeys remain DeepSeek.
