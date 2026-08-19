#!/usr/bin/env bash
# Container image vulnerability scan (H2 supply-chain; see SECURITY.md
# 'Dependency Policy'). Scans the application images with the pinned trivy
# release for HIGH/CRITICAL vulnerabilities; any finding exits 1. The CI
# tag pipeline (docker.yml) runs the same check automatically on every
# pushed image, so this script is the local convenience form of the gate.
#
# The digest-pinned infrastructure images (postgres/redis/nginx) are NOT
# scanned here: they have no local Dockerfile to remediate, and re-pinning
# them belongs to the separately approved infrastructure procedure
# (release.sh requires same-infrastructure between releases).
#
# Usage: infra/security/scan-images.sh [IMAGE ...]
set -euo pipefail
cd "$(dirname "$0")/../.."

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

for image in "${images[@]}"; do
  printf 'scan: %s\n' "$image"
  docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
    aquasec/trivy:0.68.1 image --scanners vuln \
    --severity HIGH,CRITICAL --ignore-unfixed --exit-code 1 \
    --skip-version-check "$image"
done
printf 'scan: all images clean\n'
