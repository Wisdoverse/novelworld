#!/usr/bin/env bash
# Release/rollback state-machine drill (H2 'compromised dependency/artifact'
# scope): proves, without a registry, that infra/docker/release.sh fails
# closed on every guard that runs before deployment. The image-level
# deployment itself (deploy_manifest: git checkout + compose pull of
# digest-pinned images) stays registry/CI-gated and is NOT exercised here;
# SECURITY.md 'Release Rollback' records that boundary.
#
# Runs standalone: no stack, no external network, no .release pollution (each
# case uses a fresh RELEASE_STATE_DIR and the fetch proof uses a local origin).
set -euo pipefail
cd "$(dirname "$0")/../.."

release=$(pwd)/infra/docker/release.sh
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

# Static ordering is part of the migration safety contract even though this
# no-registry drill deliberately does not execute deploy_manifest.
narrative_quiesce_line=$(grep -nF 'compose stop --timeout 120 narrative-service' "$release" | cut -d: -f1)
candidate_client_line=$(grep -nF '    frontend nginx' "$release" | tail -n 1 | cut -d: -f1)
gate_fail_stop_line=$(grep -nF 'is fail-stopped with 5xx' "$release" | cut -d: -f1)
agent_quiesce_line=$(grep -nF 'compose stop --timeout 120 agent-service' "$release" | cut -d: -f1)
schema_transition_line=$(grep -nF 'mv -f "$transition_tmp" "$schema_transition_manifest"' "$release" | cut -d: -f1)
marker_sync_line=$(grep -nE '^  sync$' "$release" | head -n 1 | cut -d: -f1)
migrate_line=$(grep -nF 'compose run --rm --no-deps postgres-migrate' "$release" | cut -d: -f1)
upgrade_guard_line=$(grep -nF 'require_schema_safe_rollback "$current_manifest" "$candidate_tmp"' "$release" | cut -d: -f1)
upgrade_fetch_line=$(grep -nF 'fetch_release_history "$current_manifest" "$candidate_tmp"' "$release" | cut -d: -f1)
upgrade_install_line=$(grep -nF 'mv -f "$candidate_tmp" "$candidate_manifest"' "$release" | tail -n 1 | cut -d: -f1)
schema_transition_clear_count=$(grep -cF 'rm -f "$schema_transition_manifest"' "$release")
restore_deploy_line=$(grep -nF 'deploy_manifest "$current_manifest" false' "$release" | cut -d: -f1)
upgrade_restore_deploy_line=$(grep -nF 'deploy_manifest "$schema_transition_manifest" false' "$release" | head -n 1 | cut -d: -f1)
initial_restore_deploy_line=$(grep -nF 'deploy_manifest "$schema_transition_manifest" false' "$release" | tail -n 1 | cut -d: -f1)
upgrade_restore_promote_line=$(grep -nF 'promote_schema_transition' "$release" | tail -n 3 | head -n 1 | cut -d: -f1)
initial_restore_promote_line=$(grep -nF 'promote_schema_transition' "$release" | tail -n 2 | head -n 1 | cut -d: -f1)
restore_candidate_validation_line=$(grep -nF 'validate_transition_candidate' "$release" | tail -n 1 | cut -d: -f1)
restore_marker_fetch_line=$(grep -nF 'fetch_release_history "$current_manifest" "$schema_transition_manifest"' "$release" | cut -d: -f1)
restore_initial_fetch_line=$(grep -nF 'fetch_release_history "$schema_transition_manifest"' "$release" | cut -d: -f1)
marked_restore_finalize_line=$(grep -nF 'finalize_schema_transition # marked restore' "$release" | cut -d: -f1)
current_restore_candidate_clear_line=$(grep -nF 'rm -f "$candidate_manifest" # discard unmarked stale candidate before restore' "$release" | cut -d: -f1)
current_restore_finalize_line=$(grep -nF 'finalize_schema_transition # current restore' "$release" | cut -d: -f1)
promotion_sync_line=$(grep -nE '^  sync$' "$release" | tail -n 2 | head -n 1 | cut -d: -f1)
schema_transition_clear_line=$(grep -nF 'rm -f "$schema_transition_manifest"' "$release" | cut -d: -f1)
clear_sync_line=$(grep -nE '^  sync$' "$release" | tail -n 1 | cut -d: -f1)
previous_promotion_line=$(grep -nF 'mv -f "$previous_tmp" "$previous_manifest"' "$release" | head -n 1 | cut -d: -f1)
current_install_line=$(grep -nF 'install -m 600 "$schema_transition_manifest" "$current_tmp"' "$release" | cut -d: -f1)
promotion_inputs_durable_line=$(grep -nF 'sync # promotion inputs must be durable before current promotion' "$release" | cut -d: -f1)
current_promotion_line=$(grep -nF 'mv -f "$current_tmp" "$current_manifest"' "$release" | head -n 1 | cut -d: -f1)
rollback_guard_line=$(grep -nF 'require_schema_safe_rollback "$current_manifest" "$previous_manifest"' "$release" | cut -d: -f1)
minimum_rollback_guard_line=$(grep -nF 'require_pre_minimum_rollback_ready "$current_manifest" "$previous_manifest"' "$release" | cut -d: -f1)
rollback_fetch_line=$(grep -nF 'fetch_release_history "$current_manifest" "$previous_manifest"' "$release" | cut -d: -f1)
rollback_deploy_line=$(grep -nF 'deploy_manifest "$previous_manifest" false' "$release" | cut -d: -f1)
rollback_candidate_clear_line=$(grep -nF 'rm -f "$candidate_manifest" # discard unmarked stale candidate before rollback' "$release" | cut -d: -f1)
rollback_promote_line=$(grep -nF 'promote_schema_transition # rollback' "$release" | cut -d: -f1)
rollback_finalize_line=$(grep -nF 'finalize_schema_transition # rollback' "$release" | cut -d: -f1)
rollback_current_temp_install_line=$(grep -nF 'install -m 600 "$rollback_current" "$current_tmp"' "$release" | cut -d: -f1)
rollback_previous_temp_install_line=$(grep -nF 'install -m 600 "$rollback_previous" "$previous_tmp"' "$release" | cut -d: -f1)
rollback_temp_sync_line=$(grep -nF 'sync # rollback replacement tempfiles durable before rename' "$release" | cut -d: -f1)
rollback_current_mv_line=$(grep -nF 'mv -f "$current_tmp" "$current_manifest"' "$release" | tail -n 1 | cut -d: -f1)
rollback_previous_mv_line=$(grep -nF 'mv -f "$previous_tmp" "$previous_manifest"' "$release" | tail -n 1 | cut -d: -f1)
rollback_pair_sync_line=$(grep -nF 'sync # rollback manifest pair durable before marker removal' "$release" | cut -d: -f1)
rollback_marker_remove_line=$(grep -nF 'rm -f "$rollback_marker"' "$release" | cut -d: -f1)
rollback_marker_remove_sync_line=$(grep -nF 'sync # rollback marker removal durable before staged cleanup' "$release" | cut -d: -f1)
rollback_staged_remove_line=$(grep -nF 'rm -f "$rollback_current" "$rollback_previous"' "$release" | cut -d: -f1)
rollback_staged_remove_sync_line=$(grep -nF 'sync # rollback staged cleanup durable' "$release" | cut -d: -f1)
adopt_guard_line=$(grep -nF 'manifest_contains_path "$candidate_tmp" "$world_memory_projection_migration"' "$release" | cut -d: -f1)
adopt_fetch_line=$(grep -nF 'fetch_release_history "$candidate_tmp"' "$release" | cut -d: -f1)
adopt_deploy_line=$(grep -nF 'deploy_manifest "$candidate_manifest" true' "$release" | head -n 1 | cut -d: -f1)
gate_recovery_call_line=$(grep -nF '[[ "$confirmation" == "$release_sha" ]] || recover_client_after_gate_failure' "$release" | cut -d: -f1)
current_restore_line=$(grep -nF 'active_manifest="$current_manifest"' "$release" | cut -d: -f1)
current_restore_up_line=$(grep -nF 'frontend nginx; then' "$release" | head -n 1 | cut -d: -f1)
current_restore_narrative_line=$(grep -nF 'narrative-service frontend nginx; then' "$release" | cut -d: -f1)
current_restore_fail_stop_line=$(grep -nF 'fail_stop_client "client contract gate was not confirmed and current client restore failed"' "$release" | cut -d: -f1)
current_restore_die_line=$(grep -nF 'die "client contract gate was not confirmed; current client was restored"' "$release" | cut -d: -f1)
adopt_fail_stop_line=$(grep -nF 'fail_stop_client "client contract gate was not confirmed and no current release exists"' "$release" | cut -d: -f1)
fail_stop_nginx_line=$(grep -nF 'compose stop --timeout 120 nginx' "$release" | cut -d: -f1)
fail_stop_frontend_line=$(grep -nF 'compose stop --timeout 120 frontend' "$release" | cut -d: -f1)
fail_stop_die_line=$(grep -nF 'die "$reason; client was stopped (fail-stopped)"' "$release" | cut -d: -f1)
[ -n "$narrative_quiesce_line" ] && [ -n "$agent_quiesce_line" ] \
  && [ -n "$candidate_client_line" ] \
  && [ -n "$gate_fail_stop_line" ] \
  && [ "$narrative_quiesce_line" -lt "$candidate_client_line" ] \
  && [ "$candidate_client_line" -lt "$gate_fail_stop_line" ] \
  && [ "$gate_fail_stop_line" -lt "$agent_quiesce_line" ] \
  && [ -n "$schema_transition_line" ] \
  && [ "$agent_quiesce_line" -lt "$schema_transition_line" ] \
  && [ "$schema_transition_line" -lt "$marker_sync_line" ] \
  && [ "$marker_sync_line" -lt "$migrate_line" ] \
  && [ "$agent_quiesce_line" -lt "$migrate_line" ] \
  || { printf 'drill: FAIL candidate client is not fail-stopped from the old producer before migrations\n' >&2; exit 1; }
