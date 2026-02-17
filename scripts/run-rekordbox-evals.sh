#!/usr/bin/env bash

set -u -o pipefail

ROUTING_PASS_THRESHOLD="${ROUTING_PASS_THRESHOLD:-100}"
TASK_PASS_THRESHOLD="${TASK_PASS_THRESHOLD:-100}"
ROUTING_MIN_TESTS="${ROUTING_MIN_TESTS:-1}"
TASK_MIN_TESTS="${TASK_MIN_TESTS:-1}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required but was not found in PATH." >&2
  exit 1
fi

LAST_SUMMARY=""

run_suite() {
  local label="$1"
  local pass_threshold="$2"
  local min_tests="$3"
  shift 3
  local -a cmd=("$@")

  local log_file
  log_file="$(mktemp -t "${label}.XXXXXX.log")"

  echo
  echo "=== ${label} ==="
  echo "+ ${cmd[*]}"
  "${cmd[@]}" 2>&1 | tee "${log_file}"
  local cargo_status=${PIPESTATUS[0]}

  local result_line
  result_line="$(grep -E "test result:" "${log_file}" | tail -n 1 || true)"

  local passed
  passed="$(echo "${result_line}" | sed -nE "s/.* ([0-9]+) passed.*/\1/p")"
  local failed
  failed="$(echo "${result_line}" | sed -nE "s/.* ([0-9]+) failed.*/\1/p")"

  if [[ -z "${passed}" ]]; then
    passed=0
  fi
  if [[ -z "${failed}" ]]; then
    failed=0
  fi

  local total
  total=$((passed + failed))

  local pass_rate
  pass_rate="$(awk -v passed="${passed}" -v total="${total}" "BEGIN { if (total == 0) printf \"0.0\"; else printf \"%.1f\", (passed * 100.0) / total }")"

  local pass_rate_ok
  pass_rate_ok="$(awk -v rate="${pass_rate}" -v threshold="${pass_threshold}" "BEGIN { if (rate + 0 >= threshold + 0) print 1; else print 0 }")"

  local min_tests_ok=0
  if [[ "${total}" -ge "${min_tests}" ]]; then
    min_tests_ok=1
  fi

  local suite_status="PASS"
  if [[ "${cargo_status}" -ne 0 || "${failed}" -gt 0 || "${pass_rate_ok}" -ne 1 || "${min_tests_ok}" -ne 1 ]]; then
    suite_status="FAIL"
  fi

  LAST_SUMMARY="${label}: ${suite_status} (passed=${passed}, failed=${failed}, total=${total}, pass_rate=${pass_rate}%, threshold>=${pass_threshold}%, min_tests>=${min_tests})"
  echo "${LAST_SUMMARY}"

  rm -f "${log_file}"

  if [[ "${suite_status}" == "PASS" ]]; then
    return 0
  fi
  return 1
}

routing_cmd=(cargo test eval_routing -- --nocapture)
task_cmd=(cargo test eval_tasks -- --nocapture)

if [[ -f "tests/tool_routing.rs" ]]; then
  routing_cmd=(cargo test --test tool_routing -- --nocapture)
fi

if [[ -f "tests/task_eval.rs" ]]; then
  task_cmd=(cargo test --test task_eval -- --nocapture)
fi

run_suite "routing-evals" "${ROUTING_PASS_THRESHOLD}" "${ROUTING_MIN_TESTS}" "${routing_cmd[@]}"
routing_status=$?
routing_summary="${LAST_SUMMARY}"

run_suite "task-evals" "${TASK_PASS_THRESHOLD}" "${TASK_MIN_TESTS}" "${task_cmd[@]}"
task_status=$?
task_summary="${LAST_SUMMARY}"

echo
echo "=== Rekordbox Eval Threshold Summary ==="
echo "${routing_summary}"
echo "${task_summary}"

if [[ "${routing_status}" -ne 0 || "${task_status}" -ne 0 ]]; then
  echo "Overall: FAIL"
  exit 1
fi

echo "Overall: PASS"
