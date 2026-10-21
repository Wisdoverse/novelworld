#!/usr/bin/env bash
# Record one trusted Jenkins build. Native Compose labels only select images.
set -euo pipefail
umask 077
cd "$(dirname "$0")/../.."

fail() { printf 'Jenkins image record: %s\n' "$1" >&2; exit 1; }
services=(gateway user-service novel-service agent-service narrative-service frontend)
record_dir=.local/jenkins-deployment
compose=(docker compose --env-file .env)
unset COMPOSE_PROJECT_NAME
mkdir -p "$record_dir"
# A failed attempt must not leave a previous success available for archival.
rm -f "$record_dir/deployment.tsv"
[[ ${1:-} != build ]] || rm -f "$record_dir/build.tsv"
[[ ${BUILD_NUMBER:-} =~ ^[1-9][0-9]*$ ]] || fail 'BUILD_NUMBER is missing or invalid. Run this from Jenkins.'
[[ -f .env && ! -e .release ]] || fail 'Use the configured source workspace. Check .env and .release.'

clean_source() {
    git diff --quiet HEAD -- || fail 'Tracked files changed. Restore the reviewed source before deployment.'
    [[ -z $(git ls-files --others --exclude-standard) ]] || fail 'Untracked files exist. Review them before deployment.'
    git rev-parse --verify HEAD
}

source_sha=$(clean_source)
[[ $source_sha =~ ^[0-9a-f]{40}$ ]] || fail 'The source commit is invalid. Check the Git checkout.'
temporary=$(mktemp "$record_dir/.record.XXXXXX")
trap 'rm -f "$temporary"' EXIT

case ${1:-} in
    build)
        "${compose[@]}" build "${services[@]}"
        [[ $(clean_source) == "$source_sha" ]] || fail 'Source changed during the build. Review the workspace before retrying.'
        printf '%s\t%s\n' "$BUILD_NUMBER" "$source_sha" >"$temporary"
        for service in "${services[@]}"; do
            candidates=$("${compose[@]}" config --images "$service")
            built_image=
            # Compose includes dependency images in this list.
            while IFS= read -r reference; do
                metadata=$(docker image inspect --format '{{.Id}} {{index .Config.Labels "com.docker.compose.service"}}' "$reference" 2>/dev/null) || continue
                read -r image_id image_service extra <<<"$metadata"
                [[ $image_service == "$service" ]] || continue
                [[ $image_id =~ ^sha256:[0-9a-f]{64}$ && -z $extra ]] || fail "Invalid image metadata for $service. Inspect the build."
                [[ -z $built_image || $built_image == "$image_id" ]] || fail "Multiple images match $service. Inspect the build."
                built_image=$image_id
            done <<<"$candidates"
            [[ -n $built_image ]] || fail "No built image matches $service. Inspect the build."
            printf '%s\t%s\n' "$service" "$built_image" >>"$temporary"
        done
        mv "$temporary" "$record_dir/build.tsv"
        ;;
    verify)
        [[ -f $record_dir/build.tsv ]] || fail 'The build record is missing. Run the build stage first.'
        exec 3<"$record_dir/build.tsv"
        read -r recorded_build recorded_source extra <&3
        [[ $recorded_build == "$BUILD_NUMBER" && $recorded_source == "$source_sha" && -z $extra ]] || fail 'The build or source changed. Inspect the deployment before retrying.'
        printf 'source\t%s\n' "$source_sha" >"$temporary"
        for service in "${services[@]}"; do
            read -r recorded_service image_id extra <&3
            [[ $recorded_service == "$service" && $image_id =~ ^sha256:[0-9a-f]{64}$ && -z $extra ]] || fail 'The build record is invalid. Inspect the deployment before retrying.'
            container=$("${compose[@]}" ps --all --quiet "$service")
            [[ $container =~ ^[0-9a-f]{64}$ ]] || fail "Expected one container for $service. Inspect the deployment."
            actual=$(docker container inspect --format '{{.Image}} {{.State.Running}} {{index .Config.Labels "com.docker.compose.service"}}' "$container")
            [[ $actual == "$image_id true $service" ]] || fail "The running image for $service differs from this build. Inspect the deployment."
            printf '%s\t%s\t%s\n' "$service" "$image_id" "$container" >>"$temporary"
        done
        [[ -z $(cat <&3) ]] || fail 'The build record has extra data. Inspect the deployment before retrying.'
        exec 3<&-
        [[ $(clean_source) == "$source_sha" ]] || fail 'Source changed during verification. Inspect the deployment before retrying.'
        mv "$temporary" "$record_dir/deployment.tsv"
        ;;
    *) fail 'Use build or verify.' ;;
esac
