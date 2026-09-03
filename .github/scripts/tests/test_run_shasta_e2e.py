import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
RUNNER = REPOSITORY_ROOT / ".github" / "scripts" / "run-shasta-e2e.sh"
DIAGNOSTICS = (
    REPOSITORY_ROOT / ".github" / "scripts" / "capture-nethermind-e2e-diagnostics.sh"
)
SMOKE_TESTS = (
    "test_avs_node.py::test_rpcs",
    "test_avs_node.py::test_preconfirm_transaction",
    "test_avs_node.py::test_p2p_preconfirmation",
)


class RunShastaE2ETests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.pytest_calls = self.root / "pytest-calls"
        self.timeout_calls = self.root / "timeout-calls"
        self.log_file = self.root / "pytest.log"

        self._write_executable(
            "timeout",
            """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$TIMEOUT_CALLS"
while [[ "$1" == --* ]]; do shift; done
shift
exec "$@"
""",
        )
        self._write_executable(
            "pytest",
            """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$PYTEST_CALLS"
call_number=$(wc -l < "$PYTEST_CALLS" | tr -d ' ')
echo "pytest invocation ${call_number}"
if [[ "${FAKE_PYTEST_FAIL_ON_CALL:-}" == "$call_number" ]]; then
  exit 7
fi
""",
        )

    def _write_executable(self, name, contents):
        path = self.bin_dir / name
        path.write_text(contents)
        path.chmod(0o755)

    def _run(self, *, fail_on_call=None, timeout="20m"):
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{self.bin_dir}:{env['PATH']}",
                "PYTEST_CALLS": str(self.pytest_calls),
                "TIMEOUT_CALLS": str(self.timeout_calls),
                "PYTEST_LOG_FILE": str(self.log_file),
                "SHASTA_SMOKE_TIMEOUT": timeout,
            }
        )
        if fail_on_call is not None:
            env["FAKE_PYTEST_FAIL_ON_CALL"] = str(fail_on_call)

        return subprocess.run(
            ["bash", str(RUNNER)],
            cwd=REPOSITORY_ROOT / "e2e_tests",
            env=env,
            capture_output=True,
            text=True,
        )

    def test_failed_smoke_gate_stops_before_the_remaining_suite(self):
        result = self._run(fail_on_call=1)

        self.assertEqual(result.returncode, 7)
        self.assertIn("Shasta smoke gate failed", result.stdout)
        self.assertEqual(len(self.pytest_calls.read_text().splitlines()), 1)

    def test_successful_smoke_gate_runs_the_remaining_tests_without_duplicates(self):
        result = self._run()

        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.pytest_calls.read_text().splitlines()
        self.assertEqual(len(calls), 2)
        for test_name in SMOKE_TESTS:
            self.assertIn(test_name, calls[0])
            self.assertIn(f"--deselect={test_name}", calls[1])
        self.assertIn("--maxfail=1", calls[0])

    def test_smoke_gate_is_bounded_and_all_pytest_output_is_logged(self):
        result = self._run(timeout="17m")

        self.assertEqual(result.returncode, 0, result.stderr)
        timeout_call = self.timeout_calls.read_text()
        self.assertIn("--signal=TERM", timeout_call)
        self.assertIn("--kill-after=30s", timeout_call)
        self.assertIn("17m pytest", timeout_call)
        log_lines = self.log_file.read_text().splitlines()
        self.assertIn("pytest invocation 1", log_lines)
        self.assertIn("pytest invocation 2", log_lines)


class CaptureNethermindE2EDiagnosticsTests(unittest.TestCase):
    def test_collects_relevant_service_logs_without_masking_the_primary_failure(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            docker_calls = root / "docker-calls"
            docker = bin_dir / "docker"
            docker.write_text(
                """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$DOCKER_CALLS"
echo "docker output: $*"
if [[ "$*" == "compose logs"* ]]; then exit 9; fi
"""
            )
            docker.chmod(0o755)
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{bin_dir}:{env['PATH']}",
                    "DOCKER_CALLS": str(docker_calls),
                    "DIAGNOSTICS_OUTPUT_DIR": str(root),
                }
            )

            result = subprocess.run(
                ["bash", str(DIAGNOSTICS)],
                cwd=REPOSITORY_ROOT,
                env=env,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            calls = docker_calls.read_text().splitlines()
            self.assertEqual(calls[0], "ps -a")
            self.assertEqual(calls[1], "compose ps -a")
            self.assertTrue(calls[2].startswith("compose logs --no-color --timestamps"))
            for service in (
                "catalyst-node-1",
                "catalyst-node-2",
                "taiko-client-go-1",
                "taiko-client-go-2",
                "taiko-nethermind-1",
                "taiko-nethermind-2",
                "web3signer_l1",
                "web3signer_l2",
                "transfer-funds",
                "p2p-bootnode",
            ):
                self.assertIn(service, calls[2])
            diagnostics = (root / "nethermind-e2e-diagnostics.log").read_text()
            self.assertIn("docker output: ps -a", diagnostics)
            self.assertIn("docker output: compose logs", diagnostics)


if __name__ == "__main__":
    unittest.main()
