"""The Clippy file-IO pass proves its refusals on a probe, then checks the bounded packages (#1894)."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]


def load(name: str):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO_ROOT / "scripts/ci" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


methods = load("check-file-io-methods")


def message(file: str, line: int, method: str, code: str = methods.LINT) -> str:
    return json.dumps({"reason": "compiler-message", "message": {
        "code": {"code": code}, "message": f"use of a disallowed method `{method}`",
        "spans": [{"is_primary": False, "file_name": "elsewhere.rs", "line_start": 1},
                  {"is_primary": True, "file_name": file, "line_start": line}]}})


class FileIoMethodsTests(unittest.TestCase):
    def test_probe_marks_every_disallowed_method_once(self):
        configured = methods.disallowed(REPO_ROOT)
        marked = list(methods.probe_markers(REPO_ROOT).values())
        self.assertEqual(sorted(marked), sorted(configured))
        self.assertEqual(len(set(marked)), len(marked))
        # Unit tests inside `src/` are in scope, so the probe calls some under #[cfg(test)].
        source = (REPO_ROOT / methods.PROBE / "src/lib.rs").read_text()
        self.assertIn("probe:", source[source.index("#[cfg(test)]"):])

    def test_command_builds_every_cfg_and_feature_and_only_this_lint(self):
        # Plain and cfg(test) library, every feature: `test-support` code is `src/` too.
        command = methods.clippy_command(["-p", "unica-address"], Path("/t"))
        self.assertIn("--all-targets", command)
        self.assertIn("--all-features", command)
        self.assertNotIn("--profile", command)
        self.assertEqual(command[command.index("--") + 1:], ["-A", "clippy::all", "-W", methods.LINT])

    def run_probe(self, stdout: str, returncode: int = 0):
        completed = subprocess.CompletedProcess([], returncode, stdout=stdout)
        with patch.object(methods, "run", return_value=completed) as run:
            problems = methods.check_probe(REPO_ROOT, Path("/t"))
        self.assertEqual(run.call_args.args[1], REPO_ROOT / methods.PROBE)
        return problems

    def test_probe_must_have_every_marked_call_refused(self):
        markers = methods.probe_markers(REPO_ROOT)
        refused = "\n".join(message(file, line, method) for (file, line), method in markers.items())
        self.assertEqual(self.run_probe(refused), [])
        (file, line), method = next(iter(markers.items()))
        partial = "\n".join(message(f, n, m) for (f, n), m in markers.items() if (f, n) != (file, line))
        self.assertEqual(self.run_probe(partial), [f"{file}:{line}: {method} was not refused"])
        # A config or wiring that refuses nothing is red, not silently green.
        self.assertEqual(len(self.run_probe("not json\n" + message(file, line, method, code="clippy::other"))), len(markers))
        self.assertIn("probe Clippy failed with exit code 101", self.run_probe(refused, 101))

    def test_packages_report_only_their_own_src_once(self):
        packages = {"unica-address": "crates/unica-address", "unica-forms-designer": "crates/unica-forms-designer"}
        stdout = "\n".join([
            message("crates/unica-address/src/lib.rs", 9, "std::path::Path::exists"),
            message("crates/unica-address/src/lib.rs", 9, "std::path::Path::exists"),
            message("crates/unica-workspace/src/discovery.rs", 68, "std::fs::File::open"),
            message("crates/unica-forms-designer/tests/io.rs", 3, "std::fs::File::open"),
        ])
        with patch.object(methods, "run", return_value=subprocess.CompletedProcess([], 0, stdout=stdout)) as run:
            problems = methods.check_packages(REPO_ROOT, packages, Path("/t"))
        self.assertEqual(problems, ["crates/unica-address/src/lib.rs:9: use of a disallowed method `std::path::Path::exists`"])
        command = run.call_args.args[0]
        self.assertEqual([command[i + 1] for i, arg in enumerate(command) if arg == "-p"], sorted(packages))
        with patch.object(methods, "run", return_value=subprocess.CompletedProcess([], 101, stdout="")):
            self.assertEqual(methods.check_packages(REPO_ROOT, packages, Path("/t")), ["clippy failed with exit code 101"])
        self.assertEqual(methods.check_packages(REPO_ROOT, {}, Path("/t")), ["no file-IO-free packages to check"])

    def test_packages_come_from_the_platform_boundary(self):
        packages = methods.boundary_packages(REPO_ROOT)
        self.assertEqual(packages["unica-address"], "crates/unica-address")
        self.assertNotIn("unica-workspace", packages)

    def test_lint_job_runs_the_pass(self):
        workflow = yaml.safe_load((REPO_ROOT / ".github/workflows/unica-plugin-release.yml").read_text())
        steps = workflow["jobs"]["lint-rust-platforms"]["steps"]
        runs = [step.get("run", "") for step in steps]
        self.assertTrue(any("scripts/ci/check-file-io-methods.py" in run for run in runs))


if __name__ == "__main__":
    unittest.main()
