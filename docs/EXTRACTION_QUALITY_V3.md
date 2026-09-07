# Extraction Quality Policy v3 — single-source event accounting

Status: **prospective policy only; not implemented or formally adopted for Qualification**.
Version: **`extraction-quality-v3`**. This decision owns the next H1 judge response
contract, not production extraction or a provider quality claim. It inherits
[v2](https://github.com/Wisdoverse/novelworld/blob/bda240656d145d4261a2f5bf204e1bc6da7c13e3/docs/EXTRACTION_QUALITY_V2.md)
and its corpus at immutable main `bda240656d145d4261a2f5bf204e1bc6da7c13e3`.
Implementation requires a separate reviewed change after this policy is merged.

## Rationale and boundary

The old judge returns expected-event semantic mappings and a second
`extracted_event_verdicts` array. Validation requires that array's Match set to
equal exactly the unique tokens mapped by expected Match/Partial verdicts; it
adds no independent semantic decision. In the
[failed bda Diagnostic](https://github.com/Wisdoverse/novelworld/issues/229#issuecomment-5565187868),
an illegal label in the redundant array or a contradiction with the mappings
correctly failed two cases under the old contract. Those responses and all
historical results remain frozen.

V3 removes duplicate model bookkeeping prospectively. It does not repair those
responses, solve production omissions or semantic overmatches, lower thresholds,
authorize a paid run, or unlock H3/H4. Removing the field changes schema acceptance
and may change generated judgments; it does not preserve raw-response acceptance
or establish model quality or causal improvement.

## Inherited invariants

Retain all supported slices, 19 offline cases and six live-report cases, sources,
expected facts, recorded artifacts, thresholds (`80/80/20/0/100`), composition and
anti-vacuity requirements. Keep full-event semantic completeness, faithful
cross-language equivalence, source provenance, mapped chronology, and every other
category and world-rule support contract unchanged. Partial earns no full recall.

Preserve model allowlisting, required private evidence, usage/budget accounting,
transport retry contracts and the one identical application-level judge retry
only for invalid JSON/schema/rubric/token/explanation. A valid low score is final;
no historical response or report may be reused or rescored.

## Prospective response and scoring contract

Remove only the `extracted_event_verdicts` key from the model response. Keep every
other response field and existing validator. Strict unknown-field rejection must
reject the old key even if its array is empty or consistent. Do not accept both
shapes, delete received keys, synthesize a replacement array, or repair JSON.

The complete expected-event verdict list still covers every expected token exactly
once. Match/Partial requires one known extracted-event token; tokens cannot be
reused across expected events. Absent has no mapping. Preserve the existing
`Option<String>` behavior: omitted mapping key or explicit null is accepted for
Absent; either is rejected for Match/Partial. Asking for explicit null in the
prompt does not introduce a new mandatory-key validator in this amendment.

Let E be the actual extracted-event token universe and M the validated unique
tokens referenced by expected Match/Partial verdicts, with M a subset of E.

| Quantity | Authoritative computation |
|---|---|
| Full-event recall numerator | Number of expected Match verdicts; Partial contributes zero |
| Extracted-event precision denominator | Size of E, not size of M |
| Matched extracted events | Size of M, including valid Partial mappings |
| Unmatched extracted events | Size of E minus size of M |
| Chronology violations | Existing relative-order check on validated semantic mappings |

Every unmapped actual event stays in the denominator, including source-grounded
finer-grained events; the inherited Hallucinated label is not a count of fabricated
facts. Empty mappings cannot create a divide-by-zero or vacuous full score.
Calculate counts directly from validated mappings and actual extraction, not
model-provided totals. No derived verdict array is needed.

## Versioned implementation and evidence

The later implementation uses policy `extraction-quality-v3`, corpus
`h1-synthetic-v5`, response rubric identity `h1-extraction-v3` and judge prompt
`h1-semantic-judge-v8`. The rubric identity records the response-contract change;
scoring semantics remain unchanged. Corpus v5 changes only the three
policy/corpus/rubric identity fields from v4; all remaining JSON content must be
identical. Report schema 3 retains its existing shape and explicit identities;
the private HTTP envelope remains schema 2.

Before implementation acceptance, require:

- Synthetic valid-response scoring equivalence for complete, Partial, Absent,
  permuted mappings, unmatched prefixed/additional events and full denominators.
- Rejection of duplicate/unknown/missing required mappings and tokens, retained
  chronology inversion and anti-vacuity checks, and unchanged other-category,
  rule-support, hostile-input and retry guards. Test the omitted Absent behavior
  separately from rejected omitted Match/Partial mappings.
- Rejection of the old field whether empty, consistent or contradictory; no
  old-response replay or compatibility path.
- Corpus equality excluding only the three identity fields, all retained
  adversarial mechanisms, recorded 19/19 twice byte-identical at the final SHA,
  affected local/required CI and independent final code/contract/evidence review.

The frozen formal H4 v1 cohort retains its old corpus-hash and policy guards;
v3 cannot satisfy or silently replace it. A future formal adoption requires a
separate versioned decision/cohort. Any new Diagnostic separately registers its
immutable inputs, model, budgets, private namespace and independent approval
before calls. This policy is not that approval.

## Delivery, rollback and review

Policy-only delivery changes no runtime, public API, DDD/FSD boundary, deployment,
database or provider configuration. Retracting prospective authorization does not
rewrite history; implementation rollback invalidates only affected new evidence.
Never reclassify a Started v3 result as v2/v1 or turn an old failure into a pass.

The implementing agent owns this decision; a non-author agent reviews it under
the private-iteration policy. Re-review before implementation, paid registration,
formal adoption or changes to any versioned input. Human acceptance does not
block private iteration; unperformed human and formal evidence stays unverified.
The [work item](https://github.com/Wisdoverse/novelworld/issues/316) records review
and delivery evidence. A merged policy is not Structural implementation or
Release-qualified evidence.
