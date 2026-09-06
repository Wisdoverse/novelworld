# NovelWorld Extraction Quality Policy v2 (Prospective)

Status: **prospective, unimplemented, and not adopted for Qualification**.
Version: **`extraction-quality-v2`**. This amendment inherits the
[reviewed v1 policy](https://github.com/Wisdoverse/novelworld/blob/84921cd59387b01619b6b75ae81e81b0f45c11e1/docs/EXTRACTION_QUALITY.md)
and [v3 corpus](https://github.com/Wisdoverse/novelworld/blob/84921cd59387b01619b6b75ae81e81b0f45c11e1/tools/h1-eval/corpus/v1.json)
at main SHA `84921cd59387b01619b6b75ae81e81b0f45c11e1`. Until a separately
reviewed implementation lands, `extraction-quality-v1` and the current
`h1-synthetic-v3` remain the executable truth.

## Purpose and boundary

The two Salt stories contain a Chapter 3 account of a missing person's last
night boat and a Chapter 4 resignation letter saying that no night boat
existed. The sources establish that the letter makes that claim; they do not
establish that the letter resolves the contradiction or that no night boat
ever existed. The current v1 expected world rule incorrectly promotes that
claim to a hard source-grounded rule.

This v2 policy corrects that oracle prospectively. It does not rewrite sources,
remove the letter fact, rescore an old report, compare quality across policy
versions, authorize a provider call, or adopt a new formal Qualification
cohort. The existing v1 policy, v3 corpus, recorded outputs, and all prior
failure evidence remain immutable historical evidence.

## Inherited contract

Future v2 evaluation inherits every supported slice, case, source, numerical
threshold, composition minimum, judge category, extraction/judge schema and retry contract, and
negative/adversarial mechanism from v1. Retain all 19 existing cases: five
positive, one local splitter, eight adversarial and five malformed. The six
registered live-report cases are the five positives plus the local splitter;
no offline case may be removed either. Retain all source text, all other expected facts, hostile-instruction
and malformed-input coverage, and the unchanged `80/80/20/0/100` thresholds.

The existing match/partial/absent/hallucinated rubric, exact token coverage,
provenance and chronology checks and fail-closed model/category validation
remain unchanged. The judge's application-level retry is one identical request
only for an invalid judge contract, never for a transport failure or a valid
low score. Existing shared LLM transport retries remain unchanged.

## Prospective oracle correction

Only future corpus v4 and explicitly v2-identified calibration/report artifacts
may apply this correction:

- In both Salt cases (`zh-gbk` and `en-bom-utf16`), remove only the unsupported
  hard world-rule expectation `wr1` about the resignation-letter claim. Preserve the
  narrative letter fact, both source passages, every other expected fact, and
  every case identity.
- The world-rule denominator therefore changes from 2 to 1 in each affected
  case. No other category denominator changes.
- Remove `wr1` from each affected recorded calibration's hard world rules,
  while retaining the source-grounded narrative-letter fact and all other
  calibration facts and adversarial failure mechanisms.
- For each affected language, an offline counterexample must prove that the
  unsupported assertion as a separate extracted hard world rule is counted
  as hallucinated under the inherited precision semantics. This is not a new
  zero-tolerance rule: the case may still pass the aggregate precision gate.
  It does not guarantee that a judge detects every unsupported clause inside
  an otherwise grounded compound rule.

No expected fact may be deleted or relabeled for a result already produced.
The v1 corpus and reports are not recomputed, repaired, or used for a
cross-version production-quality comparison.

## Versioned adoption conditions

A future implementation must use an explicitly versioned `extraction-quality-v2`
policy identity and `h1-synthetic-v4` corpus identity in every report. It must
version the report metadata extension explicitly; the inherited extraction
and judge response schemas do not prohibit adding this report identity. It must
verify unchanged source/case identities, nonempty categories, the two corrected
world-rule denominators, retained letter facts, unsupported-claim
hallucination accounting, unchanged thresholds, and unchanged adversarial
coverage. Version mismatch must fail closed. Recorded evaluation must pass
twice with byte-identical output before final implementation review.

This document does not automatically change the formal journey runner or make
v2 evidence satisfy the frozen v1 Qualification contract. A future formal
Qualification cohort requires its own reviewed adoption/version decision after
the implementation and its deterministic evidence are complete.

## Evidence and rollback

This is a documentation-only policy decision, not measured model quality,
qualification, or a paid-run authorization. No runtime, prompt, rubric,
provider, database, deployment, DDD, or FSD change is included. Existing
reports and failed results remain frozen. The prospective policy may be
withdrawn before implementation; after any v2 run starts, its evidence remains
version-bound and cannot be reclassified as v1 evidence.

The implementing agent owns this amendment; a non-author agent reviews its
source adjudication and final evidence under the private-iteration policy.
Re-review before implementation, any paid registration, formal Qualification
adoption, or any change to these versioned inputs. A later decision cannot
rewrite a Started cohort. The [source-level adjudication](https://github.com/Wisdoverse/novelworld/issues/229#issuecomment-5559785641)
and [policy work item](https://github.com/Wisdoverse/novelworld/issues/304)
retain the decision history; neither is a model-quality pass.
