#!/usr/bin/env bash

set -uo pipefail

readonly output_dir="${DIAGNOSTICS_OUTPUT_DIR:-../e2e_tests}"
readonly diagnostics_log="${output_dir}/${DIAGNOSTICS_LOG_NAME:-nethermind-e2e-diagnostics.log}"
readonly tail_lines="${DIAGNOSTICS_TAIL_LINES:-2000}"
readonly default_services=(
  shasta-deployer
  catalyst-node-1
  catalyst-node-2
  taiko-client-go-1
  taiko-client-go-2
  taiko-nethermind-1
  taiko-nethermind-2
  web3signer_l1
  web3signer_l2
  transfer-funds
  p2p-bootnode
)

if (( $# > 0 )); then
  services=("$@")
else
  services=("${default_services[@]}")
fi
readonly -a services

mkdir -p "$output_dir"
: > "$diagnostics_log"

capture() {
  local command_status

  echo "+ $*" | tee -a "$diagnostics_log"
  "$@" 2>&1 | tee -a "$diagnostics_log"
  command_status=${PIPESTATUS[0]}
  if (( command_status != 0 )); then
    echo "Diagnostic command exited with ${command_status}; continuing" \
      | tee -a "$diagnostics_log"
  fi
}

capture docker ps -a
capture docker compose ps -a
for service in "${services[@]}"; do
  capture docker compose logs --no-color --timestamps --tail "$tail_lines" "$service"
done

exit 0
