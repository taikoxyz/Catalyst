#!/usr/bin/env bash

set -euo pipefail

readonly actionlint_version="1.7.12"
readonly install_dir="${RUNNER_TEMP:?RUNNER_TEMP must be set}/actionlint"

if [[ "${RUNNER_OS:?RUNNER_OS must be set}" != "Linux" ]]; then
  echo "actionlint installer only supports Linux runners" >&2
  exit 1
fi

case "${RUNNER_ARCH:?RUNNER_ARCH must be set}" in
  # Refresh these digests with:
  # gh api repos/rhysd/actionlint/releases/tags/v1.7.12 \
  #   --jq '.assets[] | select(.name | test("linux_(386|amd64|arm64|armv6)")) | [.name, .digest] | @tsv'
  X64)
    actionlint_arch="amd64"
    expected_sha256="8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"
    ;;
  ARM64)
    actionlint_arch="arm64"
    expected_sha256="325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6"
    ;;
  X86)
    actionlint_arch="386"
    expected_sha256="72a44b32c2d032700e6d0c23ca2f540b67519ec68db098ddfcfa96059e61f723"
    ;;
  ARM)
    actionlint_arch="armv6"
    expected_sha256="ae4a0a5227578e66f5d00ee02788d5c64fdae1fa6484ab88ceaeee9359c28fa4"
    ;;
  *)
    echo "unsupported runner architecture: ${RUNNER_ARCH}" >&2
    exit 1
    ;;
esac

readonly actionlint_arch expected_sha256
readonly archive="actionlint_${actionlint_version}_linux_${actionlint_arch}.tar.gz"
readonly archive_path="${RUNNER_TEMP}/${archive}"
readonly download_url="https://github.com/rhysd/actionlint/releases/download/v${actionlint_version}/${archive}"
readonly executable="${install_dir}/actionlint"

mkdir -p "$install_dir"
trap 'rm -f "$archive_path"' EXIT

curl --fail --silent --show-error --location \
  --retry 3 --retry-delay 2 --retry-max-time 300 \
  --connect-timeout 10 --max-time 120 \
  --output "$archive_path" "$download_url"
printf '%s  %s\n' "$expected_sha256" "$archive_path" | sha256sum --check -
tar -xzf "$archive_path" -C "$install_dir" actionlint

if [[ ! -x "$executable" ]]; then
  echo "actionlint executable was not extracted" >&2
  exit 1
fi

printf 'executable=%s\n' "$executable" >> "${GITHUB_OUTPUT:?GITHUB_OUTPUT must be set}"
