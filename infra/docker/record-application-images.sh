#!/usr/bin/env bash
# Consume the six successful build/scan records from this workflow run.
set -euo pipefail

die() { printf 'release-images: %s\n' "$*" >&2; exit 1; }
[[ $# -eq 2 ]] || die 'usage: record-application-images.sh RECORD_DIR IMAGE_PREFIX'
record_dir=$1
image_prefix=$2
[[ -d "$record_dir" && ! -L "$record_dir" ]] || die 'record directory is missing or a symlink'
[[ "$image_prefix" =~ ^ghcr\.io/[a-z0-9][a-z0-9-]*/[a-z0-9][a-z0-9._-]*$ ]] || die 'invalid image prefix'

services=(gateway user-service novel-service agent-service narrative-service frontend)
shopt -s nullglob dotglob
records=("$record_dir"/*)
[[ ${#records[@]} -eq ${#services[@]} ]] || die 'expected exactly six image records'
images=()
for service in "${services[@]}"; do
  record="$record_dir/$service.txt"
  [[ -f "$record" && ! -L "$record" ]] || die "missing or non-regular record: $service"
  mapfile -t lines < "$record"
  [[ ${#lines[@]} -eq 1 ]] || die "expected one image reference: $service"
  image=${lines[0]}
  [[ "$image" == "$image_prefix-$service@sha256:"* ]] || die "wrong image identity: $service"
  digest=${image#*@sha256:}
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || die "invalid image digest: $service"
  images+=("$image")
done

# Emit only after every record passes; a partial manifest is never a result.
for index in "${!services[@]}"; do
  key=${services[$index]^^}
  printf '%s_IMAGE=%s\n' "${key//-/_}" "${images[$index]}"
done
