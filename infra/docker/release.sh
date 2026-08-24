#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

state_dir=${RELEASE_STATE_DIR:-"$repo_root/.release"}
current_manifest="$state_dir/current.env"
previous_manifest="$state_dir/previous.env"
candidate_manifest="$state_dir/candidate.env"
schema_transition_manifest="$state_dir/schema-transition.pending"
rollback_marker="$state_dir/rollback.pending"
rollback_current="$state_dir/rollback-current.env"
rollback_previous="$state_dir/rollback-previous.env"
secrets_file="$repo_root/.env"
image_keys=(
  GATEWAY_IMAGE USER_SERVICE_IMAGE NOVEL_SERVICE_IMAGE AGENT_SERVICE_IMAGE
  NARRATIVE_SERVICE_IMAGE FRONTEND_IMAGE POSTGRES_IMAGE REDIS_IMAGE NGINX_IMAGE
)
world_memory_projection_migration=infra/postgres/migrations/0021_world_turn_memory_projection.sql

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

manifest_contains_path() {
  local manifest="$1" path="$2" sha
  sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)
  git cat-file -e "$sha^{commit}" 2>/dev/null \
    || die "release commit is not available locally: $sha"
  git cat-file -e "$sha:$path" 2>/dev/null
}

fetch_release_history() {
  local manifest sha missing=false
  local -a shas=()
  for manifest in "$@"; do
    sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)
    shas+=("$sha")
    git cat-file -e "$sha^{commit}" 2>/dev/null || missing=true
  done
  if [[ "$missing" == true ]]; then
    git fetch --tags origin
  fi
  for sha in "${shas[@]}"; do
    git cat-file -e "$sha^{commit}" 2>/dev/null \
      || die "release commit is not available after fetching origin: $sha"
  done
}

require_schema_safe_rollback() {
  local from="$1" to="$2"
  if manifest_contains_path "$from" "$world_memory_projection_migration" \
    && ! manifest_contains_path "$to" "$world_memory_projection_migration"; then
    die "rollback target predates the world-memory projection contract; use the separately approved database compatibility procedure"
  fi
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

fail_stop_client() {
  local reason="$1" nginx_stopped=true frontend_stopped=true
  compose stop --timeout 120 nginx || nginx_stopped=false
  compose stop --timeout 120 frontend || frontend_stopped=false
  if [[ "$nginx_stopped" == true && "$frontend_stopped" == true ]]; then
    die "$reason; client was stopped (fail-stopped)"
  fi
  die "$reason; fail-stop incomplete, operator intervention is required"
}

recover_client_after_gate_failure() {
  local current_sha
  if [[ ! -f "$current_manifest" ]]; then
    fail_stop_client "client contract gate was not confirmed and no current release exists"
  fi

  current_sha=$(manifest_value "$current_manifest" RELEASE_GIT_SHA)
  if ! git cat-file -e "$current_sha^{commit}" 2>/dev/null \
    || ! git checkout --detach "$current_sha"; then
    fail_stop_client "client contract gate was not confirmed and current client checkout failed"
  fi
  active_manifest="$current_manifest"
  if ! compose up -d --wait --wait-timeout 120 --no-build --no-deps --force-recreate \
    narrative-service frontend nginx; then
    fail_stop_client "client contract gate was not confirmed and current client restore failed"
  fi
  die "client contract gate was not confirmed; current client was restored"
}

deploy_manifest() {
  local manifest="$1" require_client_gate="$2" release_sha confirmation transition_tmp
  validate_manifest "$manifest"
  [[ -f "$secrets_file" ]] || die "production secrets file not found: $secrets_file"
  active_manifest="$manifest"
  release_sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)

  require_clean_worktree
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

  # The candidate client and the previous Narrative API are not assumed to be
  # wire-compatible. Quiesce the world-turn producer before exposing candidate
  # assets so a submitted action receives a retryable 5xx and retains its exact
  # browser recovery key instead of being terminally rejected by the old DTO.
  compose stop --timeout 120 narrative-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps --force-recreate \
    frontend nginx
  if [[ "$require_client_gate" == true ]]; then
    printf 'Verify PUT /api/progress/:novelId and that a world action with expected_turn_number and UUID v4 Idempotency-Key is fail-stopped with 5xx, then enter %s: ' \
      "$release_sha" >&2
    IFS= read -r confirmation || confirmation=
    [[ "$confirmation" == "$release_sha" ]] || recover_client_after_gate_failure
  fi

  # The world-turn producer is already stopped. Drain its memory consumer
  # before migration 0021 installs the unresolved-turn journal and the new
  # Agent image activates the lossless legacy-prompt quarantine. Once both
  # stops return, every old write is either committed and covered by that
  # release boundary or absent; requests receive a temporary 5xx.
  compose stop --timeout 120 agent-service
  transition_tmp=$(mktemp "$state_dir/.schema-transition.XXXXXX")
  install -m 600 "$manifest" "$transition_tmp"
  mv -f "$transition_tmp" "$schema_transition_manifest"
  # The migration cannot start until the exact recovery target is durable.
  # A host crash before this global flush leaves the old writer valid; after
  # it returns, every recovery path must honor the marker.
  sync
  compose run --rm --no-deps postgres-migrate
  compose up -d --wait --wait-timeout 120 --no-build --no-deps novel-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps \
    user-service narrative-service agent-service gateway nginx
  curl --fail --silent --show-error http://localhost/health >/dev/null
  curl --fail --silent --show-error http://localhost/ready >/dev/null
}

