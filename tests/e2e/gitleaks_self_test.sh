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
      --config .gitleaks.toml --source "$1"
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
      ghcr.io/gitleaks/gitleaks:v8.24.3 detect --no-banner \
      --exit-code 42 --config /repo/.gitleaks.toml --source "$src"
  }
fi

# The planted token is generated at runtime: a committed ghp_-shaped literal
# would trip the repository's own delta scan in CI.
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

if ! scan "$PWD" >"$GITLEAKS_WORK/repo.out" 2>&1; then
  printf 'self-test: FAIL the repository is not clean under the committed config\n' >&2
  cat "$GITLEAKS_WORK/repo.out" >&2
  exit 1
fi
printf 'self-test: ok   the repository is clean under the committed config\n'