[ -n "$upgrade_guard_line" ] && [ "$upgrade_guard_line" -lt "$upgrade_install_line" ] \
  || { printf 'drill: FAIL schema downgrade guard does not precede candidate installation\n' >&2; exit 1; }
[ "$schema_transition_clear_count" -eq 1 ] \
  && [ "$promotion_sync_line" -lt "$schema_transition_clear_line" ] \
  && [ "$schema_transition_clear_line" -lt "$clear_sync_line" ] \
  || { printf 'drill: FAIL schema marker can clear before durable promotion\n' >&2; exit 1; }
[ -n "$previous_promotion_line" ] \
  && [ -n "$current_install_line" ] \
  && [ -n "$promotion_inputs_durable_line" ] \
  && [ -n "$current_promotion_line" ] \
  && [ "$current_install_line" -lt "$promotion_inputs_durable_line" ] \
  && [ "$previous_promotion_line" -lt "$promotion_inputs_durable_line" ] \
  && [ "$promotion_inputs_durable_line" -lt "$current_promotion_line" ] \
  || { printf 'drill: FAIL current can become durable before its previous release\n' >&2; exit 1; }
[ -n "$initial_restore_deploy_line" ] \
  && [ "$initial_restore_deploy_line" -lt "$initial_restore_promote_line" ] \
  && [ -n "$upgrade_restore_deploy_line" ] \
  && [ "$upgrade_restore_deploy_line" -lt "$upgrade_restore_promote_line" ] \
  && [ "$restore_candidate_validation_line" -lt "$upgrade_restore_deploy_line" ] \
  && [ "$restore_marker_fetch_line" -lt "$upgrade_restore_deploy_line" ] \
  && [ "$restore_initial_fetch_line" -lt "$initial_restore_deploy_line" ] \
  && [ "$initial_restore_promote_line" -lt "$marked_restore_finalize_line" ] \
  && [ "$upgrade_restore_promote_line" -lt "$marked_restore_finalize_line" ] \
  || { printf 'drill: FAIL interrupted schema transition cannot roll forward before promotion\n' >&2; exit 1; }
