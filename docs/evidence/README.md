# Review evidence

User-visible changes keep a representative screenshot here so pull requests
can link stable, versioned evidence alongside the browser test that produced it.

Machine-readable, sanitized live baselines also live here. Raw provider metrics,
prompts, model prose, source text, identities, and secrets remain outside Git.

## Advanced rules implementation review

The [world-agency attributes screenshot](./world-agency-attributes.png) uses
synthetic browser fixtures to show the latest story, character actions, attribute
deltas, and canon event time. It is presentation evidence only, not live-provider
quality or deployment evidence.

Pre-implementation review rejected a separate rules service, executable formula
DSL, unbounded per-action adjudication, and per-reader templates. It made
progress safety, exact version binding, leases, provider budgets, and the meaning
of a successful check explicit before implementation.

Post-implementation review corrected template immutability under an exact
version, prevented technical failures from rerolling the same action/state, and
bounded failed v1 template generation to three claims (one logical provider call
per claim, with bounded transport retries). V2 shares the cross-prompt canonical
budget. The default narrative path performs no template generation or dice
work.

The independent local review and checks covered the v1/Laya implementation,
not v2 templates or migration 0030. V2 requires its own runtime and required-CI
evidence. No live-provider or deployment result is asserted by that review; the
separate evidence owner is [issue #418](https://github.com/Wisdoverse/novelworld/issues/418).

The [failed-action confirmation screenshot](./failed-action-confirmation.png)
uses a synthetic rejected generation to show automatic status confirmation and
unlocking without a second submission. It is browser presentation evidence,
not live provider or deployment evidence.

## Interface language

The [English interface](./ui-language-english.png) shows the default locale and
accessible language selector while preserving Chinese story samples. The
[English](./failed-action-confirmation.png) and
[Simplified Chinese](./failed-action-confirmation-zh-CN.png) failed-action views
show the same automatic confirmation flow in both locales. These screenshots
use synthetic browser fixtures, including deliberate long-text wrapping; they
are presentation evidence, not live-provider or accessibility qualification.
