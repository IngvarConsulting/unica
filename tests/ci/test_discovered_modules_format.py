"""One format command reaches ordinary, path, and build-discovered Rust modules."""
from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "discovered_modules_format", ROOT / "scripts/ci/check-discovered-modules-format.py"
)
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)

INCLUDE = 'include!(concat!(env!("OUT_DIR"), "/a_modules.rs"));\n'


class DiscoveredModulesFormatTests(unittest.TestCase):
    def write(self, root: Path, files: dict[str, str]) -> None:
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

    def test_tracked_sources_cover_both_reported_paths(self) -> None:
        sources = checker.tracked_sources(ROOT)
        self.assertIn("crates/unica-metadata-designer/src/extension.rs", sources)
        self.assertIn("crates/unica-coder/src/infrastructure/v13_read/tests/configuration_root.rs", sources)
        self.assertIn("crates/unica-metadata-designer/src/lib.rs", sources)
        self.assertIn("crates/unica-daemon/src/infrastructure/daemon/metadata_can_tests.rs", sources)
        for directory in (
            "crates/unica-daemon/src/infrastructure/daemon/metadata_can_tests/",
            "crates/unica-coder/src/infrastructure/v13_read_port/",
            "crates/unica-metadata/src/properties/",
        ):
            with self.subTest(directory=directory):
                self.assertTrue(any(source.startswith(directory) for source in sources))

    def test_every_batch_runs_even_after_an_earlier_failure(self) -> None:
        sources = [f"crates/a/src/module_{index:03}.rs" for index in range(130)]
        with mock.patch.object(checker, "tracked_sources", return_value=sources), \
                mock.patch.object(checker.subprocess, "run") as run:
            run.side_effect = [subprocess.CompletedProcess([], code) for code in (1, 0, 1)]
            self.assertEqual(checker.main(), 1)
        selected = [name for call in run.call_args_list for name in call.args[0][6:]]
        self.assertEqual(selected, sources)

    def test_one_command_checks_path_ordinary_and_discovered_modules(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {
                "crates/a/src/lib.rs": 'mod ordinary;\n#[path = "alternate/path module.rs"]\nmod special;\n' + INCLUDE,
                "crates/a/src/ordinary.rs": "pub fn ordinary() {}\n",
                "crates/a/src/alternate/path module.rs": "pub fn special() {}\n",
                "crates/a/build.rs": "fn main() {}\n",
                "crates/a/src/generated.rs": "pub fn discovered() {}\n",
                "crates/a/tests/platform/configuration_root.rs": "pub fn configuration() {}\n",
                "crates/a/tests/platform.rs": '#[path = "platform/configuration_root.rs"]\nmod configuration_root;\n',
            }
            self.write(root, files)
            script = root / "scripts/ci/check-discovered-modules-format.py"
            script.parent.mkdir(parents=True)
            shutil.copyfile(ROOT / "scripts/ci/check-discovered-modules-format.py", script)
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            subprocess.run(["git", "add", "crates"], cwd=root, check=True)

            def check():
                return subprocess.run([sys.executable, str(script)], cwd=root,
                                      capture_output=True, text=True)

            clean = check()
            self.assertEqual(clean.returncode, 0, clean.stdout + clean.stderr)
            for name in ("crates/a/src/alternate/path module.rs",
                         "crates/a/tests/platform/configuration_root.rs",
                         "crates/a/src/ordinary.rs", "crates/a/src/generated.rs",
                         "crates/a/src/lib.rs", "crates/a/build.rs"):
                with self.subTest(name=name):
                    (root / name).write_text("pub fn broken( ){let a=1;}\n", encoding="utf-8")
                    broken = check()
                    self.assertNotEqual(broken.returncode, 0)
                    self.assertIn(name, (broken.stdout + broken.stderr).replace("\\", "/"))
                    (root / name).write_text(files[name], encoding="utf-8")
                    fixed = check()
                    self.assertEqual(fixed.returncode, 0, fixed.stdout + fixed.stderr)

    def test_both_format_gates_run_the_pass(self) -> None:
        import yaml
        for workflow in ("ci.yml", "release.yml"):
            with self.subTest(workflow):
                data = yaml.safe_load((ROOT / ".github/workflows" / workflow).read_text(encoding="utf-8"))
                steps = [step for job in data["jobs"].values() for step in job.get("steps", [])]
                format_steps = [step for step in steps if step.get("name") == "Формат"
                                or step.get("id") == "guards-fmt"]
                self.assertEqual(len(format_steps), 1)
                with tempfile.TemporaryDirectory() as temporary:
                    calls = Path(temporary) / "calls"
                    # Execute the workflow step with tools that only record calls.
                    shell = ('python() { printf "python %s\\n" "$*" >> "$FORMAT_CALLS"; }\n'
                             'cargo() { printf "cargo %s\\n" "$*" >> "$FORMAT_CALLS"; return 99; }\n')
                    completed = subprocess.run(
                        ["bash", "-eu", "-c", shell + format_steps[0]["run"]],
                        env={**os.environ, "FORMAT_CALLS": str(calls)}, capture_output=True, text=True
                    )
                    self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
                    self.assertEqual(calls.read_text().splitlines(),
                                     ["python scripts/ci/check-discovered-modules-format.py"])


if __name__ == "__main__":
    unittest.main()
