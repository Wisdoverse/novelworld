#!/usr/bin/env bash
# Secret-scanning gate self-test. A gitleaks config that loses its rules
# scans nothing and passes silently, so this test pins the gate's strength:
# a planted GitHub-shaped token must FAIL the committed .gitleaks.toml scan,
# and the repository itself must stay clean under the same config.
#
# Uses the local .tools-bin/gitleaks binary when present, otherwise the
# pinned gitleaks docker image (CI path).
set -euo pipefail
cd "$(dirname "$0")/../.."

GITLEAKS_WORK=$(mktemp -d)
trap 'rm -rf "$GITLEAKS_WORK"' EXIT
export GITLEAKS_WORK

if [ -x .tools-bin/gitleaks ]; then
  scan() {
    .tools-bin/gitleaks detect --no-banner --exit-code 42 \
      --config .gitleaks.toml --log-opts HEAD --source "$1"
  }
else
  repo_mount=$PWD
  work_mount=$GITLEAKS_WORK
  if command -v cygpath >/dev/null 2>&1; then
    repo_mount=$(cygpath -m "$PWD")
    work_mount=$(cygpath -m "$GITLEAKS_WORK")
    export MSYS_NO_PATHCONV=1
  fi
  scan() {
    case "$1" in
      "$GITLEAKS_WORK"/*) src="/plant/${1#"$GITLEAKS_WORK"/}" ;;
      *) src="/repo" ;;
    esac
    docker run --rm -v "$repo_mount:/repo" -v "$work_mount:/plant" \
      ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f detect --no-banner \
      --exit-code 42 --config /repo/.gitleaks.toml --log-opts HEAD --source "$src"
  }
fi

# The planted token is generated at runtime: a committed ghp_-shaped literal
# would trip the repository's current-history scan in CI.
plant_token="ghp_$(python3 -c 'import secrets,string; print("".join(secrets.choice(string.ascii_letters + string.digits) for _ in range(36)))')"
plant="$GITLEAKS_WORK/plant"
mkdir -p "$plant"
(
  cd "$plant"
  git init -q
  git -c user.email=a@b -c user.name=a commit -q --allow-empty -m init
  printf '%s\n' "$plant_token" >leak.txt
  git add leak.txt
  git -c user.email=a@b -c user.name=a commit -qm plant
)

set +e
scan "$plant" >"$GITLEAKS_WORK/plant.out" 2>&1
plant_status=$?
set -e
if [ "$plant_status" -eq 0 ]; then
  printf 'self-test: FAIL a planted GitHub token was not detected\n' >&2
  cat "$GITLEAKS_WORK/plant.out" >&2
  exit 1
fi
if [ "$plant_status" -ne 42 ]; then
  printf 'self-test: FAIL the planted-token scan failed operationally (exit %s)\n' \
    "$plant_status" >&2
  cat "$GITLEAKS_WORK/plant.out" >&2
  exit 1
fi
printf 'self-test: ok   a planted GitHub token fails the gate\n'

# A full-depth checkout contains remote refs that are not part of the change
# under test. Keep the planted leak reachable from the original branch, then
# prove that a clean HEAD is not poisoned by that unrelated ref.
(
  cd "$plant"
  git checkout -qb clean HEAD~1
)
if ! scan "$plant" >"$GITLEAKS_WORK/unrelated-ref.out" 2>&1; then
  printf 'self-test: FAIL an unrelated ref poisoned the clean HEAD scan\n' >&2
  cat "$GITLEAKS_WORK/unrelated-ref.out" >&2
  exit 1
fi
printf 'self-test: ok   unrelated refs do not poison the clean HEAD scan\n'

if ! scan "$PWD" >"$GITLEAKS_WORK/repo.out" 2>&1; then
  printf 'self-test: FAIL the repository is not clean under the committed config\n' >&2
  cat "$GITLEAKS_WORK/repo.out" >&2
  exit 1
fi
printf 'self-test: ok   the repository is clean under the committed config\n'
