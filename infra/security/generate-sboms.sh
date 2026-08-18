#!/usr/bin/env bash
# SBOM generation (H2 supply-chain; see SECURITY.md 'Dependency Policy').
# Emits one CycloneDX 1.6 JSON SBOM per application image with the pinned
# trivy release, plus a digests.txt sidecar binding each SBOM to the
# sha256 digest (registry digest, or the content-addressed image id for
# locally built images) it describes. The release pipeline (docker.yml)
# runs equivalent generation against the pushed registry images.
#
# Deploy-time SBOM verification, provenance/attestation, and signing remain
# open release-infrastructure work and are recorded as such.
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
  service=${image##*/}
  service=${service%%:*}
  service=${service#novel-world-}
  digest=$(docker inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$image" \
    | head -1 | sed 's|.*@||')
  if [ -z "$digest" ]; then
    # Locally built images have no registry digest; the content-addressed
    # image id (kept sha256-prefixed) is the equivalent binding.
    digest=$(docker inspect --format '{{.Id}}' "$image")
  fi
  [ -n "$digest" ] || { printf 'sbom: no digest for %s\n' "$image" >&2; exit 1; }
  printf 'sbom: %s %s\n' "$service" "$digest"
  docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$out_dir:/out" \
    aquasec/trivy:0.68.1 image --scanners vuln --format cyclonedx \
    --skip-version-check --output "/out/$service.cdx.json" "$image" >/dev/null 2>&1
  printf '%s %s\n' "$service" "$digest" >>"$out_dir/digests.txt"
done

printf 'sbom: wrote %s SBOMs to %s\n' "${#images[@]}" "$out_dir"