[ -n "$restore_deploy_line" ] \
  && [ "$current_restore_candidate_clear_line" -lt "$restore_deploy_line" ] \
  && [ "$restore_deploy_line" -lt "$current_restore_finalize_line" ] \
  || { printf 'drill: FAIL normal restore can retain a stale candidate or pending marker\n' >&2; exit 1; }
[ -n "$rollback_guard_line" ] \
  && [ "$rollback_guard_line" -lt "$rollback_candidate_clear_line" ] \
  && [ -n "$minimum_rollback_guard_line" ] \
  && [ "$rollback_guard_line" -lt "$minimum_rollback_guard_line" ] \
  && [ "$minimum_rollback_guard_line" -lt "$rollback_candidate_clear_line" ] \
  && [ "$rollback_candidate_clear_line" -lt "$rollback_deploy_line" ] \
  && [ "$rollback_deploy_line" -lt "$rollback_promote_line" ] \
  && [ "$rollback_promote_line" -lt "$rollback_finalize_line" ] \
  || { printf 'drill: FAIL schema rollback guard does not precede deployment\n' >&2; exit 1; }
[ "$rollback_current_temp_install_line" -lt "$rollback_temp_sync_line" ] \
  && [ "$rollback_previous_temp_install_line" -lt "$rollback_temp_sync_line" ] \
  && [ "$rollback_temp_sync_line" -lt "$rollback_current_mv_line" ] \
  && [ "$rollback_temp_sync_line" -lt "$rollback_previous_mv_line" ] \
  && [ "$rollback_current_mv_line" -lt "$rollback_pair_sync_line" ] \
  && [ "$rollback_previous_mv_line" -lt "$rollback_pair_sync_line" ] \
  && [ "$rollback_pair_sync_line" -lt "$rollback_marker_remove_line" ] \
  && [ "$rollback_marker_remove_line" -lt "$rollback_marker_remove_sync_line" ] \
  && [ "$rollback_marker_remove_sync_line" -lt "$rollback_staged_remove_line" ] \
  && [ "$rollback_staged_remove_line" -lt "$rollback_staged_remove_sync_line" ] \
  || { printf 'drill: FAIL rollback pair or marker can outrun its durable recovery data\n' >&2; exit 1; }
