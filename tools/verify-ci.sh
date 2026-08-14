#!/usr/bin/env bash
set -euo pipefail

workflow=.github/workflows/ci.yml
dry_run=false

die() {
  printf 'verify: %s\n' "$*" >&2
  exit 1
}

if [[ ${1:-} == --dry-run && $# == 1 ]]; then
  dry_run=true
elif (( $# != 0 )); then
  die 'usage: make verify (internal self-check: bash tools/verify-ci.sh --dry-run)'
fi

for dependency in git gh; do
  command -v "$dependency" >/dev/null || die "$dependency is required"
done

root=$(git rev-parse --show-toplevel 2>/dev/null) || die 'run from a Git checkout'
cd "$root"

[[ -z $(git status --porcelain --untracked-files=all) ]] ||
  die 'checkout must be clean, including untracked files'

local_branch=$(git symbolic-ref --quiet --short HEAD) ||
  die 'detached HEAD cannot identify a remote workflow ref'
remote=$(git config --get "branch.$local_branch.remote") ||
  die 'current branch has no upstream remote'
merge_ref=$(git config --get "branch.$local_branch.merge") ||
  die 'current branch has no upstream branch'
[[ $remote != . && $merge_ref == refs/heads/* ]] ||
  die 'current branch must track a remote branch'
remote_branch=${merge_ref#refs/heads/}
remote_url=$(git remote get-url "$remote") || die "cannot read remote $remote"
repo=$(gh repo view "$remote_url" --json nameWithOwner --jq .nameWithOwner) ||
  die "cannot resolve GitHub repository for remote $remote"

sha=$(git rev-parse HEAD)
[[ $sha =~ ^[0-9a-f]{40}$ ]] || die "local HEAD is not a GitHub commit SHA: $sha"
remote_line=$(git ls-remote --exit-code "$remote" "refs/heads/$remote_branch") ||
  die "upstream branch $remote/$remote_branch does not exist"
remote_sha=${remote_line%%$'\t'*}
[[ $remote_sha == "$sha" ]] ||
  die "upstream is $remote_sha, not local HEAD $sha; push the exact commit first"

gh workflow view "$workflow" --ref "$remote_branch" --yaml --repo "$repo" >/dev/null
if $dry_run; then
  printf 'verify: would dispatch %s/%s at %s (%s)\n' \
    "$repo" "$workflow" "$remote_branch" "$sha"
  exit 0
fi

previous_run=$(gh run list \
  --workflow "$workflow" \
  --commit "$sha" \
  --event workflow_dispatch \
  --limit 1 \
  --json databaseId \
  --jq '.[0].databaseId // ""' \
  --repo "$repo")

gh workflow run "$workflow" --ref "$remote_branch" --repo "$repo"

run_id=
for ((attempt = 0; attempt < 45; attempt++)); do
  run_id=$(gh run list \
    --workflow "$workflow" \
    --commit "$sha" \
    --event workflow_dispatch \
    --limit 1 \
    --json databaseId \
    --jq '.[0].databaseId // ""' \
    --repo "$repo")
  if [[ -n $run_id && $run_id != "$previous_run" ]]; then
    break
  fi
  sleep 2
done
[[ -n $run_id && $run_id != "$previous_run" ]] ||
  die "workflow run for $sha did not register within 90 seconds"

run_url=$(gh run view "$run_id" --json url --jq .url --repo "$repo")
printf 'verify: waiting for %s\n' "$run_url"
if ! gh run watch "$run_id" --exit-status --repo "$repo" >/dev/null; then
  die "could not confirm required CI success: $run_url"
fi

read -r observed_sha conclusion < <(
  gh run view "$run_id" --json headSha,conclusion \
    --jq '[.headSha,.conclusion] | @tsv' \
    --repo "$repo"
)
[[ $observed_sha == "$sha" && $conclusion == success ]] ||
  die "run completed as $conclusion for $observed_sha, expected success for $sha"

printf 'verify: required CI passed for %s (%s)\n' "$sha" "$run_url"
