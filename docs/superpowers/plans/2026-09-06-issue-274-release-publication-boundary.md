# Issue #274 Release Publication Boundary Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make release-file provenance publication fail closed so manual validation can never publish and one GitHub Release can contain only one verified same-run artifact generation.

**Architecture:** Keep the existing single Docker Image workflow, release artifact, attestation job, and GitHub CLI. Tighten only the final publication job: admit tag-push events and pass every verified asset to one native `gh release create` command. GitHub CLI refuses an existing Release, stages uploads in a draft, publishes only after all uploads succeed, and cleans the draft after an upload or publish failure. This changes release orchestration only; frontend FSD and backend Cloud Native/DDD/microservice boundaries are unaffected.

**Tech Stack:** GitHub Actions YAML, GitHub CLI, Python `unittest`, Markdown.

**Spec:** [Roadmap issue #274](https://github.com/Wisdoverse/novelworld/issues/274), including the takeover review comment dated 2026-09-06.

**Global constraints:** Preserve the exact pinned `actions/attest` action, its documented permissions, the 14-file flat release artifact, public filenames, native positive/negative verification, tag naming, and same-run dependency chain. Do not publish a tag or Release during validation. Do not add a script, dependency, profile flag, retry loop, custom cleanup, frontend change, backend change, or broader H2 claim. If GitHub CLI cannot clean its temporary draft after failure, the leftover Release must block an automatic rerun until an operator diagnoses and removes it.

---

## Task 1: Lock the publication boundary with a failing regression

**Files:**

- Modify: `tests/e2e/release_image_digest_test.py`
- Read: `.github/workflows/docker.yml`

- [ ] Add `WORKFLOW = ROOT / ".github/workflows/docker.yml"` beside the existing path constants.
- [ ] Add one `ReleaseImageDigestTest.test_publication_is_new_tag_push_only` method that extracts the final `github-release` job and asserts all four contract points:

```python
workflow = WORKFLOW.read_text()
publish = workflow.split("\n  github-release:\n", 1)[1]
self.assertIn(
    "if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')",
    publish,
)
self.assertIn('gh release create "$tag" ./* "${flags[@]}"', publish)
for forbidden in ("gh release view", "gh release upload", "gh release edit", "--draft", "--clobber"):
    self.assertNotIn(forbidden, publish)
```

- [ ] Run `python3 tests/e2e/release_image_digest_test.py` and confirm the new test fails against the current workflow because the event guard and single native create are absent while the custom draft/upload/edit state machine is present.
- [ ] Review the test failure to ensure it is caused only by the intended missing behavior; do not loosen unrelated existing assertions.

## Task 2: Make first publication single-generation and fail closed

**Files:**

- Modify: `.github/workflows/docker.yml:270`
- Modify: `SECURITY.md:250`
- Test: `tests/e2e/release_image_digest_test.py`

- [ ] Change the `github-release` job condition to require both a `push` event and a `refs/tags/v*` ref:

```yaml
if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')
```

- [ ] Replace the existing-release state machine and `--clobber` upload with one native create call that receives all assets. Do not pass `--draft`: GitHub CLI owns the temporary draft, failure cleanup, and final publish transition when assets are supplied.

```bash
flags=(--verify-tag --generate-notes)
[[ "$tag" == *-* ]] && flags+=(--prerelease)
gh release create "$tag" ./* "${flags[@]}"
```

- [ ] Keep the existing `needs: release-attestation`, `contents: write` job scope, artifact download, timeouts, filenames, and attestation permissions unchanged.
- [ ] Update `SECURITY.md` to state that `workflow_dispatch` is validation-only even when its selected ref is a tag; tag-push publication uses the CLI's native create-with-assets transaction, which refuses every existing draft or public Release and cleans its temporary draft after upload/publish failure. If cleanup itself fails, require diagnosis and explicit removal before another run.
- [ ] Run `python3 tests/e2e/release_image_digest_test.py` and confirm all four tests pass.

## Task 3: Final verification and review

**Files:**

- Review: `.github/workflows/docker.yml`
- Review: `tests/e2e/release_image_digest_test.py`
- Review: `SECURITY.md`
- Review: `docs/superpowers/plans/2026-09-06-issue-274-release-publication-boundary.md`

- [ ] Run `git diff --check`.
- [ ] Run `python3 tests/e2e/release_image_digest_test.py` once more from a clean test process.
- [ ] Inspect `git diff -- .github/workflows/docker.yml tests/e2e/release_image_digest_test.py SECURITY.md` for event-context mistakes, permission expansion, shell quoting, accidental retries, destructive cleanup, filename drift, secret exposure, and unsupported completion claims.
- [ ] Record frontend FSD as N/A because no frontend file or import edge changes.
- [ ] Record backend Cloud Native/DDD/microservice architecture as N/A because no runtime package, dependency call, SQL, service boundary, or authoritative state changes.
- [ ] Push the final commit, open a PR with `Closes #274`, run required CI on the exact head, and request independent final diff review. Do not return the Project item to Done until the merge commit is on `main` and required CI is green.