[ -n "$adopt_guard_line" ] && [ "$adopt_guard_line" -lt "$adopt_deploy_line" ] \
  || { printf 'drill: FAIL schema adopt guard does not precede deployment\n' >&2; exit 1; }
[ -n "$adopt_fetch_line" ] && [ "$adopt_fetch_line" -lt "$adopt_guard_line" ] \
  && [ -n "$upgrade_fetch_line" ] && [ "$upgrade_fetch_line" -lt "$upgrade_guard_line" ] \
  && [ -n "$rollback_fetch_line" ] && [ "$rollback_fetch_line" -lt "$rollback_guard_line" ] \
  || { printf 'drill: FAIL trusted history is not fetched before every schema guard\n' >&2; exit 1; }
[ -n "$gate_recovery_call_line" ] \
  && [ -n "$current_restore_line" ] \
  && [ -n "$current_restore_narrative_line" ] \
  && [ "$current_restore_line" -lt "$current_restore_up_line" ] \
  && [ "$current_restore_up_line" -lt "$current_restore_fail_stop_line" ] \
  && [ "$current_restore_fail_stop_line" -lt "$current_restore_die_line" ] \
  && [ -n "$adopt_fail_stop_line" ] \
  && [ -n "$fail_stop_nginx_line" ] \
  && [ "$fail_stop_nginx_line" -lt "$fail_stop_frontend_line" ] \
  && [ "$fail_stop_frontend_line" -lt "$fail_stop_die_line" ] \
  || { printf 'drill: FAIL rejected client gate can exit before restore or fail-stop\n' >&2; exit 1; }
printf 'drill: ok   candidate client fail-stop, migration drain and downgrade barrier ordering\n'
printf 'drill: ok   rejected client gate restores current or stops candidate client before exit\n'

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

# A release artifact normally arrives before its commit exists in the local
# deployment clone. Prove the schema guard fetches that trusted remote history
# and reaches the human gate without touching Docker or the public network.
fetch_origin="$work/fetch-origin.git"
fetch_seed="$work/fetch-seed"
fetch_client="$work/fetch-client"
git init --bare "$fetch_origin" >/dev/null
git -C "$fetch_origin" symbolic-ref HEAD refs/heads/main
git init --initial-branch=main "$fetch_seed" >/dev/null
git -C "$fetch_seed" config user.name 'NovelWorld release drill'
git -C "$fetch_seed" config user.email 'release-drill@test.invalid'
printf 'base\n' >"$fetch_seed/README.md"
git -C "$fetch_seed" add README.md
git -C "$fetch_seed" commit -m base >/dev/null
git -C "$fetch_seed" remote add origin "$fetch_origin"
git -C "$fetch_seed" push -u origin main >/dev/null
git clone "$fetch_origin" "$fetch_client" >/dev/null
mkdir -p "$fetch_seed/infra/postgres/migrations"
printf '%s\n' '-- remote-only migration contract' \
  >"$fetch_seed/infra/postgres/migrations/0021_world_turn_memory_projection.sql"
git -C "$fetch_seed" add infra/postgres/migrations/0021_world_turn_memory_projection.sql
git -C "$fetch_seed" commit -m candidate >/dev/null
remote_candidate=$(git -C "$fetch_seed" rev-parse HEAD)
git -C "$fetch_seed" push origin main >/dev/null
if git -C "$fetch_client" cat-file -e "$remote_candidate^{commit}" 2>/dev/null; then
  printf 'drill: FAIL remote candidate unexpectedly existed before fetch\n' >&2
  exit 1
fi
remote_manifest=$(mktemp "$work/remote-candidate.XXXXXX")
write_manifest "$remote_manifest" "$remote_candidate"
remote_state=$(new_state)
expect_fail 'adopt fetches a remote-only candidate before the schema guard' \
  'manual recovery was not confirmed' bash -c \
  'cd "$1"; printf "NO\n" | RELEASE_STATE_DIR="$2" "$3" adopt "$4"' \
  _ "$fetch_client" "$remote_state" "$release" "$remote_manifest"
git -C "$fetch_client" cat-file -e "$remote_candidate^{commit}"

# A downloaded candidate is not evidence that its migration ran. Before the
# durable transition marker exists, restore reaches the current deployment
# preflight. Once migration may have started, the same pre-0020 current target
# can no longer run; restore must roll the exact marked release forward.
remote_base=$(git -C "$fetch_client" rev-parse "$remote_candidate^")
remote_restore_state=$(new_state)
remote_current=$(mktemp "$work/remote-current.XXXXXX")
write_manifest "$remote_current" "$remote_base"
cp "$remote_current" "$remote_restore_state/current.env"
cp "$remote_manifest" "$remote_restore_state/candidate.env"
expect_fail 'pre-migration candidate does not block current restore' \
  'production secrets file not found' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_restore_state" "$release"
