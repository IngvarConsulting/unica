"""Выпуск alpha: номер, пропуск среза, отказ после выпуска и очистка."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "release-alpha.py"


def load_module():
    spec = importlib.util.spec_from_file_location("release_alpha", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    import sys

    sys.modules["release_alpha"] = module
    spec.loader.exec_module(module)
    return module


class PlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def plan(self, tags, parent="old", **options):
        return self.module.plan_next("0.15.0", "head", tags, lambda tag: parent, **options)

    def test_the_first_alpha_of_a_core_is_number_one(self) -> None:
        self.assertEqual(self.plan(["v0.14.0", "v0.15.0-rc.1"]).tag, "v0.15.0-alpha.1")

    def test_the_number_compares_as_a_number_and_ignores_other_cores(self) -> None:
        tags = ["v0.15.0-alpha.9", "v0.15.0-alpha.10", "v0.16.0-alpha.40"]
        self.assertEqual(self.plan(tags).tag, "v0.15.0-alpha.11")

    def test_a_released_slice_is_not_released_again(self) -> None:
        plan = self.plan(["v0.15.0-alpha.3"], parent="head")
        self.assertIsNone(plan.tag)
        self.assertIn("v0.15.0-alpha.3", plan.reason)

    def test_force_releases_the_same_slice_under_a_new_number(self) -> None:
        plan = self.plan(["v0.15.0-alpha.3"], parent="head", force=True)
        self.assertEqual(plan.tag, "v0.15.0-alpha.4")

    def test_a_full_release_of_the_core_refuses(self) -> None:
        with self.assertRaisesRegex(self.module.AlphaError, "v0.15.0"):
            self.plan(["v0.15.0", "v0.15.0-alpha.3"])

    def test_the_core_comes_from_the_workspace_version(self) -> None:
        cargo = '[package]\nversion = "9.9.9"\n\n[workspace.package]\nedition = "2021"\nversion = "0.15.0-rc.1"\n'
        self.assertEqual(self.module.core_version(cargo), "0.15.0")


class PublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_a_dry_run_changes_nothing(self) -> None:
        commands = []

        def runner(command, **_):
            commands.append(command)
            return "head\n"

        def gh(path, **_):
            return [{"ref": "refs/tags/v0.15.0-rc.1"}] if "matching-refs" in path else {"parents": []}

        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.15.0-rc.1"\n', encoding="utf-8")
            tag = self.module.publish(root, "o/r", force=False, dry_run=True, gh=gh, runner=runner)

        self.assertEqual(tag, "v0.15.0-alpha.1")
        self.assertEqual(commands, [["git", "rev-parse", "HEAD"]])


class DispatchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def publish(self, failures: int):
        import os
        import tempfile
        from unittest import mock

        left = [failures]

        def runner(command, **_):
            if command[:2] == ["git", "rev-parse"]:
                return "head\n"
            if command[:2] == ["gh", "workflow"]:
                if left[0]:
                    left[0] -= 1
                    raise self.module.AlphaError("HTTP 404")
            return ""

        def gh(path, **_):
            return []

        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"GH_TOKEN": "t"}):
            root = Path(directory)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.15.0"\n', encoding="utf-8")
            return self.module.publish(root, "o/r", force=False, dry_run=False, gh=gh, runner=runner, pause=lambda _: None)

    def test_the_build_is_dispatched_after_github_sees_the_tag(self) -> None:
        self.assertEqual(self.publish(failures=2), "v0.15.0-alpha.1")

    def test_a_dispatch_that_never_works_names_the_tag_and_the_command(self) -> None:
        with self.assertRaisesRegex(self.module.AlphaError, "v0.15.0-alpha.1.*gh workflow run"):
            self.publish(failures=99)


class PruneTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def gh(self, tags, releases, refuse=False):
        calls = []

        def gh(path, method="GET"):
            calls.append((method, path))
            if method == "DELETE" and refuse and "git/refs" in path:
                raise self.module.RuleRefused("HTTP 422 Repository rule violations")
            if "matching-refs" in path:
                return [{"ref": f"refs/tags/{tag}"} for tag in tags]
            if path.endswith("/releases"):
                return [{"tag_name": tag, "id": index, "draft": False} for index, tag in enumerate(releases)]
            return None

        return gh, calls

    def test_two_newest_published_stay_and_only_alphas_go(self) -> None:
        tags = ["v0.15.0-rc.1", "v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3", "v0.15.0-alpha.10"]
        gh, calls = self.gh(tags, tags)
        self.assertEqual(self.module.prune("o/r", keep=2, dry_run=False, gh=gh), ["v0.15.0-alpha.1", "v0.15.0-alpha.2"])
        deleted = [path for method, path in calls if method == "DELETE"]
        self.assertEqual(deleted[0], "repos/o/r/git/refs/tags/v0.15.0-alpha.1")
        self.assertEqual(deleted[1], "repos/o/r/releases/1")
        self.assertFalse(any("rc.1" in path for path in deleted))

    def test_a_tag_still_building_does_not_push_out_a_published_alpha(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        self.assertEqual(self.module.stale_alphas(tags, {"v0.15.0-alpha.1", "v0.15.0-alpha.2"}, 2), [])

    def test_an_old_tag_without_a_release_goes_with_the_rest(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        self.assertEqual(self.module.stale_alphas(tags, {"v0.15.0-alpha.2", "v0.15.0-alpha.3"}, 2), ["v0.15.0-alpha.1"])

    def test_a_rule_refusal_names_the_fix_and_keeps_the_release(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        gh, calls = self.gh(tags, tags, refuse=True)
        with self.assertRaisesRegex(self.module.RuleRefused, r"refs/tags/v\*-alpha"):
            self.module.prune("o/r", keep=2, dry_run=False, gh=gh)
        self.assertFalse(any(method == "DELETE" and "/releases/" in path for method, path in calls))

    def test_an_immutable_release_goes_even_if_its_tag_stays(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        calls = []

        def gh(path, method="GET"):
            calls.append((method, path))
            if "matching-refs" in path:
                return [{"ref": f"refs/tags/{tag}"} for tag in tags]
            if path.endswith("/releases"):
                return [{"tag_name": tag, "id": index, "draft": False, "immutable": True} for index, tag in enumerate(tags)]
            if method == "DELETE" and "git/refs" in path:
                raise self.module.AlphaError("HTTP 422 tag of an immutable release")
            return None

        self.assertEqual(self.module.prune("o/r", keep=2, dry_run=False, gh=gh), ["v0.15.0-alpha.1"])
        self.assertIn(("DELETE", "repos/o/r/releases/0"), calls)

    def test_a_leftover_tag_without_a_release_does_not_fail_the_cleanup(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        gh, _ = self.gh(tags, tags[1:], refuse=True)
        self.assertEqual(self.module.prune("o/r", keep=2, dry_run=False, gh=gh), ["v0.15.0-alpha.1"])

    def test_a_dry_run_deletes_nothing(self) -> None:
        tags = ["v0.15.0-alpha.1", "v0.15.0-alpha.2", "v0.15.0-alpha.3"]
        gh, calls = self.gh(tags, tags)
        self.assertEqual(self.module.prune("o/r", keep=2, dry_run=True, gh=gh), [])
        self.assertFalse(any(method == "DELETE" for method, _ in calls))


if __name__ == "__main__":
    unittest.main()
