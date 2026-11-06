"""Тесты записи очереди: весь small, medium изменённых крейтов, когда полный набор."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
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


def choose(paths, found=(), **options):
    return queue.choose(paths, DIRS, find=lambda text: list(found), **options)


class ChoiceTests(unittest.TestCase):
    def test_any_file_of_a_crate_selects_that_crate_and_no_dependents(self) -> None:
        for path in ("crates/a/src/lib.rs", "crates/a/tests/t.rs", "crates/a/build.rs", "crates/a/Cargo.toml", "crates/a/README.md"):
            with self.subTest(path=path):
                choice = choose([path])
                self.assertFalse(choice.full)
                self.assertEqual(choice.packages, ("unica-a",))

    def test_unica_coder_is_selected_like_any_other_crate(self) -> None:
        self.assertEqual(choose(["crates/coder/src/lib.rs", "crates/a/src/lib.rs"]).packages, ("unica-a", "unica-coder"))

    def test_only_the_whole_build_runs_everything(self) -> None:
        for path in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            with self.subTest(path=path):
                self.assertTrue(choose([path, "crates/a/src/lib.rs"]).full)

    def test_paths_rust_tests_read_select_the_crates_that_mention_them(self) -> None:
        for path in ("plugins/unica/references/x.md", "tests/fixtures/xdto/a.xml", "scripts/dev/x.py"):
            with self.subTest(path=path):
                self.assertEqual(choose([path], ["crates/b/tests/t.rs"]).packages, ("unica-b",))

    def test_the_search_is_for_the_data_set_not_the_single_file(self) -> None:
        self.assertEqual(queue.needle("plugins/unica/references/specs/x.md"), "plugins/unica")
        self.assertEqual(queue.needle("tests/fixtures/xdto/deep/a.xml"), "tests/fixtures/xdto")
        self.assertEqual(queue.needle("scripts/dev/x.py"), "scripts/dev/x.py")

    def test_everything_else_runs_only_small(self) -> None:
        for path in (".github/workflows/ci.yml", ".github/CODEOWNERS", "scripts/ci/run-tests.py", ".config/nextest.toml", ".config/python-sizes.toml",
                     "tests/fixtures/acceptance/scenario-manifest/x.json", "tests/ci/test_x.py", "docs/a.md", "README.md", ".gitattributes"):
            with self.subTest(path=path):
                choice = choose([path], ["crates/b/tests/t.rs"])
                self.assertFalse(choice.full)
                self.assertEqual(choice.packages, ())

    def test_an_empty_diff_and_the_switch_run_everything(self) -> None:
        self.assertTrue(choose([]).full)
        self.assertTrue(choose(["crates/a/src/lib.rs"], force_full=True).full)

    def test_small_runs_everywhere_and_selected_crates_are_not_run_twice(self) -> None:
        runner = mock.Mock(LEDGER_FEATURE_SELECTION=("ledger",), UREQ_RECYCLING_SELECTION=("ureq",))
        runner.rust_command.side_effect = lambda profile, selection: [profile, *selection]
        self.assertEqual(queue.commands((), runner), [["pr", "--workspace", "--no-tests=pass"]])
        self.assertEqual(queue.commands(("unica-a",), runner), [
            ["pr", "--workspace", "--exclude", "unica-a", "--no-tests=pass"],
            ["queue", "-p", "unica-a", "--no-tests=pass"],
        ])
        planned = queue.commands(("unica-coder", "ureq"), runner)
        self.assertEqual(planned[2:], [["queue", "ledger"], ["queue", "ureq"]])

    def test_a_failed_diff_runs_everything(self) -> None:
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "o")}):
            with mock.patch.object(queue.subprocess, "check_output", side_effect=OSError("no git")):
                self.assertEqual(queue.main(["--base", "a", "--head", "b", "--dry-run"]), 0)
            self.assertEqual((Path(directory) / "o").read_text(encoding="utf-8"), "full=true\n")


class GuardTests(unittest.TestCase):
    """Правило держится на том, что Rust читает эти пути только прямым упоминанием."""

    def grep(self, text: str) -> list[str]:
        return subprocess.run(["git", "grep", "-lF", text, "--", "crates/*.rs", "vendor/*.rs"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8").stdout.split()

    def test_rust_does_not_read_the_paths_that_run_only_small(self) -> None:
        for text in ("scenario-manifest", "python-sizes.toml", "CODEOWNERS", "scripts/ci/", ".github/workflows", ".agents/", '"tests/ci/', '"tests/dev/', '"docs/', "../docs/"):
            with self.subTest(text=text):
                self.assertEqual(self.grep(text), [])

    def test_plugins_and_fixtures_are_really_found_by_mention(self) -> None:
        self.assertTrue(self.grep("plugins/unica"))
        self.assertTrue(self.grep("tests/fixtures/xdto"))

    def test_rust_builds_fixture_paths_only_from_a_named_data_set(self) -> None:
        # Склейка из одного `tests/fixtures` спрятала бы каталог от поиска.
        self.assertEqual(self.grep('"tests/fixtures"'), [])


class WorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        jobs = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]
        self.steps = {step.get("name"): step for step in jobs["unica-ci"]["steps"]}
        self.windows = jobs["windows-clippy"]

    def test_clippy_of_the_whole_workspace_runs_on_linux_and_windows(self) -> None:
        self.assertIn("--workspace --all-targets", self.steps["Clippy"]["run"])
        self.assertEqual(self.steps["Clippy"]["if"], "env.KEPT != 'true'")
        self.assertEqual(self.windows["if"], "github.event_name == 'merge_group'")

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