cp "$remote_manifest" "$remote_restore_state/schema-transition.pending"
expect_fail 'migration-phase marker rolls forward instead of restoring a pre-0020 writer' \
  'production secrets file not found' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_restore_state" "$release"
rm -f "$remote_restore_state/candidate.env"
expect_fail 'migration-phase marker remains authoritative without candidate' \
  'production secrets file not found' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_restore_state" "$release"
cp "$remote_current" "$remote_restore_state/candidate.env"
expect_fail 'migration-phase marker rejects a different candidate' \
  'candidate release does not match the schema transition' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_restore_state" "$release"

# Initial adoption has no current release to restore. Once the durable marker
# exists, restore must roll the exact marked release forward (even if the
# downloaded candidate vanished) instead of wedging every state-machine entry.
remote_initial_state=$(new_state)
cp "$remote_manifest" "$remote_initial_state/schema-transition.pending"
cp "$remote_manifest" "$remote_initial_state/candidate.env"
expect_fail 'interrupted initial adoption reaches exact roll-forward preflight' \
  'production secrets file not found' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_initial_state" "$release"
rm -f "$remote_initial_state/candidate.env"
expect_fail 'initial adoption marker remains authoritative without candidate' \
  'production secrets file not found' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_initial_state" "$release"
cp "$remote_current" "$remote_initial_state/candidate.env"
expect_fail 'initial adoption rejects a candidate different from its marker' \
  'candidate release does not match the schema transition' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_initial_state" "$release"
rm -f "$remote_initial_state/candidate.env"
cp "$remote_current" "$remote_initial_state/previous.env"
expect_fail 'initial adoption marker rejects an impossible previous release' \
  'initial schema transition cannot coexist with a previous release' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_initial_state" "$release"
rm -f "$remote_initial_state/previous.env"
cp "$remote_current" "$remote_initial_state/schema-transition.pending"
expect_fail 'initial adoption marker must contain migration 0021' \
  'initial schema transition predates the world-memory projection contract' bash -c \
  'cd "$1"; RELEASE_STATE_DIR="$2" "$3" restore' \
  _ "$fetch_client" "$remote_initial_state" "$release"

# Exercise the successful half of initial-adoption recovery without a registry:
# real git/state transitions, with only Docker and health probes replaced by
# deterministic local successes. The missing candidate proves the marker alone
# is the durable target.
roll_repo="$work/initial-roll-forward-repo"
roll_bin="$work/initial-roll-forward-bin"
mkdir -p "$roll_repo" "$roll_bin"
git init --initial-branch=main "$roll_repo" >/dev/null
git -C "$roll_repo" config user.name 'NovelWorld release drill'
git -C "$roll_repo" config user.email 'release-drill@test.invalid'
printf '%s\n' '.env' >"$roll_repo/.gitignore"
printf '%s\n' 'services: {}' >"$roll_repo/docker-compose.yml"
git -C "$roll_repo" add .gitignore docker-compose.yml
git -C "$roll_repo" commit -m 'pre-0020 control fixture' >/dev/null
roll_base_sha=$(git -C "$roll_repo" rev-parse HEAD)
mkdir -p "$roll_repo/infra/postgres/migrations"
printf '%s\n' '-- initial roll-forward migration contract' \
  >"$roll_repo/infra/postgres/migrations/0021_world_turn_memory_projection.sql"
git -C "$roll_repo" add infra/postgres/migrations/0021_world_turn_memory_projection.sql
git -C "$roll_repo" commit -m 'initial roll-forward fixture' >/dev/null
roll_sha=$(git -C "$roll_repo" rev-parse HEAD)
printf '%s\n' 'post-0020 release fixture' >"$roll_repo/release-fixture.txt"
mkdir -p "$roll_repo/docs/adr"
printf '%s\n' '# Minimal bootstrap decision fixture' \
  >"$roll_repo/docs/adr/0002-minimal-bootstrap-and-deferred-runtime-configuration.md"
git -C "$roll_repo" add release-fixture.txt \
  docs/adr/0002-minimal-bootstrap-and-deferred-runtime-configuration.md
git -C "$roll_repo" commit -m 'post-0020 rollback fixture' >/dev/null
roll_new_sha=$(git -C "$roll_repo" rev-parse HEAD)
printf '%s\n' \
  'REDIS_PASSWORD=0123456789abcdef0123456789abcdef' \
  'LLM_API_KEY=' >"$roll_repo/.env"

