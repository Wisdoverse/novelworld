#!/usr/bin/env bash
# SBOM generation (H2 supply-chain; see SECURITY.md 'Dependency Policy').
# Emits one CycloneDX 1.7 JSON SBOM per application image with the pinned
# trivy release, plus a digests.txt sidecar binding each SBOM to the
# sha256 digest (registry digest, or the content-addressed image id for
# locally built images) it describes. The release pipeline supplies exact
# build digests; never re-resolve them through a tag or RepoDigests ordering.
# Registry pulls have a 10-minute deadline and scans a 15-minute deadline,
# with 30 seconds of termination grace and no automatic retry. Terminating
# the client does not prove Docker has stopped all daemon-side work.
#
# Release-file provenance/attestation is implemented in the release workflow;
# deploy-time SBOM admission and platform-native signing remain open.
#
# Usage: infra/security/generate-sboms.sh [OUTPUT_DIR] [IMAGE ...]
set -euo pipefail
cd "$(dirname "$0")/../.."

out_dir=${1:-sboms}
out_dir=$(realpath "$out_dir")
shift 2>/dev/null || true
images=("$@")
if [ "${#images[@]}" -eq 0 ]; then
  images=(
    novel-world-gateway:local
    novel-world-user-service:local
    novel-world-novel-service:local
    novel-world-agent-service:local
    novel-world-narrative-service:local
    novel-world-frontend:local
  )
fi

mkdir -p "$out_dir"
: >"$out_dir/digests.txt"

for image in "${images[@]}"; do
  service=${image%@*}
  service=${service##*/}
  service=${service%%:*}
  service=${service#novel-world-}
  service=${service#novelworld-}
  if [[ "$image" == *@* ]]; then
    [[ "$image" =~ ^[a-z0-9][a-z0-9._/:-]*@sha256:[0-9a-f]{64}$ ]] \
      || { printf 'sbom: invalid registry digest reference\n' >&2; exit 1; }
    digest=${image##*@}
    timeout --kill-after=30s 600s docker pull "$image" >/dev/null
  else
    digest=$(docker inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$image" \
      | head -1 | sed 's|.*@||')
    if [ -z "$digest" ]; then
      # Local-only mode has no registry digest. Keep its content-addressed
      # image-ID binding distinct from the release pipeline's registry proof.
      digest=$(docker inspect --format '{{.Id}}' "$image")
    fi
  fi
  [ -n "$digest" ] || { printf 'sbom: no digest for %s\n' "$image" >&2; exit 1; }
  printf 'sbom: %s %s\n' "$service" "$digest"
  timeout --kill-after=30s 900s docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$out_dir:/out" \
    aquasec/trivy:0.74.0@sha256:62b1e65e8869bc4b4c6aa4fa2b21595256c7c2f6018a9d9ad61caf87187c1969 image --scanners vuln --format cyclonedx \
    --skip-version-check --output "/out/$service.cdx.json" "$image" >/dev/null 2>&1
  printf '%s %s\n' "$service" "$digest" >>"$out_dir/digests.txt"
done

printf 'sbom: wrote %s SBOMs to %s\n' "${#images[@]}" "$out_dir"
