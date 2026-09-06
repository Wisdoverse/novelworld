#!/usr/bin/env bash
# One HEAD-history gate for CI and local worktrees. Exit 42 means detected
# credentials; 1 means the scan could not establish a complete result.
set -euo pipefail
umask 077
root=$(cd "$(dirname "$0")/.." && pwd -P)
fail() { printf 'secret-scan: %s\n' "$1" >&2; exit 1; }
[ "$#" -le 1 ] || fail 'expected at most one checkout path'
source_dir=$(cd "${1:-$root}" 2>/dev/null && pwd -P) || fail 'checkout unavailable'
# An inherited Git context must not silently select another history.
unset GIT_DIR GIT_COMMON_DIR GIT_WORK_TREE GIT_INDEX_FILE
source_git() { git -c safe.directory="$source_dir" -C "$source_dir" "$@"; }
source_git rev-parse --verify 'HEAD^{commit}' >/dev/null 2>&1 || fail 'HEAD unavailable'
shallow=$(source_git rev-parse --is-shallow-repository 2>/dev/null) || fail 'history unavailable'
[ "$shallow" = false ] || fail 'full history required; fetch without a depth limit'
git_dir=$(source_git rev-parse --absolute-git-dir 2>/dev/null) || fail 'Git metadata unavailable'
common_dir=$(source_git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || fail 'Git metadata unavailable'
git_dir=$(cd "$git_dir" 2>/dev/null && pwd -P) || fail 'Git metadata unavailable'
common_dir=$(cd "$common_dir" 2>/dev/null && pwd -P) || fail 'Git metadata unavailable'

scan_log=$(mktemp)
trap 'rm -f -- "$scan_log"' EXIT
flags=(detect --no-banner --no-color --redact=100 --log-level debug
       --timeout 300 --exit-code 42 --log-opts HEAD)
config="$root/.gitleaks.toml"
# Git's refs backend resolves commondir relative to GIT_DIR, even with
# GIT_COMMON_DIR set. Preserve that hierarchy for branch-backed worktrees.
case "$git_dir" in
  "$common_dir") container_git=/git ;;
  "$common_dir"/*) container_git="/git/${git_dir#"$common_dir"/}" ;;
  *) fail 'unsupported Git metadata layout' ;;
esac
if command -v cygpath >/dev/null 2>&1; then
  source_dir=$(cygpath -m "$source_dir")
  common_dir=$(cygpath -m "$common_dir")
  config=$(cygpath -m "$config")
  export MSYS_NO_PATHCONV=1
fi
scanner=(docker run --rm
  -v "$source_dir:/repo:ro" -v "$common_dir:/git:ro"
  -v "$config:/policy.toml:ro"
  -e "GIT_DIR=$container_git" -e GIT_COMMON_DIR=/git -e GIT_WORK_TREE=/repo
  -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory
  -e GIT_CONFIG_VALUE_0=/repo
  ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f
  "${flags[@]}" --config /policy.toml --source /repo)
status=0
"${scanner[@]}" >"$scan_log" 2>&1 || status=$?
[ "$status" -eq 0 ] || [ "$status" -eq 42 ] || fail 'scanner failed operationally'
# The pinned scanner can log a Git Wait failure only at DEBUG and still exit
# zero. Accept its completion evidence as well as its status; never echo raw
# diagnostics (even metadata/path/config errors can contain sensitive values).
commits=$(awk -v status="$status" '
  / (ERR|FTL|PNC) / || /command aborted|partial scan/ { bad = 1 }
  / INF [1-9][0-9]* commits scanned\.$/ { counts++; count = $(NF-2) }
  / INF no leaks found$/ { clean++ }
  / WRN leaks found: [1-9][0-9]*$/ { leaks++ }
  END {
    if (bad || counts != 1 ||
        (status == 0 && (clean != 1 || leaks != 0)) ||
        (status == 42 && (leaks != 1 || clean != 0))) exit 1
    print count
  }
' "$scan_log") || fail 'scan incomplete; verify Git history, scanner and configuration'
if [ "$status" -eq 42 ]; then
  printf 'secret-scan: credentials detected in %s scanned commits (values withheld)\n' "$commits" >&2
  exit 42
fi
printf 'secret-scan: %s commits scanned; no leaks found\n' "$commits"