validate_transition_candidate() {
  [[ -f "$candidate_manifest" ]] || return 0
  validate_manifest "$candidate_manifest"
  manifests_equal "$schema_transition_manifest" "$candidate_manifest" \
    || die "candidate release does not match the schema transition"
}

promote_schema_transition() {
  local current_tmp previous_tmp
  current_tmp=$(mktemp "$state_dir/.current.XXXXXX")
  install -m 600 "$schema_transition_manifest" "$current_tmp"
  if [[ -f "$current_manifest" ]]; then
    previous_tmp=$(mktemp "$state_dir/.previous.XXXXXX")
    install -m 600 "$current_manifest" "$previous_tmp"
    mv -f "$previous_tmp" "$previous_manifest"
  fi
  # Persist the target tempfile before its rename. On upgrade this also makes
  # previous durable first: the marker can reconstruct current, but it cannot
  # reconstruct the prior release after current has been replaced.
  sync # promotion inputs must be durable before current promotion
  mv -f "$current_tmp" "$current_manifest"
  rm -f "$candidate_manifest"
}

finalize_schema_transition() {
  # Persist current/previous promotion and candidate removal before deleting
  # the sole recovery authority. If either sync fails, set -e leaves the
  # marker in place. A crash between marker removal and the second sync sees
  # either the durable marker or the already-durable promoted pair.
  sync
  rm -f "$schema_transition_manifest"
  sync
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
  sync # rollback replacement tempfiles durable before rename
  mv -f "$current_tmp" "$current_manifest"
  mv -f "$previous_tmp" "$previous_manifest"
  sync # rollback manifest pair durable before marker removal
  rm -f "$rollback_marker"
  sync # rollback marker removal durable before staged cleanup
  rm -f "$rollback_current" "$rollback_previous"
  sync # rollback staged cleanup durable
}

command=${1:-}

