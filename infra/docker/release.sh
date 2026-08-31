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
container_prefix=novel
qualification_project=
http_bind=
http_port=
health_origin=http://localhost
qualification_scope=false
compose_project_args=()
image_keys=(
  GATEWAY_IMAGE USER_SERVICE_IMAGE NOVEL_SERVICE_IMAGE AGENT_SERVICE_IMAGE
  NARRATIVE_SERVICE_IMAGE FRONTEND_IMAGE POSTGRES_IMAGE REDIS_IMAGE NGINX_IMAGE
)
schema_barrier_migrations=(
  infra/postgres/migrations/0021_world_turn_memory_projection.sql
  infra/postgres/migrations/0024_persona_provenance.sql
  infra/postgres/migrations/0025_chat_world_revision.sql
)
minimal_bootstrap_adr=docs/adr/0002-minimal-bootstrap-and-deferred-runtime-configuration.md

die() {
  printf 'release: %s\n' "$*" >&2
  exit 1
}

configure_qualification_scope() {
  local project=${RELEASE_COMPOSE_PROJECT:-}
  local prefix=${RELEASE_CONTAINER_PREFIX:-}
  local bind=${RELEASE_HTTP_BIND:-}
  local port=${RELEASE_HTTP_PORT:-}
  if [[ -z "$project$prefix$bind$port" ]]; then
    return
  fi
  [[ -n "$project" && -n "$prefix" && -n "$bind" && -n "$port" ]] \
    || die "qualification scope requires project, prefix, bind, and port together"
  [[ "$project" =~ ^nwq-[a-f0-9]{10}$ ]] \
    || die "qualification project must match nwq-<10 lowercase hex>"
  [[ "$prefix" == "$project" ]] \
    || die "qualification container prefix must equal its project"
  [[ "$bind" == 127.0.0.1 ]] \
    || die "qualification HTTP bind must be 127.0.0.1"
  [[ "$port" =~ ^[1-9][0-9]{3,4}$ ]] && ((10#$port >= 1024 && 10#$port <= 65535)) \
    || die "qualification HTTP port must be between 1024 and 65535"
  container_prefix=$prefix
  qualification_project=$project
  http_bind=$bind
  http_port=$port
  health_origin="http://$bind:$port"
  qualification_scope=true
  compose_project_args=(--project-name "$project")
}

configure_qualification_scope
readonly container_prefix qualification_project http_bind http_port health_origin qualification_scope
readonly -a compose_project_args

qualification_phase() {
  [[ "$qualification_scope" == true ]] || return 0
  printf 'qualification-phase %s %s %s\n' "$1" "$2" "$(date +%s%3N)"
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

secret_value() {
  local wanted=$1 key value count=0 found=
  [[ -r "$secrets_file" ]] || die "production secrets file not found: $secrets_file"
  while IFS='=' read -r key value || [[ -n "$key$value" ]]; do
    if [[ "$key" == "$wanted" ]]; then
      count=$((count + 1))
      found=$value
    fi
  done < "$secrets_file"
  [[ "$count" -le 1 ]] || die "duplicate $wanted in production secrets"
  [[ "$count" -eq 1 ]] || return 1
  printf '%s\n' "$found"
}

set_secret_value() {
  local key=$1 value=$2 count
  count=$(grep -c "^${key}=" "$secrets_file" || true)
  [[ "$count" -le 1 ]] || die "duplicate $key in production secrets"
  if [[ "$count" -eq 1 ]]; then
    sed -i "s|^${key}=.*$|${key}=${value}|" "$secrets_file"
  else
    printf '\n%s=%s\n' "$key" "$value" >> "$secrets_file"
  fi
}

valid_redis_password() {
  local value=$1 lowered distinct
  lowered=$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')
  distinct=$(printf '%s' "$value" | fold -w1 | sort -u | wc -l | tr -d ' ')
  [[ "$value" =~ ^[A-Za-z0-9._~-]{16,}$ ]] \
    && [[ "$distinct" -ge 8 ]] \
    && [[ "$lowered" != *placeholder* ]] \
    && [[ "$lowered" != *change_me* ]] \
    && [[ "$lowered" != your_redis_password_here ]] \
    && [[ "$lowered" != runtime-redis-only ]]
}

cache_mode=
cache_redis_password=
cache_redis_url=
compose_profile_args=()

load_cache_mode() {
  local mode_count
  [[ -r "$secrets_file" ]] || die "production secrets file not found: $secrets_file"
  mode_count=$(grep -c '^CACHE_MODE=' "$secrets_file" || true)
  [[ "$mode_count" -le 1 ]] || die "duplicate CACHE_MODE in production secrets"
  cache_redis_password=$(secret_value REDIS_PASSWORD || true)

  if [[ "$mode_count" -eq 0 ]]; then
    if [[ -n "$cache_redis_password" \
      && "$cache_redis_password" != your_redis_password_here ]]; then
      cache_mode=redis
    else
      cache_mode=postgres
      if [[ "$cache_redis_password" == your_redis_password_here ]]; then
        set_secret_value REDIS_PASSWORD ''
        cache_redis_password=
      fi
    fi
    set_secret_value CACHE_MODE "$cache_mode"
    chmod 600 "$secrets_file"
    sync
  else
    cache_mode=$(secret_value CACHE_MODE)
  fi

  compose_profile_args=()
  case "$cache_mode" in
    postgres)
      cache_redis_url=memory://
      ;;
    redis)
      valid_redis_password "$cache_redis_password" \
        || die "CACHE_MODE=redis requires a URL-safe, non-placeholder REDIS_PASSWORD of at least 16 characters with 8 distinct characters"
      cache_redis_url="redis://:${cache_redis_password}@redis:6379"
      compose_profile_args=(--profile redis)
      ;;
    *) die "CACHE_MODE must be exactly postgres or redis" ;;
  esac
  readonly cache_mode cache_redis_password cache_redis_url
  readonly -a compose_profile_args
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
  local from="$1" to="$2" migration contract
  for migration in "${schema_barrier_migrations[@]}"; do
    if manifest_contains_path "$from" "$migration" \
      && ! manifest_contains_path "$to" "$migration"; then
      case "$migration" in
        *0021_world_turn_memory_projection.sql) contract='world-memory projection' ;;
        *0024_persona_provenance.sql) contract='persona-provenance' ;;
        *0025_chat_world_revision.sql) contract='chat-world revision' ;;
      esac
      die "rollback target predates the $contract contract; use the separately approved database compatibility procedure"
    fi
  done
}

