#!/usr/bin/env bash
# Real scanner controls complement the completion fault-injection checks.
set -euo pipefail
cd "$(dirname "$0")/../.."
umask 077
python3 tests/e2e/secret_scan_test.py

GITLEAKS_WORK=$(mktemp -d)
trap 'rm -rf "$GITLEAKS_WORK"' EXIT
export GITLEAKS_WORK

scan() { bash tools/scan-secrets.sh "$1"; }
expect_scan() {
  local expected=$1 source=$2 label=$3 observed=0
  scan "$source" >"$GITLEAKS_WORK/scan.out" 2>&1 || observed=$?
  if [ "$observed" -ne "$expected" ]; then
    printf 'self-test: FAIL %s (expected %s, got %s)\n' "$label" "$expected" "$observed" >&2
    exit 1
  fi
  if grep -Fq -e "$plant_token" -e "$provider_token" "$GITLEAKS_WORK/scan.out"; then
    printf 'self-test: FAIL scanner output exposed a planted value\n' >&2
    exit 1
  fi
  printf 'self-test: ok   %s\n' "$label"
}

# The planted token is generated at runtime: a committed ghp_-shaped literal
# would trip the repository's current-history scan in CI.
plant_token="ghp_$(python3 -c 'import secrets,string; print("".join(secrets.choice(string.ascii_letters + string.digits) for _ in range(36)))')"
# Generic-key detection applies entropy and stopword filters. Use a known
# positive, constructed at runtime, rather than a randomly filtered sample.
provider_token="sk-$(python3 -c 'print("".join(format(i, "x") for i in range(15, -1, -1)) * 2)')"
plant="$GITLEAKS_WORK/plant"
mkdir -p "$plant"
(
  cd "$plant"
  git init -q
  printf 'clean fixture\n' >README.md
  git add README.md
  git -c user.email=a@b -c user.name=a commit -qm init
  printf '%s\n' "$plant_token" >leak.txt
  git add leak.txt
  git -c user.email=a@b -c user.name=a commit -qm plant
)

expect_scan 42 "$plant" 'a planted GitHub token fails with redacted output'

# A full-depth checkout contains remote refs that are not part of the change
# under test. Keep the planted leak reachable from the original branch, then
# prove that a clean HEAD is not poisoned by that unrelated ref.
(
  cd "$plant"
  git checkout -qb clean HEAD~1
)
expect_scan 0 "$plant" 'unrelated refs do not poison the clean HEAD scan'

linked="$GITLEAKS_WORK/linked worktree"
git -C "$plant" worktree add --quiet -b linked-control "$linked" HEAD
expect_scan 0 "$linked" 'a clean linked worktree has readable history'
(
  cd "$linked"
  printf 'LLM_API_KEY=%s\n' "$provider_token" >provider.env
  git add provider.env
  git -c user.email=a@b -c user.name=a commit -qm provider-fixture
)
expect_scan 42 "$linked" 'a planted DeepSeek-shaped key fails with redacted output'

corrupt="$GITLEAKS_WORK/corrupt"
git init --quiet "$corrupt"
printf 'clean content\n' >"$corrupt/README.md"
git -C "$corrupt" add README.md
git -C "$corrupt" -c user.email=a@b -c user.name=a commit -qm fixture
# HEAD resolves, but its content cannot be scanned. Remove only a generated
# fixture blob, which is always loose in this fresh repository.
blob=$(git -C "$corrupt" rev-parse HEAD:README.md)
rm -- "$corrupt/.git/objects/${blob:0:2}/${blob:2}"
expect_scan 1 "$corrupt" 'unreadable history fails operationally'

expect_scan 0 "$PWD" 'the repository is clean under the committed config'
