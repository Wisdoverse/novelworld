#!/usr/bin/env bash
# Release/rollback state-machine drill (H2 'compromised dependency/artifact'
# scope): proves, without a registry, that infra/docker/release.sh fails
# closed on every guard that runs before deployment. The image-level
# deployment itself (deploy_manifest: git checkout + compose pull of
# digest-pinned images) stays registry/CI-gated and is NOT exercised here;
# SECURITY.md 'Release Rollback' records that boundary.
#
# Runs standalone: no stack, no network, no .release pollution (each case
# uses a fresh RELEASE_STATE_DIR).
set -euo pipefail
cd "$(dirname "$0")/../.."

release=infra/docker/release.sh
sha_a=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
sha_b=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

write_manifest() {
  local file=$1 sha=$2
  {
    printf 'RELEASE_VERSION=v1.0.0\n'
    printf 'RELEASE_GIT_SHA=%s\n' "$sha"
    for key in GATEWAY_IMAGE USER_SERVICE_IMAGE NOVEL_SERVICE_IMAGE \
      AGENT_SERVICE_IMAGE NARRATIVE_SERVICE_IMAGE FRONTEND_IMAGE \
      POSTGRES_IMAGE REDIS_IMAGE NGINX_IMAGE; do
      printf '%s=registry.example/novel@sha256:%064d\n' "$key" 0
    done
  } >"$file"
}

new_state() {
  local state
  state=$(mktemp -d "$work/state.XXXXXX")
  printf '%s\n' "$state"
}

expect_ok() {
  local label=$1 want=$2 status=0 output
  shift 2
  set +e
  output=$("$@" 2>&1)
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    printf 'drill: FAIL %s: expected success, got exit %s:\n%s\n' "$label" "$status" "$output" >&2
    exit 1
  fi
  if [ -n "$want" ] && ! grep -Fq -- "$want" <<<"$output"; then
    printf 'drill: FAIL %s: expected [%s] in output, got:\n%s\n' "$label" "$want" "$output" >&2
    exit 1
  fi
  printf 'drill: ok   %s\n' "$label"
}

expect_fail() {
  local label=$1 want=$2 status=0 output
  shift 2
  set +e
  output=$("$@" 2>&1)
  status=$?
  set -e
  if [ "$status" -eq 0 ]; then
    printf 'drill: FAIL %s: expected failure, got exit 0:\n%s\n' "$label" "$output" >&2
    exit 1
  fi
  if ! grep -Fq -- "$want" <<<"$output"; then
    printf 'drill: FAIL %s: expected [%s] in output, got:\n%s\n' "$label" "$want" "$output" >&2
    exit 1
  fi
  printf 'drill: ok   %s (exit %s)\n' "$label" "$status"
}

# --- validate: the manifest grammar fails closed ---
good=$(mktemp "$work/good.XXXXXX")
write_manifest "$good" "$sha_a"
expect_ok 'validate accepts a well-formed digest-pinned manifest' '' \
  "$release" validate "$good"

bad=$(mktemp "$work/bad.XXXXXX")
write_manifest "$bad" "$sha_a"
sed -i 's|^GATEWAY_IMAGE=.*|GATEWAY_IMAGE=registry.example/novel:latest|' "$bad"
expect_fail 'validate rejects a non-digest image' 'image is not an immutable digest' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
sed -i 's|^RELEASE_GIT_SHA=.*|RELEASE_GIT_SHA=short|' "$bad"
expect_fail 'validate rejects a malformed git SHA' 'invalid release git SHA' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
sed -i 's|^RELEASE_VERSION=.*|RELEASE_VERSION=.bad|' "$bad"
expect_fail 'validate rejects a malformed version' 'invalid release version' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
printf 'SURPRISE_KEY=value\n' >>"$bad"
expect_fail 'validate rejects an unknown key' 'unexpected manifest key: SURPRISE_KEY' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
printf 'RELEASE_GIT_SHA=%s\n' "$sha_b" >>"$bad"
expect_fail 'validate rejects a duplicate key' 'duplicate manifest key: RELEASE_GIT_SHA' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
printf 'GATEWAY_IMAGE=\n' >>"$bad"
expect_fail 'validate rejects an empty value' 'empty manifest value: GATEWAY_IMAGE' \
  "$release" validate "$bad"

write_manifest "$bad" "$sha_a"
sed -i '/^NOVEL_SERVICE_IMAGE=/d' "$bad"
expect_fail 'validate rejects a missing key' 'missing manifest key: NOVEL_SERVICE_IMAGE' \
  "$release" validate "$bad"

# --- upgrade pre-flights: every guard fires before any network access ---
state=$(new_state)
current=$(mktemp "$work/current.XXXXXX")
candidate=$(mktemp "$work/candidate.XXXXXX")
write_manifest "$current" "$sha_a"
write_manifest "$candidate" "$sha_a"
cp "$current" "$state/current.env"
RELEASE_STATE_DIR=$state expect_ok 'upgrade reports the manifest is already current' 'release: already current' \
  "$release" upgrade "$candidate"

