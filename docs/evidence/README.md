# Review evidence

User-visible changes keep a representative screenshot here so pull requests
can link stable, versioned evidence alongside the browser test that produced it.

Machine-readable, sanitized live baselines also live here. Raw provider metrics,
prompts, model prose, source text, identities, and secrets remain outside Git.

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
