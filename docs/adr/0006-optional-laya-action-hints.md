# ADR 0006: Optional local action hints

- Status: Accepted decision; implementation delivered in the same change
- Date: 2026-09-24
- Owners: Narrative and frontend

## Context

The open-world form already asks the reader to choose an action type and target.
Laya can classify a short Chinese intent, but its base checkpoint is not
qualified for NovelWorld and can make confident mistakes. Action hints may
reduce form friction without changing who owns a world turn.

## Decision

When both `LAYA_API_URL` and `LAYA_API_KEY` are configured, Narrative exposes an
authenticated optional `POST /world/action-suggestion`. It first reuses the
owned, self-identity, source-visible open-world read. It sends only the reader's
bounded intent and currently valid action-type descriptions to the separate
Laya `/v1/systemone` service. It sends no novel text, target IDs, world state,
or credentials other than the Laya bearer token. The client has a 300 ms connect
deadline, 2 s total deadline, four in-flight slots, a 16 KiB response ceiling,
and no retry. Invalid, unavailable, or low-probability output yields no hint.
The probability cutoff is an abstention heuristic, not a calibrated quality
guarantee.

The browser sends the intent only after a click and displays a type suggestion.
A second click on the suggestion selects its type and clears the selected target;
the reader still confirms the target and submits the turn. Edits, a new world
view, and locked or pending turns discard stale hints. The existing turn API,
validation, idempotency, and DeepSeek generation remain authoritative. Without
both configuration values, the feature is absent from the form.

## Alternatives considered

- Keep only the existing action dropdown: safest and still the default.
- Call Laya from the browser: would expose the token and couple every reader to
  the model network.
- Automatically select or submit the predicted type: a classification mistake could
  silently change the player's intended action.

## Rollout and rollback

No schema or durable data changes. Connect Laya to a private network reachable
from Narrative, then set the two environment values and recreate Narrative.
Unset either value and recreate Narrative to remove the hint. Laya outage must
leave manual actions available.

## Evidence limits

A local synthetic Chinese sample was classified correctly in 12 of 14 cases;
it is a development observation, not a representative quality gate. Adapter,
authorization, browser state, architecture, and CI checks establish only their
respective structural behavior. No provider or product qualification follows.
The [fixture-based browser screenshot](../evidence/laya-action-suggestion.png)
records the visible hint before the player selects it.