cat >"$roll_bin/docker" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = inspect ]; then
  printf '%s\n' "${MOCK_REDIS_HEALTH:-healthy}"
elif [ "${1:-}" = exec ]; then
  case " $* " in
    *" novel-redis "*) [ "${MOCK_REDIS_AUTH:-ok}" = ok ] || exit 1 ;;
    *" novel-user-service "*) [ "${MOCK_LLM_CONFIGURED:-false}" = true ] || exit 1 ;;
  esac
fi
exit 0
EOF
cat >"$roll_bin/curl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$roll_bin/sync" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$roll_bin/docker" "$roll_bin/curl" "$roll_bin/sync"

roll_marker=$(mktemp "$work/roll-marker.XXXXXX")
write_manifest "$roll_marker" "$roll_sha"
roll_state=$(new_state)
cp "$roll_marker" "$roll_state/schema-transition.pending"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_state" "$release" restore
)
cmp -s "$roll_marker" "$roll_state/current.env" \
  || { printf 'drill: FAIL initial roll-forward promoted a different manifest\n' >&2; exit 1; }
test ! -e "$roll_state/schema-transition.pending" \
  || { printf 'drill: FAIL initial roll-forward left the schema marker\n' >&2; exit 1; }
test ! -e "$roll_state/candidate.env" \
  || { printf 'drill: FAIL initial roll-forward left a candidate\n' >&2; exit 1; }
test ! -e "$roll_state/previous.env" \
  || { printf 'drill: FAIL initial roll-forward invented a previous release\n' >&2; exit 1; }
grep -qx 'CACHE_MODE=redis' "$roll_repo/.env" \
  || { printf 'drill: FAIL legacy Redis environment did not persist redis mode\n' >&2; exit 1; }
printf 'drill: ok   interrupted initial adoption rolls exact marker forward and promotes atomically\n'

roll_base_manifest=$(mktemp "$work/roll-base.XXXXXX")
write_manifest "$roll_base_manifest" "$roll_base_sha"
roll_upgrade_state=$(new_state)
cp "$roll_base_manifest" "$roll_upgrade_state/current.env"
cp "$roll_marker" "$roll_upgrade_state/schema-transition.pending"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_upgrade_state" "$release" restore
)
cmp -s "$roll_marker" "$roll_upgrade_state/current.env" \
  || { printf 'drill: FAIL upgrade roll-forward promoted a different manifest\n' >&2; exit 1; }
cmp -s "$roll_base_manifest" "$roll_upgrade_state/previous.env" \
  || { printf 'drill: FAIL upgrade roll-forward lost its previous release\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/schema-transition.pending" \
  || { printf 'drill: FAIL upgrade roll-forward left the schema marker\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/candidate.env" \
  || { printf 'drill: FAIL upgrade roll-forward left a candidate\n' >&2; exit 1; }
printf 'drill: ok   interrupted 0020 upgrade rolls exact marker forward with previous preserved\n'

# A crash after current promotion but before marker cleanup is schema-compatible:
# restore the now-current release and discard the stale matching candidate.
cp "$roll_marker" "$roll_upgrade_state/schema-transition.pending"
cp "$roll_marker" "$roll_upgrade_state/candidate.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_upgrade_state" "$release" restore
)
cmp -s "$roll_marker" "$roll_upgrade_state/current.env" \
  || { printf 'drill: FAIL post-promotion restore changed current\n' >&2; exit 1; }
cmp -s "$roll_base_manifest" "$roll_upgrade_state/previous.env" \
  || { printf 'drill: FAIL post-promotion restore changed previous\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/schema-transition.pending" \
  || { printf 'drill: FAIL post-promotion restore left the schema marker\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/candidate.env" \
  || { printf 'drill: FAIL post-promotion restore left the candidate\n' >&2; exit 1; }
printf 'drill: ok   post-promotion restore clears matching transition residue\n'

# A normal restore creates a temporary exact schema marker while migrations
# replay. A rejected-gate candidate is not authority and must be removed before
# that marker becomes durable, then the healthy restore must clear the marker.
roll_new_manifest=$(mktemp "$work/roll-new.XXXXXX")
write_manifest "$roll_new_manifest" "$roll_new_sha"
cp "$roll_new_manifest" "$roll_upgrade_state/candidate.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_upgrade_state" "$release" restore
)
cmp -s "$roll_marker" "$roll_upgrade_state/current.env" \
  || { printf 'drill: FAIL normal restore changed current\n' >&2; exit 1; }
