#!/usr/bin/env bash
# Native release-file provenance check, including content/source rejection cases.
# Requires GitHub CLI 2.100.0, GNU timeout and an independently obtained trusted root.
set -euo pipefail
[[ $# -eq 3 ]] || { echo "Usage: $0 RELEASE_DIR EXPECTED_SHA TRUSTED_ROOT" >&2; exit 2; }
release_dir=$(realpath "$1")
expected_sha=$2
trusted_root=$(realpath "$3")
[[ "$expected_sha" =~ ^[0-9a-f]{40}$ ]]
[[ -d "$release_dir" && -s "$trusted_root" ]]
files=(
  release.env
  gateway.cdx.json user-service.cdx.json novel-service.cdx.json
  agent-service.cdx.json narrative-service.cdx.json frontend.cdx.json
  digests.txt desktop-SHA256SUMS
  novelworld-windows-x64-portable.zip
  novelworld-linux-x64-appimage.tar.gz
  novelworld-macos-arm64-app.zip novelworld-macos-x64-app.zip
)
shopt -s nullglob dotglob
entries=("$release_dir"/*)
[[ ${#entries[@]} -eq $((${#files[@]} + 1)) ]]
for file in "${files[@]}" release-attestation.json; do
  [[ -f "$release_dir/$file" && -s "$release_dir/$file" && ! -L "$release_dir/$file" ]]
done

verify() {
  timeout --kill-after=10s 120s gh attestation verify "$1" \
    --hostname github.com \
    --repo Wisdoverse/novelworld \
    --signer-workflow Wisdoverse/novelworld/.github/workflows/docker.yml \
    --source-digest "$2" --signer-digest "$expected_sha" \
    --deny-self-hosted-runners \
    --bundle "$release_dir/release-attestation.json" \
    --custom-trusted-root "$trusted_root"
}
for file in "${files[@]}"; do
  verify "$release_dir/$file" "$expected_sha"
done

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
cp "$release_dir/release.env" "$scratch/release.env"
printf '\n# tampered after signing\n' >> "$scratch/release.env"
reject() {
  local path=$1 source=$2 reason=$3 status=0
  verify "$path" "$source" > "$scratch/rejection.log" 2>&1 || status=$?
  cat "$scratch/rejection.log"
  # A timeout, tool error or unrelated policy failure is not a passing negative case.
  [[ "$status" -eq 1 ]]
  grep -Fq "$reason" "$scratch/rejection.log"
}
reject "$scratch/release.env" "$expected_sha" \
  'provided artifact digest does not match any digest in statement'
wrong_sha=0000000000000000000000000000000000000000
[[ "$wrong_sha" != "$expected_sha" ]] || wrong_sha=1111111111111111111111111111111111111111
reject "$release_dir/release.env" "$wrong_sha" \
  "expected SourceRepositoryDigest to be $wrong_sha, got $expected_sha"
echo "Verified all ${#files[@]} release files; native content and source rejection cases passed."