require_schema_barriers_present() {
  local manifest="$1" context="$2" migration contract
  for migration in "${schema_barrier_migrations[@]}"; do
    if ! manifest_contains_path "$manifest" "$migration"; then
      case "$migration" in
        *0021_world_turn_memory_projection.sql) contract='world-memory projection' ;;
        *0024_persona_provenance.sql) contract='persona-provenance' ;;
        *0025_chat_world_revision.sql) contract='chat-world revision' ;;
      esac
      die "$context predates the $contract contract"
    fi
  done
}

valid_llm_environment_override() {
  local value
  if [[ -n "${LLM_API_KEY+x}" ]]; then
    value=$LLM_API_KEY
  else
    value=$(secret_value LLM_API_KEY || true)
  fi
  [[ -n "$value" ]] \
    && [[ "$value" != sk-your-api-key ]] \
    && [[ "$value" != *placeholder* ]] \
    && [[ "$value" != change_me* ]]
}

current_user_service_proves_database_llm_configured() {
  docker exec "${container_prefix}-user-service" sh -ec '
    test -z "${LLM_API_KEY:-}" || exit 1
    code=$(curl --silent --show-error --output /dev/null --write-out "%{http_code}" \
      --max-time 15 -H "X-Internal-Service-Token: ${INTERNAL_SERVICE_TOKEN:?}" \
      http://127.0.0.1:8001/internal/runtime/llm)
    test "$code" = 200
  '
}

redis_accepts_persisted_password() {
  printf '%s\n' "$cache_redis_password" \
    | docker exec -i "${container_prefix}-redis" sh -ec '
        IFS= read -r persisted_password
        REDISCLI_AUTH="$persisted_password" exec redis-cli --no-auth-warning ping
      ' >/dev/null 2>&1
}

redis_runs_manifest_image() {
  local expected running
  expected=$(manifest_value "$active_manifest" REDIS_IMAGE)
  running=$(docker inspect --format '{{.Config.Image}}' \
    "${container_prefix}-redis" 2>/dev/null || true)
  [[ -n "$running" && "$running" == "$expected" ]]
}

require_pre_minimum_rollback_ready() {
  local from=$1 to=$2 redis_health
  if ! manifest_contains_path "$from" "$minimal_bootstrap_adr" \
    || manifest_contains_path "$to" "$minimal_bootstrap_adr"; then
    return 0
  fi

  load_cache_mode
  [[ "$cache_mode" == redis ]] \
    || die "rollback target predates minimal bootstrap and requires CACHE_MODE=redis"
  redis_health=$(docker inspect --format '{{.State.Health.Status}}' "${container_prefix}-redis" 2>/dev/null || true)
  [[ "$redis_health" == healthy ]] \
    || die "rollback target predates minimal bootstrap and requires healthy Redis"
  redis_accepts_persisted_password \
    || die "rollback target predates minimal bootstrap and requires the persisted Redis credential to authenticate"

  if valid_llm_environment_override; then
    return 0
  fi
  current_user_service_proves_database_llm_configured \
    || die "rollback target predates minimal bootstrap and current User Service did not prove a decryptable database LLM configuration"
}

active_manifest=
compose() (
  [[ -n "$cache_mode" ]] || die "cache mode was not initialized"
  export CACHE_MODE="$cache_mode"
  export REDIS_PASSWORD="$cache_redis_password"
  export REDIS_URL="$cache_redis_url"
  if [[ "$qualification_scope" == true ]]; then
    export CONTAINER_PREFIX="$container_prefix"
    export NGINX_HTTP_BIND="$http_bind"
    export NGINX_HTTP_PORT="$http_port"
  fi
  env \
    -u COMPOSE_PROFILES \
    -u RELEASE_VERSION -u RELEASE_GIT_SHA \
    -u GATEWAY_IMAGE -u USER_SERVICE_IMAGE -u NOVEL_SERVICE_IMAGE \
    -u AGENT_SERVICE_IMAGE -u NARRATIVE_SERVICE_IMAGE -u FRONTEND_IMAGE \
    -u POSTGRES_IMAGE -u REDIS_IMAGE -u NGINX_IMAGE \
    docker compose "${compose_project_args[@]}" \
      --project-directory "$repo_root" -f "$repo_root/docker-compose.yml" \
      --env-file "$secrets_file" --env-file "$active_manifest" \
      "${compose_profile_args[@]}" "$@"
)

require_empty_qualification_project() {
  local existing
  [[ "$qualification_scope" == true ]] \
    || die "cold adoption is restricted to an isolated qualification project"
  if ! existing=$(compose ps --all --quiet); then
    die "cannot inspect qualification Compose project"
  fi
  [[ -z "$existing" ]] \
    || die "qualification cold adopt requires an empty Compose project"
  if ! existing=$(docker volume ls --quiet \
    --filter "label=com.docker.compose.project=$qualification_project"); then
    die "cannot inspect qualification project volumes"
  fi
  [[ -z "$existing" ]] \
    || die "qualification cold adopt requires no existing project volumes"
  if ! existing=$(docker network ls --quiet \
    --filter "label=com.docker.compose.project=$qualification_project"); then
    die "cannot inspect qualification project networks"
  fi
  [[ -z "$existing" ]] \
    || die "qualification cold adopt requires no existing project networks"
}

deploy_initial_manifest() {
  local manifest="$1" release_sha initial_transition_tmp
  validate_manifest "$manifest"
  [[ -f "$secrets_file" ]] || die "production secrets file not found: $secrets_file"
  if [[ -z "$cache_mode" ]]; then
    load_cache_mode
  fi
  [[ "$cache_mode" == postgres ]] \
    || die "qualification cold adopt requires CACHE_MODE=postgres"
  active_manifest="$manifest"
  release_sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)

  require_clean_worktree
  git cat-file -e "$release_sha^{commit}"
  git checkout --detach "$release_sha"
  require_clean_worktree

  qualification_phase pull start
  compose pull \
    postgres-migrate nginx frontend user-service novel-service agent-service \
    narrative-service gateway # qualification cold pull
  qualification_phase pull end
  if [[ -f "$schema_transition_manifest" ]]; then
    manifests_equal "$schema_transition_manifest" "$manifest" \
      || die "initial schema transition does not match its recovery target"
  else
    initial_transition_tmp=$(mktemp "$state_dir/.initial-schema-transition.XXXXXX")
    install -m 600 "$manifest" "$initial_transition_tmp"
    command mv -f "$initial_transition_tmp" "$schema_transition_manifest" # qualification cold marker
    sync # qualification cold marker durable before first migration
  fi

  qualification_phase database_start start
  compose up -d --wait --wait-timeout 120 --no-build postgres # qualification cold postgres
  qualification_phase database_start end
  qualification_phase migration start
  compose run --rm --no-deps postgres-migrate # qualification cold migration
  qualification_phase migration end
  qualification_phase application_deployment start
  compose up -d --wait --wait-timeout 120 --no-build --no-deps \
    user-service novel-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps narrative-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps agent-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps \
    gateway frontend nginx # qualification cold applications
  qualification_phase application_deployment end
  qualification_phase readiness start
  curl --fail --silent --show-error "$health_origin/health" >/dev/null # qualification cold health
  curl --fail --silent --show-error "$health_origin/ready" >/dev/null
  qualification_phase readiness end
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
  if [[ -z "$cache_mode" ]]; then
    load_cache_mode
  fi
  active_manifest="$manifest"
  release_sha=$(manifest_value "$manifest" RELEASE_GIT_SHA)

  require_clean_worktree
  git cat-file -e "$release_sha^{commit}"
  git checkout --detach "$release_sha"
  require_clean_worktree

  qualification_phase pull start
  compose pull \
    postgres-migrate nginx frontend user-service novel-service agent-service \
    narrative-service gateway
  qualification_phase pull end
  [[ "$(docker inspect --format '{{.State.Health.Status}}' "${container_prefix}-postgres")" == healthy ]] \
    || die "PostgreSQL is not healthy"
  if [[ "$cache_mode" == redis ]]; then
    [[ "$(docker inspect --format '{{.State.Health.Status}}' "${container_prefix}-redis" 2>/dev/null || true)" == healthy ]] \
      || die "Redis is not healthy"
    redis_runs_manifest_image \
      || die "running Redis image does not match the release manifest"
    redis_accepts_persisted_password \
      || die "the persisted Redis credential cannot authenticate"
  else
    [[ "$(docker inspect --format '{{.State.Status}}' "${container_prefix}-redis" 2>/dev/null || true)" != running ]] \
      || die "Redis is running while CACHE_MODE=postgres"
  fi

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

  # The world-turn producer is already stopped. Stop the old public persona
  # reader and drain Agent before the 0021/0024/0025 semantic migrations. Once
  # all stops return, no old process can serve unprovenanced persona data or
  # create a chat claim without the exact world revision while these barriers
  # are crossed.
  compose stop --timeout 120 novel-service
  compose stop --timeout 120 agent-service
  transition_tmp=$(mktemp "$state_dir/.schema-transition.XXXXXX")
  install -m 600 "$manifest" "$transition_tmp"
  mv -f "$transition_tmp" "$schema_transition_manifest"
  # The migration cannot start until the exact recovery target is durable.
  # A host crash before this global flush leaves the old writer valid; after
  # it returns, every recovery path must honor the marker.
  sync # schema transition target durable before migration
  qualification_phase migration start
  compose run --rm --no-deps postgres-migrate # schema transition migration
  qualification_phase migration end
  qualification_phase application_deployment start
  compose up -d --wait --wait-timeout 120 --no-build --no-deps novel-service
  compose up -d --wait --wait-timeout 120 --no-build --no-deps \
    user-service narrative-service agent-service gateway nginx
  qualification_phase application_deployment end
  qualification_phase readiness start
  curl --fail --silent --show-error "$health_origin/health" >/dev/null
  curl --fail --silent --show-error "$health_origin/ready" >/dev/null
  qualification_phase readiness end
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
    require_schema_barriers_present "$candidate_tmp" "adopt target"
    mv -f "$candidate_tmp" "$candidate_manifest"
    release_sha=$(manifest_value "$candidate_manifest" RELEASE_GIT_SHA)
    trap - EXIT
    if [[ "$qualification_scope" == true ]]; then
      active_manifest="$candidate_manifest"
      load_cache_mode
      require_empty_qualification_project
      deploy_initial_manifest "$candidate_manifest"
    else
      printf 'Confirm the legacy source, exact images, and database backup can restore service, then enter ADOPT-%s: ' \
        "$release_sha" >&2
      IFS= read -r confirmation
      [[ "$confirmation" == "ADOPT-$release_sha" ]] \
        || die "manual recovery was not confirmed"
      deploy_manifest "$candidate_manifest" true
    fi
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
        require_schema_safe_rollback "$current_manifest" "$schema_transition_manifest"
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
        require_schema_barriers_present "$schema_transition_manifest" \
          "initial schema transition"
        if [[ "$qualification_scope" == true ]]; then
          deploy_initial_manifest "$schema_transition_manifest"
        else
          deploy_manifest "$schema_transition_manifest" false
        fi
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
    require_pre_minimum_rollback_ready "$current_manifest" "$previous_manifest"
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