cmp -s "$roll_base_manifest" "$roll_upgrade_state/previous.env" \
  || { printf 'drill: FAIL normal restore changed previous\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/candidate.env" \
  || { printf 'drill: FAIL normal restore retained a stale candidate\n' >&2; exit 1; }
test ! -e "$roll_upgrade_state/schema-transition.pending" \
  || { printf 'drill: FAIL normal restore left the schema marker\n' >&2; exit 1; }
printf 'drill: ok   normal restore clears stale candidate and exact schema marker\n'

# A healthy rollback uses the schema-transition promotion protocol itself: the
# target becomes current, the former current becomes previous, and no recovery
# authority or staged file remains.
roll_guard_state=$(new_state)
cp "$roll_new_manifest" "$roll_guard_state/current.env"
cp "$roll_marker" "$roll_guard_state/previous.env"
sed -i 's/^CACHE_MODE=.*/CACHE_MODE=postgres/' "$roll_repo/.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_guard_state" \
    expect_fail 'pre-minimum rollback rejects PostgreSQL cache mode before replacement' \
      'rollback target predates minimal bootstrap and requires CACHE_MODE=redis' \
      "$release" rollback "$roll_sha"
)
sed -i 's/^CACHE_MODE=.*/CACHE_MODE=redis/' "$roll_repo/.env"
sed -i 's/^REDIS_PASSWORD=.*/REDIS_PASSWORD=/' "$roll_repo/.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_guard_state" \
    expect_fail 'pre-minimum rollback rejects a half-selected Redis mode' \
      'CACHE_MODE=redis requires a URL-safe, non-placeholder REDIS_PASSWORD' \
      "$release" rollback "$roll_sha"
)
sed -i 's/^REDIS_PASSWORD=.*/REDIS_PASSWORD=0123456789abcdef0123456789abcdef/' "$roll_repo/.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" MOCK_REDIS_HEALTH=starting \
    RELEASE_STATE_DIR="$roll_guard_state" \
    expect_fail 'pre-minimum rollback requires healthy Redis' \
      'rollback target predates minimal bootstrap and requires healthy Redis' \
      "$release" rollback "$roll_sha"
)
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" MOCK_REDIS_AUTH=fail \
    RELEASE_STATE_DIR="$roll_guard_state" \
    expect_fail 'pre-minimum rollback authenticates with the persisted Redis credential' \
      'persisted Redis credential to authenticate' \
      "$release" rollback "$roll_sha"
)
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_guard_state" \
    expect_fail 'pre-minimum rollback requires an effective LLM configuration' \
      'current User Service did not prove a decryptable database LLM configuration' \
      "$release" rollback "$roll_sha"
)
test ! -e "$roll_guard_state/schema-transition.pending" \
  || { printf 'drill: FAIL rejected compatibility guard replaced services\n' >&2; exit 1; }

roll_override_state=$(new_state)
cp "$roll_new_manifest" "$roll_override_state/current.env"
cp "$roll_marker" "$roll_override_state/previous.env"
sed -i 's/^LLM_API_KEY=.*/LLM_API_KEY=sk-0123456789abcdef0123456789abcdef/' "$roll_repo/.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_override_state" \
    "$release" rollback "$roll_sha"
)
cmp -s "$roll_marker" "$roll_override_state/current.env" \
  || { printf 'drill: FAIL valid LLM environment override did not permit guarded rollback\n' >&2; exit 1; }
sed -i 's/^LLM_API_KEY=.*/LLM_API_KEY=/' "$roll_repo/.env"

roll_rollback_state=$(new_state)
cp "$roll_new_manifest" "$roll_rollback_state/current.env"
cp "$roll_marker" "$roll_rollback_state/previous.env"
cp "$roll_base_manifest" "$roll_rollback_state/candidate.env"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" MOCK_LLM_CONFIGURED=true RELEASE_STATE_DIR="$roll_rollback_state" \
    "$release" rollback "$roll_sha"
)
cmp -s "$roll_marker" "$roll_rollback_state/current.env" \
  || { printf 'drill: FAIL rollback did not promote its target\n' >&2; exit 1; }
cmp -s "$roll_new_manifest" "$roll_rollback_state/previous.env" \
  || { printf 'drill: FAIL rollback did not preserve the former current\n' >&2; exit 1; }
for residue in candidate.env schema-transition.pending rollback.pending \
  rollback-current.env rollback-previous.env; do
  test ! -e "$roll_rollback_state/$residue" \
    || { printf 'drill: FAIL healthy rollback left %s\n' "$residue" >&2; exit 1; }
done
printf 'drill: ok   healthy rollback promotes one durable pair without residue\n'

