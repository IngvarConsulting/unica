"""Выбор тестов записи очереди: что идёт, что остаётся ночи, когда полный набор."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import unittest
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("queue_tests", ROOT / "scripts" / "ci" / "queue-tests.py")
assert SPEC is not None and SPEC.loader is not None
queue = importlib.util.module_from_spec(SPEC)
sys.modules["queue_tests"] = queue
SPEC.loader.exec_module(queue)

DIRS = {"crates/a": "unica-a", "crates/b": "unica-b", "crates/coder": "unica-coder", "vendor/ureq": "ureq"}
DEPENDENTS = {"unica-a": {"unica-b", "unica-coder"}, "unica-b": {"unica-coder"}}


def choose(files, **options):
    return queue.choose(files, DIRS, DEPENDENTS, **options)


class ChoiceTests(unittest.TestCase):
    def test_a_private_change_runs_only_its_crate(self) -> None:
        choice = choose({"crates/a/src/lib.rs": ["+    let x = 1;", "-pub(crate) fn y() {}"]})
        self.assertFalse(choice.full)
        self.assertEqual(choice.packages, ("unica-a",))

    def test_a_pub_line_adds_the_direct_dependents_not_the_transitive(self) -> None:
        self.assertEqual(choose({"crates/b/src/lib.rs": ["+pub fn y() {}"]}).packages, ("unica-b", "unica-coder"))
        self.assertEqual(choose({"crates/a/src/lib.rs": ["-pub struct S;"]}).packages, ("unica-a", "unica-b", "unica-coder"))

    def test_a_pub_line_in_the_tests_of_a_crate_does_not_widen(self) -> None:
        self.assertEqual(choose({"crates/a/tests/t.rs": ["+pub fn helper() {}"]}).packages, ("unica-a",))

    def test_unresolvable_paths_run_everything(self) -> None:
        for path in ("Cargo.lock", "Cargo.toml", "crates/a/Cargo.toml", "crates/a/build.rs", "rust-toolchain.toml",
                     ".config/nextest.toml", ".github/workflows/ci.yml", "scripts/ci/run-tests.py", ".cargo/config.toml"):
            with self.subTest(path=path):
                self.assertTrue(choose({path: [], "crates/a/src/lib.rs": []}).full)

    def test_a_path_outside_crates_runs_everything_unless_it_cannot_touch_rust(self) -> None:
        for path in ("plugins/unica/x.json", "tests/fixtures/a.xml", "arch/resource-limits.json"):
            with self.subTest(path=path):
                self.assertTrue(choose({path: []}).full)
        for path in ("README.md", "docs/pages/a.html", ".agents/skills/x/SKILL.md", "tests/ci/test_x.py", "arch/rules/a.md"):
            with self.subTest(path=path):
                choice = choose({path: []})
                self.assertFalse(choice.full)
                self.assertEqual(choice.packages, ())

    def test_an_empty_diff_and_the_switch_run_everything(self) -> None:
        self.assertTrue(choose({}).full)
        self.assertTrue(choose({"crates/a/src/lib.rs": []}, force_full=True).full)

    def test_the_diff_is_read_by_file(self) -> None:
        text = (
            "diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs\n--- a/crates/a/src/lib.rs\n+++ b/crates/a/src/lib.rs\n"
            "@@ -1 +1 @@\n-old\n+pub fn new() {}\n"
            "diff --git a/gone.md b/gone.md\n--- a/gone.md\n+++ /dev/null\n@@ -1 +0,0 @@\n-text\n"
        )
        files = queue.parse_diff(text)
        self.assertEqual(files["crates/a/src/lib.rs"], ["-old", "+pub fn new() {}"])
        self.assertEqual(files["gone.md"], ["-text"])

    def test_receipt_and_vendor_lanes_follow_their_crates(self) -> None:
        runner = mock.Mock(LEDGER_FEATURE_SELECTION=("ledger",), UREQ_RECYCLING_SELECTION=("ureq",))
        runner.rust_command.side_effect = lambda profile, selection: [profile, *selection]
        self.assertEqual(queue.commands(("unica-a",), runner), [["queue", "-p", "unica-a"]])
        self.assertEqual(queue.commands(("unica-coder", "ureq"), runner),
                         [["queue", "-p", "unica-coder", "-p", "ureq"], ["queue", "ledger"], ["queue", "ureq"]])
        self.assertEqual(queue.commands((), runner), [])

    def test_a_failed_diff_runs_everything(self) -> None:
        import tempfile
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "o")}):
            with mock.patch.object(queue.subprocess, "check_output", side_effect=OSError("no git")):
                self.assertEqual(queue.main(["--base", "a", "--head", "b", "--dry-run"]), 0)
            self.assertEqual((Path(directory) / "o").read_text(encoding="utf-8"), "full=true\n")


class WorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        job = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]["unica-ci"]
        self.steps = {step.get("name"): step for step in job["steps"]}

    def test_clippy_of_the_whole_workspace_always_runs(self) -> None:
        self.assertIn("--workspace --all-targets", self.steps["Clippy"]["run"])
        self.assertNotIn("if", self.steps["Clippy"])

    def test_the_queue_selects_rust_and_keeps_the_rollback_switch(self) -> None:
        rust = self.steps["Тесты Rust"]
        self.assertEqual(rust["id"], "selection")
        self.assertIn("python scripts/ci/queue-tests.py", rust["run"])
        self.assertEqual(rust["env"]["QUEUE_FULL_TESTS"], "${{ vars.QUEUE_FULL_TESTS }}")
        self.assertEqual(rust["env"]["BASE_SHA"], "${{ github.event.merge_group.base_sha }}")

    def test_python_medium_leaves_the_queue_unless_the_choice_is_full(self) -> None:
        run = self.steps["Тесты Python"]["run"]
        self.assertIn("python scripts/ci/run-static-tests.py --queue", run)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python', run)
        self.assertEqual(self.steps["Тесты Python"]["env"]["FULL"], "${{ steps.selection.outputs.full }}")


if __name__ == "__main__":
    unittest.main()