case "$command" in
  adopt)
    [[ $# -eq 2 ]] || die "usage: $0 adopt /path/to/release.env"
    [[ -f "$2" ]] || die "candidate manifest must be a regular file"
    acquire_release_lock
    [[ ! -e "$current_manifest" && ! -e "$previous_manifest" \
      && ! -e "$schema_transition_manifest" ]] \
      || die "release state already exists; use upgrade"
    candidate_tmp=$(mktemp "$state_dir/.candidate.XXXXXX")
    trap 'rm -f "$candidate_tmp" "$candidate_manifest"' EXIT
    install -m 600 "$2" "$candidate_tmp"
    validate_manifest "$candidate_tmp"
    fetch_release_history "$candidate_tmp"
    manifest_contains_path "$candidate_tmp" "$world_memory_projection_migration" \
      || die "adopt target predates the world-memory projection contract"
    mv -f "$candidate_tmp" "$candidate_manifest"
    release_sha=$(manifest_value "$candidate_manifest" RELEASE_GIT_SHA)
    printf 'Confirm the legacy source, exact images, and database backup can restore service, then enter ADOPT-%s: ' \
      "$release_sha" >&2
    IFS= read -r confirmation
    [[ "$confirmation" == "ADOPT-$release_sha" ]] \
      || die "manual recovery was not confirmed"
    trap - EXIT
    deploy_manifest "$candidate_manifest" true
    promote_schema_transition
    finalize_schema_transition
    ;;
  upgrade)
    [[ $# -eq 2 ]] || die "usage: $0 upgrade /path/to/release.env"
    [[ -f "$2" ]] || die "candidate manifest must be a regular file"
    acquire_release_lock
    [[ ! -e "$schema_transition_manifest" ]] \
      || die "schema transition is pending; use restore"
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
    fetch_release_history "$current_manifest" "$candidate_tmp"
    require_schema_safe_rollback "$current_manifest" "$candidate_tmp"
    mv -f "$candidate_tmp" "$candidate_manifest"
    trap - EXIT
    deploy_manifest "$candidate_manifest" true
    promote_schema_transition
    finalize_schema_transition
    ;;
  restore)
    [[ $# -eq 1 ]] || die "usage: $0 restore"
    acquire_release_lock
    if [[ -f "$schema_transition_manifest" ]]; then
      # The marker is written only after the human client gate and durably
      # flushed before migration. Always roll that exact transition forward;
      # choosing current by schema compatibility can tear the release pair or
      # revive a writer after its migration already committed.
      validate_manifest "$schema_transition_manifest"
      validate_transition_candidate
      if [[ -f "$current_manifest" ]]; then
        validate_manifest "$current_manifest"
        fetch_release_history "$current_manifest" "$schema_transition_manifest"
        require_same_infrastructure "$current_manifest" "$schema_transition_manifest"
        deploy_manifest "$schema_transition_manifest" false
        if manifests_equal "$current_manifest" "$schema_transition_manifest"; then
          # Promotion already reached current before the crash. Preserve its
          # previous release and discard only the matching staged candidate.
          rm -f "$candidate_manifest"
        else
          promote_schema_transition
        fi
      else
        [[ ! -e "$previous_manifest" ]] \
          || die "initial schema transition cannot coexist with a previous release"
        fetch_release_history "$schema_transition_manifest"
        manifest_contains_path "$schema_transition_manifest" \
          "$world_memory_projection_migration" \
          || die "initial schema transition predates the world-memory projection contract"
        deploy_manifest "$schema_transition_manifest" false
        promote_schema_transition
      fi
      finalize_schema_transition # marked restore
    else
      validate_manifest "$current_manifest"
      fetch_release_history "$current_manifest"
      # A rejected client gate may leave an unmarked staged candidate. It is
      # not recovery authority and must not become a mismatched companion to
      # the exact marker that deploy_manifest is about to persist.
      rm -f "$candidate_manifest" # discard unmarked stale candidate before restore
      deploy_manifest "$current_manifest" false
      finalize_schema_transition # current restore
    fi
    ;;
  rollback)
    [[ $# -eq 2 ]] || die "usage: $0 rollback RELEASE_GIT_SHA"
    [[ "$2" =~ ^[0-9a-f]{40}$ ]] || die "invalid rollback git SHA"
    acquire_release_lock
    validate_manifest "$current_manifest"
    [[ ! -e "$schema_transition_manifest" ]] \
      || die "schema transition is pending; use restore"
    if [[ "$(manifest_value "$current_manifest" RELEASE_GIT_SHA)" == "$2" ]]; then
      printf 'release: rollback target already current\n'
      exit 0
    fi
    validate_manifest "$previous_manifest"
    [[ "$(manifest_value "$previous_manifest" RELEASE_GIT_SHA)" == "$2" ]] \
      || die "previous release does not match rollback target"
    require_same_infrastructure "$current_manifest" "$previous_manifest"
    fetch_release_history "$current_manifest" "$previous_manifest"
    require_schema_safe_rollback "$current_manifest" "$previous_manifest"
    rm -f "$candidate_manifest" # discard unmarked stale candidate before rollback
    deploy_manifest "$previous_manifest" false
    # deploy_manifest made the rollback target the exact durable schema marker;
    # reuse the same proven current/previous promotion protocol as upgrade.
    promote_schema_transition # rollback
    finalize_schema_transition # rollback
    ;;
  validate)
    [[ $# -eq 2 ]] || die "usage: $0 validate /path/to/release.env"
    validate_manifest "$2"
    ;;
  *)
    die "usage: $0 adopt /path/to/release.env | upgrade /path/to/release.env | restore | rollback RELEASE_GIT_SHA | validate /path/to/release.env"
    ;;
esac
