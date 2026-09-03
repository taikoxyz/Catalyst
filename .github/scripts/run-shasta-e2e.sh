#!/usr/bin/env bash

set -uo pipefail

readonly pytest_log_file="${PYTEST_LOG_FILE:-pytest-shasta.log}"
readonly smoke_timeout="${SHASTA_SMOKE_TIMEOUT:-20m}"
readonly pytest_options=(
  -v
  --capture=tee-sys
  -r a
  --tb=long
)
readonly smoke_tests=(
  test_avs_node.py::test_rpcs
  test_avs_node.py::test_preconfirm_transaction
  test_avs_node.py::test_p2p_preconfirmation
)

: > "$pytest_log_file"

run_and_log() {
  "$@" 2>&1 | tee -a "$pytest_log_file"
  local -a pipeline_status=("${PIPESTATUS[@]}")
  if (( pipeline_status[0] != 0 )); then
    return "${pipeline_status[0]}"
  fi
  return "${pipeline_status[1]}"
}

echo "Running Shasta block-production smoke gate"
run_and_log \
  timeout --signal=TERM --kill-after=30s "$smoke_timeout" \
  pytest "${pytest_options[@]}" --maxfail=1 "${smoke_tests[@]}"
smoke_status=$?

if (( smoke_status != 0 )); then
  if (( smoke_status == 124 )); then
    echo "Shasta smoke gate timed out after ${smoke_timeout}" | tee -a "$pytest_log_file"
  else
    echo "Shasta smoke gate failed with exit code ${smoke_status}" | tee -a "$pytest_log_file"
  fi
  exit "$smoke_status"
fi

echo "Running remaining Shasta E2E tests" | tee -a "$pytest_log_file"
remaining_test_options=()
for test_name in "${smoke_tests[@]}"; do
  remaining_test_options+=("--deselect=${test_name}")
done

run_and_log pytest "${pytest_options[@]}" "${remaining_test_options[@]}"
exit $?
