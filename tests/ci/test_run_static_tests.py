"""Уровень PR: быстрые Python-проверки и что снято с PR."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("run_static_tests", ROOT / "scripts" / "ci" / "run-static-tests.py")
assert SPEC is not None and SPEC.loader is not None
static = importlib.util.module_from_spec(SPEC)
sys.modules["run_static_tests"] = static
SPEC.loader.exec_module(static)


class SelectionTests(unittest.TestCase):
    def test_every_skip_names_an_existing_module(self) -> None:
        for name in sorted(static.skipped_modules()):
            with self.subTest(name=name):
                self.assertTrue((ROOT / (name.replace(".", "/") + ".py")).is_file())

    def test_a_module_runs_unless_it_is_skipped(self) -> None:
        selected = static.modules()
        self.assertIn("tests.ci.test_rule_marks", selected)
        self.assertIn("tests.dev.test_host_preflight", selected)
        # A module whose only cargo test is `medium` runs: its small tests are static.
        self.assertIn("tests.ci.test_product_contracts", selected)
        self.assertNotIn("tests.ci.test_acceptance_scenarios", selected)
        self.assertEqual(set(selected) & static.skipped_modules(), set())
        self.assertGreater(len(selected), 80)

    def test_medium_tests_stay_out(self) -> None:
        spec = importlib.util.spec_from_file_location("run_unittest", ROOT / "scripts" / "ci" / "run-unittest.py")
        runner = importlib.util.module_from_spec(spec)
        sys.modules["run_unittest"] = runner
        spec.loader.exec_module(runner)
        sizes = runner.Sizes(["test_x.Heavy"])

        class Heavy(unittest.TestCase):
            def test_one(self) -> None:
                pass

        class Light(unittest.TestCase):
            def test_two(self) -> None:
                pass

        def renamed(case, module):
            case.id = lambda: f"tests.ci.{module}.{type(case).__name__}.{case._testMethodName}"
            return case

        suite = unittest.TestSuite([renamed(Heavy("test_one"), "test_x"), renamed(Light("test_two"), "test_x")])
        kept = list(static.small_tests(suite, sizes))
        self.assertEqual([case._testMethodName for case in kept], ["test_two"])


class PullRequestLevelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.steps = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]["unica-ci"]["steps"]

    def step(self, name: str) -> dict:
        return next(step for step in self.steps if step.get("name") == name)

    def test_full_tests_run_only_in_the_queue(self) -> None:
        for name in ("Тесты Rust", "Тесты Python", "Продукт для Python"):
            with self.subTest(name=name):
                self.assertIn("github.event_name == 'merge_group'", self.step(name)["if"])
        nextest = next(step for step in self.steps if step.get("uses") == "./.github/actions/install-nextest")
        self.assertIn("github.event_name == 'merge_group'", nextest["if"])

    def test_the_pull_request_keeps_format_contract_clippy_and_static_tests(self) -> None:
        for name in ("Формат", "Контракт версий", "Clippy"):
            with self.subTest(name=name):
                self.assertEqual(self.step(name).get("if"), "env.KEPT != 'true'")
        checks = self.step("Статические проверки Python")
        self.assertEqual(checks["if"], "${{ !cancelled() && github.event_name == 'pull_request' && env.KEPT != 'true' }}")
        self.assertEqual(checks["run"], "python scripts/ci/background_step.py wait static-python --timeout 900")
        started = self.step("Статические проверки Python (запуск)")
        self.assertEqual(started["if"], "${{ github.event_name == 'pull_request' && env.KEPT != 'true' }}")
        self.assertEqual(started["run"],
                         "python scripts/ci/background_step.py start static-python -- python scripts/ci/run-static-tests.py")

    def test_python_checks_start_before_rust_and_are_awaited_after_clippy(self) -> None:
        """Проверки Python идут параллельно с кэшем, fmt и Clippy, а исход отдают шаги ожидания."""
        names = [step.get("name") for step in self.steps]
        for job in ("static-python", "removed-tests"):
            start = next(index for index, step in enumerate(self.steps) if f" start {job} " in step.get("run", ""))
            wait = next(index for index, step in enumerate(self.steps) if f" wait {job} " in step.get("run", ""))
            with self.subTest(job=job):
                self.assertLess(start, names.index("Кэш зависимостей Rust"))
                self.assertGreater(wait, names.index("Clippy"))

    def test_the_pull_request_checks_removed_rust_tests_against_its_base(self) -> None:
        removed = self.step("Удалённые тесты объявлены")
        self.assertEqual(removed["if"], "${{ !cancelled() && github.event_name == 'pull_request' && env.KEPT != 'true' }}")
        self.assertEqual(removed["run"], "python scripts/ci/background_step.py wait removed-tests --timeout 600")
        started = self.step("Удалённые тесты (запуск)")
        self.assertEqual(started["run"], 'python scripts/ci/background_step.py start removed-tests -- '
                                         'python scripts/ci/removed_tests.py --base "origin/$BASE_REF" --nightly-profile large')
        # Not the event's base.sha: it is frozen at PR creation, while the
        # checkout is the merge ref with the current base branch.
        self.assertEqual(started["env"]["BASE_REF"], "${{ github.base_ref }}")
        self.assertNotIn("BASE_SHA", started["env"])
        # The description is read fresh through the API: the job may read pull requests.
        # Reading actions is for the queue's nightly path, which finds the last night's inventory.
        job = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]["unica-ci"]
        self.assertEqual(job["permissions"], {"contents": "read", "pull-requests": "read", "actions": "read"})


if __name__ == "__main__":
    unittest.main()