write_manifest "$candidate" "$sha_a"
sed -i 's|^GATEWAY_IMAGE=.*|GATEWAY_IMAGE=registry.example/novel@sha256:1111111111111111111111111111111111111111111111111111111111111111|' "$candidate"
RELEASE_STATE_DIR=$state expect_fail 'upgrade rejects a divergent manifest for the current SHA' \
  'current release SHA has a different manifest' "$release" upgrade "$candidate"

write_manifest "$candidate" "$sha_b"
sed -i 's|^POSTGRES_IMAGE=.*|POSTGRES_IMAGE=registry.example/postgres@sha256:2222222222222222222222222222222222222222222222222222222222222222|' "$candidate"
RELEASE_STATE_DIR=$state expect_fail 'upgrade rejects infrastructure changes' \
  'POSTGRES_IMAGE changed; use the separately approved infrastructure procedure' \
  "$release" upgrade "$candidate"

# --- rollback pre-flights ---
RELEASE_STATE_DIR=$state expect_fail 'rollback rejects a malformed SHA argument' 'invalid rollback git SHA' \
  "$release" rollback not-a-sha
RELEASE_STATE_DIR=$state expect_ok 'rollback reports the target is already current' \
  'release: rollback target already current' "$release" rollback "$sha_a"

state2=$(new_state)
cp "$current" "$state2/current.env"
RELEASE_STATE_DIR=$state2 expect_fail 'rollback fails when no previous release exists' 'manifest not readable' \
  "$release" rollback "$sha_b"

previous=$(mktemp "$work/previous.XXXXXX")
write_manifest "$previous" "$sha_a"
cp "$previous" "$state2/previous.env"
RELEASE_STATE_DIR=$state2 expect_fail 'rollback rejects a previous SHA mismatch' \
  'previous release does not match rollback target' "$release" rollback "$sha_b"

write_manifest "$previous" "$sha_b"
sed -i 's|^NGINX_IMAGE=.*|NGINX_IMAGE=registry.example/nginx@sha256:3333333333333333333333333333333333333333333333333333333333333333|' "$previous"
cp "$previous" "$state2/previous.env"
RELEASE_STATE_DIR=$state2 expect_fail 'rollback rejects infrastructure changes' \
  'NGINX_IMAGE changed; use the separately approved infrastructure procedure' \
  "$release" rollback "$sha_b"

# --- interrupted-rollback recovery ---
state3=$(new_state)
write_manifest "$current" "$sha_a"
write_manifest "$previous" "$sha_b"
cp "$current" "$state3/rollback-current.env"
cp "$previous" "$state3/rollback-previous.env"
: >"$state3/rollback.pending"
RELEASE_STATE_DIR=$state3 expect_ok 'an interrupted rollback recovers before the command' \
  'release: rollback target already current' "$release" rollback "$sha_a"
test ! -e "$state3/rollback.pending" || { printf 'drill: FAIL pending marker survived recovery\n' >&2; exit 1; }
test ! -e "$state3/rollback-current.env" || { printf 'drill: FAIL rollback-current survived recovery\n' >&2; exit 1; }
test ! -e "$state3/rollback-previous.env" || { printf 'drill: FAIL rollback-previous survived recovery\n' >&2; exit 1; }
cmp -s "$current" "$state3/current.env" || { printf 'drill: FAIL recovered current mismatch\n' >&2; exit 1; }
cmp -s "$previous" "$state3/previous.env" || { printf 'drill: FAIL recovered previous mismatch\n' >&2; exit 1; }
printf 'drill: ok   recovered current/previous matches the rollback pair\n'

state4=$(new_state)
: >"$state4/rollback.pending"
RELEASE_STATE_DIR=$state4 expect_fail 'a wedged recovery marker fails closed' 'manifest not readable' \
  "$release" rollback "$sha_a"
test -e "$state4/rollback.pending" || { printf 'drill: FAIL wedged marker did not survive\n' >&2; exit 1; }
printf 'drill: ok   wedged marker survives for operator clearing\n'

# --- lock exclusion and adopt/restore guards ---
state5=$(new_state)
cp "$current" "$state5/current.env"
RELEASE_STATE_DIR=$state5 expect_fail 'adopt refuses when release state exists' \
  'release state already exists; use upgrade' "$release" adopt "$candidate"

state6=$(new_state)
RELEASE_STATE_DIR=$state6 expect_fail 'restore refuses without a current manifest' 'manifest not readable' \
  "$release" restore

state7=$(new_state)
cp "$current" "$state7/current.env"
(
  exec 9>"$state7/release.lock"
  flock -n 9
  : >"$state7/lock-held"
  sleep 5
) &
holder=$!
for _ in $(seq 1 20); do
  [ -e "$state7/lock-held" ] && break
  sleep 0.1
done
test -e "$state7/lock-held" || { printf 'drill: FAIL lock holder never acquired the lock\n' >&2; exit 1; }
RELEASE_STATE_DIR=$state7 expect_fail 'a held release lock excludes concurrent operations' \
  'another release operation is running' "$release" restore
wait "$holder"

printf 'drill: release state-machine drill passed\n'
