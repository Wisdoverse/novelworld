#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

state_dir=${RELEASE_STATE_DIR:-"$repo_root/.release"}
current_manifest="$state_dir/current.env"
previous_manifest="$state_dir/previous.env"
candidate_manifest="$state_dir/candidate.env"
rollback_marker="$state_dir/rollback.pending"
rollback_current="$state_dir/rollback-current.env"
rollback_previous="$state_dir/rollback-previous.env"
secrets_file="$repo_root/.env"
image_keys=(
  GATEWAY_IMAGE USER_SERVICE_IMAGE NOVEL_SERVICE_IMAGE AGENT_SERVICE_IMAGE
  NARRATIVE_SERVICE_IMAGE FRONTEND_IMAGE POSTGRES_IMAGE REDIS_IMAGE NGINX_IMAGE
)

die() {
  printf 'release: %s\n' "$*" >&2
  exit 1
}

acquire_release_lock() {
  install -d -m 700 "$state_dir"
  exec 9>"$state_dir/release.lock"
  flock -n 9 || die "another release operation is running"
  recover_pending_rollback
}

require_clean_worktree() {
  [[ -z "$(git status --porcelain=v1 --untracked-files=normal)" ]] \
    || die "working tree is not clean"
}

validate_manifest() {
  local file="$1" key value required
  local -A seen=()
  [[ -r "$file" ]] || die "manifest not readable: $file"

  while IFS='=' read -r key value || [[ -n "$key$value" ]]; do
    case "$key" in
      RELEASE_VERSION|RELEASE_GIT_SHA|GATEWAY_IMAGE|USER_SERVICE_IMAGE|NOVEL_SERVICE_IMAGE|AGENT_SERVICE_IMAGE|NARRATIVE_SERVICE_IMAGE|FRONTEND_IMAGE|POSTGRES_IMAGE|REDIS_IMAGE|NGINX_IMAGE) ;;
      *) die "unexpected manifest key: ${key:-<empty>}" ;;
    esac
    [[ -n "$value" ]] || die "empty manifest value: $key"
    [[ -z "${seen[$key]+present}" ]] || die "duplicate manifest key: $key"
    seen[$key]=1

    case "$key" in
      RELEASE_VERSION)
        [[ "$value" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] || die "invalid release version"
        ;;
      RELEASE_GIT_SHA)
        [[ "$value" =~ ^[0-9a-f]{40}$ ]] || die "invalid release git SHA"
        ;;
      *_IMAGE)
        [[ "$value" =~ ^[a-z0-9][a-z0-9._/:@-]*@sha256:[0-9a-f]{64}$ ]] \
          || die "image is not an immutable digest: $key"
        ;;
    esac
  done < "$file"

  for required in RELEASE_VERSION RELEASE_GIT_SHA "${image_keys[@]}"; do
    [[ -n "${seen[$required]+present}" ]] || die "missing manifest key: $required"
  done
  [[ ${#seen[@]} -eq 11 ]] || die "manifest must contain exactly 11 keys"
}

manifest_value() {
  local file="$1" wanted="$2" key value
  while IFS='=' read -r key value || [[ -n "$key$value" ]]; do
    if [[ "$key" == "$wanted" ]]; then
      printf '%s\n' "$value"
      return
    fi
  done < "$file"
  return 1
}

manifests_equal() {
  local left="$1" right="$2" key
  for key in RELEASE_VERSION RELEASE_GIT_SHA "${image_keys[@]}"; do
    [[ "$(manifest_value "$left" "$key")" == "$(manifest_value "$right" "$key")" ]] \
      || return 1
  done
}

require_same_infrastructure() {
  local from="$1" to="$2" key
  for key in POSTGRES_IMAGE REDIS_IMAGE NGINX_IMAGE; do
    [[ "$(manifest_value "$from" "$key")" == "$(manifest_value "$to" "$key")" ]] \
      || die "$key changed; use the separately approved infrastructure procedure"
  done
}

active_manifest=
compose() {
  env \
    -u RELEASE_VERSION -u RELEASE_GIT_SHA \
    -u GATEWAY_IMAGE -u USER_SERVICE_IMAGE -u NOVEL_SERVICE_IMAGE \
    -u AGENT_SERVICE_IMAGE -u NARRATIVE_SERVICE_IMAGE -u FRONTEND_IMAGE \
    -u POSTGRES_IMAGE -u REDIS_IMAGE -u NGINX_IMAGE \
    docker compose --project-directory "$repo_root" -f "$repo_root/docker-compose.yml" \
      --env-file "$secrets_file" --env-file "$active_manifest" "$@"
}

deploy_manifest() {
  local manifest="$1" require_client_gate="$2" release_sha confirmation
  validate_manifest "$manifest"
  [[ -f "$secrets_file" ]] || die "production secrets file not found: $secrets_file"
  active_manifest="$manifest"
  release_sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)

  require_clean_worktree
  git fetch --tags origin
  git cat-file -e "$release_sha^{commit}"
  git checkout --detach "$release_sha"
  require_clean_worktree

  compose pull \
    postgres-migrate nginx frontend user-service novel-service agent-service \
    narrative-service gateway
  [[ "$(docker inspect --format '{{.State.Health.Status}}' novel-postgres)" == healthy ]] \
    || die "PostgreSQL is not healthy"
  [[ "$(docker inspect --format '{{.State.Health.Status}}' novel-redis)" == healthy ]] \
    || die "Redis is not healthy"

  compose up -d --wait --wait-timeout 120 --no-build --no-deps --force-recreate \
    frontend nginx
  if [[ "$require_client_gate" == true ]]; then
    printf 'Verify PUT /api/progress/:novelId and UUID v4 Idempotency-Key, then enter %s: ' \
      "$release_sha" >&2
    IFS= read -r confirmation
    [[ "$confirmation" == "$release_sha" ]] || die "client contract gate was not confirmed"
  fi

  compose run --rm --no-deps postgres-migrate
  compose up -d --wait --wait-timeout 120 --no-build --no-deps novel-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps \
    user-service narrative-service agent-service gateway nginx
  curl --fail --silent --show-error http://localhost/health >/dev/null
  curl --fail --silent --show-error http://localhost/ready >/dev/null
}

promote_upgrade() {
  local previous_tmp current_tmp
  previous_tmp=$(mktemp "$state_dir/.previous.XXXXXX")
  current_tmp=$(mktemp "$state_dir/.current.XXXXXX")
  install -m 600 "$current_manifest" "$previous_tmp"
  install -m 600 "$candidate_manifest" "$current_tmp"
  mv -f "$previous_tmp" "$previous_manifest"
  mv -f "$current_tmp" "$current_manifest"
  rm -f "$candidate_manifest"
}

promote_rollback() {
  install -m 600 "$previous_manifest" "$rollback_current"
  install -m 600 "$current_manifest" "$rollback_previous"
  : > "$rollback_marker"
  recover_pending_rollback
}

recover_pending_rollback() {
  local current_tmp previous_tmp
  [[ -f "$rollback_marker" ]] || return 0
  validate_manifest "$rollback_current"
  validate_manifest "$rollback_previous"
  current_tmp=$(mktemp "$state_dir/.current.XXXXXX")
  previous_tmp=$(mktemp "$state_dir/.previous.XXXXXX")
  install -m 600 "$rollback_current" "$current_tmp"
  install -m 600 "$rollback_previous" "$previous_tmp"
  mv -f "$current_tmp" "$current_manifest"
  mv -f "$previous_tmp" "$previous_manifest"
  rm -f "$rollback_marker" "$rollback_current" "$rollback_previous"
}

command=${1:-}

case "$command" in
  adopt)
    [[ $# -eq 2 ]] || die "usage: $0 adopt /path/to/release.env"
    [[ -f "$2" ]] || die "candidate manifest must be a regular file"
    acquire_release_lock
    [[ ! -e "$current_manifest" && ! -e "$previous_manifest" ]] \
      || die "release state already exists; use upgrade"
    candidate_tmp=$(mktemp "$state_dir/.candidate.XXXXXX")
    trap 'rm -f "$candidate_tmp" "$candidate_manifest"' EXIT
    install -m 600 "$2" "$candidate_tmp"
    validate_manifest "$candidate_tmp"
    mv -f "$candidate_tmp" "$candidate_manifest"
    release_sha=$(manifest_value "$candidate_manifest" RELEASE_GIT_SHA)
    printf 'Confirm the legacy source, exact images, and database backup can restore service, then enter ADOPT-%s: ' \
      "$release_sha" >&2
    IFS= read -r confirmation
    [[ "$confirmation" == "ADOPT-$release_sha" ]] \
      || die "manual recovery was not confirmed"
    trap - EXIT
    deploy_manifest "$candidate_manifest" true
    mv -f "$candidate_manifest" "$current_manifest"
    ;;
  upgrade)
    [[ $# -eq 2 ]] || die "usage: $0 upgrade /path/to/release.env"
    [[ -f "$2" ]] || die "candidate manifest must be a regular file"
    acquire_release_lock
    candidate_tmp=$(mktemp "$state_dir/.candidate.XXXXXX")
    trap 'rm -f "$candidate_tmp"' EXIT
    install -m 600 "$2" "$candidate_tmp"
    validate_manifest "$current_manifest"
    validate_manifest "$candidate_tmp"
    require_same_infrastructure "$current_manifest" "$candidate_tmp"
    if [[ "$(manifest_value "$candidate_tmp" RELEASE_GIT_SHA)" == \
      "$(manifest_value "$current_manifest" RELEASE_GIT_SHA)" ]]; then
      manifests_equal "$current_manifest" "$candidate_tmp" \
        || die "current release SHA has a different manifest"
      printf 'release: already current\n'
      exit 0
    fi
    mv -f "$candidate_tmp" "$candidate_manifest"
    trap - EXIT
    deploy_manifest "$candidate_manifest" true
    promote_upgrade
    ;;
  restore)
    [[ $# -eq 1 ]] || die "usage: $0 restore"
    acquire_release_lock
    validate_manifest "$current_manifest"
    deploy_manifest "$current_manifest" false
    ;;
  rollback)
    [[ $# -eq 2 ]] || die "usage: $0 rollback RELEASE_GIT_SHA"
    [[ "$2" =~ ^[0-9a-f]{40}$ ]] || die "invalid rollback git SHA"
    acquire_release_lock
    validate_manifest "$current_manifest"
    if [[ "$(manifest_value "$current_manifest" RELEASE_GIT_SHA)" == "$2" ]]; then
      printf 'release: rollback target already current\n'
      exit 0
    fi
    validate_manifest "$previous_manifest"
    [[ "$(manifest_value "$previous_manifest" RELEASE_GIT_SHA)" == "$2" ]] \
      || die "previous release does not match rollback target"
    require_same_infrastructure "$current_manifest" "$previous_manifest"
    deploy_manifest "$previous_manifest" false
    promote_rollback
    ;;
  validate)
    [[ $# -eq 2 ]] || die "usage: $0 validate /path/to/release.env"
    validate_manifest "$2"
    ;;
  *)
    die "usage: $0 adopt /path/to/release.env | upgrade /path/to/release.env | restore | rollback RELEASE_GIT_SHA | validate /path/to/release.env"
    ;;
esac