# Compatibility with a valid rollback.pending left by the old release script:
# acquire repairs that pair first, then the exact schema marker replays and
# clears normally.
roll_legacy_pending_state=$(new_state)
cp "$roll_new_manifest" "$roll_legacy_pending_state/current.env"
cp "$roll_marker" "$roll_legacy_pending_state/previous.env"
cp "$roll_marker" "$roll_legacy_pending_state/rollback-current.env"
cp "$roll_new_manifest" "$roll_legacy_pending_state/rollback-previous.env"
cp "$roll_marker" "$roll_legacy_pending_state/schema-transition.pending"
: >"$roll_legacy_pending_state/rollback.pending"
(
  cd "$roll_repo"
  PATH="$roll_bin:$PATH" RELEASE_STATE_DIR="$roll_legacy_pending_state" "$release" restore
)
cmp -s "$roll_marker" "$roll_legacy_pending_state/current.env" \
  || { printf 'drill: FAIL legacy rollback recovery changed the exact target\n' >&2; exit 1; }
cmp -s "$roll_new_manifest" "$roll_legacy_pending_state/previous.env" \
  || { printf 'drill: FAIL legacy rollback recovery lost the former current\n' >&2; exit 1; }
for residue in schema-transition.pending rollback.pending \
  rollback-current.env rollback-previous.env; do
  test ! -e "$roll_legacy_pending_state/$residue" \
    || { printf 'drill: FAIL legacy rollback recovery left %s\n' "$residue" >&2; exit 1; }
done
printf 'drill: ok   legacy rollback marker converges through exact schema restore\n'

# --- upgrade pre-flights: basic manifest/infra guards fire before fetch ---
state=$(new_state)
current=$(mktemp "$work/current.XXXXXX")
candidate=$(mktemp "$work/candidate.XXXXXX")
write_manifest "$current" "$sha_a"
write_manifest "$candidate" "$sha_a"
cp "$current" "$state/current.env"
cp "$current" "$state/schema-transition.pending"
RELEASE_STATE_DIR=$state expect_fail 'upgrade refuses while a schema transition is pending' \
  'schema transition is pending; use restore' "$release" upgrade "$candidate"
rm -f "$state/schema-transition.pending"
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

# Once migration 0021 is committed, exercise every real-history downgrade path
# using its direct ancestor. Before that commit exists (for example, in a local
# uncommitted worktree), the static ordering assertions above still run.
world_memory_projection_migration=infra/postgres/migrations/0021_world_turn_memory_projection.sql
if git cat-file -e "HEAD:$world_memory_projection_migration" 2>/dev/null; then
  introduced=$(git log --diff-filter=A --format=%H -- "$world_memory_projection_migration" | tail -n 1)
  before_introduction=$(git rev-parse "$introduced^")
  after_introduction=$(git rev-parse HEAD)
  schema_error='rollback target predates the world-memory projection contract'
  schema_current=$(mktemp "$work/schema-current.XXXXXX")
  schema_candidate=$(mktemp "$work/schema-candidate.XXXXXX")

  state_schema=$(new_state)
  write_manifest "$schema_current" "$after_introduction"
  write_manifest "$schema_candidate" "$before_introduction"
  cp "$schema_current" "$state_schema/current.env"
  RELEASE_STATE_DIR=$state_schema expect_fail 'upgrade cannot disguise a schema downgrade' \
    "$schema_error" "$release" upgrade "$schema_candidate"

  state_schema_adopt=$(new_state)
  RELEASE_STATE_DIR=$state_schema_adopt expect_fail 'adopt cannot activate a pre-0020 release' \
    'adopt target predates the world-memory projection contract' \
    "$release" adopt "$schema_candidate"

  state_schema_rollback=$(new_state)
  cp "$schema_current" "$state_schema_rollback/current.env"
  cp "$schema_candidate" "$state_schema_rollback/previous.env"
  RELEASE_STATE_DIR=$state_schema_rollback expect_fail 'rollback cannot cross migration 0021' \
    "$schema_error" "$release" rollback "$before_introduction"

  state_schema_restore=$(new_state)
  cp "$schema_candidate" "$state_schema_restore/current.env"
  cp "$schema_current" "$state_schema_restore/schema-transition.pending"
  RELEASE_STATE_DIR=$state_schema_restore expect_fail 'restore rolls marked migration forward rather than reviving pre-0020 current' \
    'production secrets file not found' "$release" restore
else
  printf 'drill: skip real-history schema barriers until migration 0021 is committed\n'
fi

# --- rollback pre-flights ---
RELEASE_STATE_DIR=$state expect_fail 'rollback rejects a malformed SHA argument' 'invalid rollback git SHA' \
  "$release" rollback not-a-sha
cp "$current" "$state/schema-transition.pending"
RELEASE_STATE_DIR=$state expect_fail 'rollback cannot bypass a pending schema transition as already current' \
  'schema transition is pending; use restore' "$release" rollback "$sha_a"
test -e "$state/schema-transition.pending" \
  || { printf 'drill: FAIL already-current rollback removed the schema marker\n' >&2; exit 1; }
rm -f "$state/schema-transition.pending"
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
