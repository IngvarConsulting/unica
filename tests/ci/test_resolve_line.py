"""Линия прогона: ветка как есть, очередь — её база, тег — только на релизной линии."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "resolve-line.py"


def load_module():
    spec = importlib.util.spec_from_file_location("resolve_line", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ResolveLineTests(unittest.TestCase):
    def test_an_alpha_tag_outside_every_branch_belongs_to_main(self) -> None:
        module = load_module()
        self.assertEqual(module.resolve("tag", "v0.15.0-alpha.3", "abc", branches_containing=lambda sha: []), "main")

    def test_a_measurement_tag_outside_a_release_line_is_still_refused(self) -> None:
        module = load_module()
        with self.assertRaises(SystemExit):
            module.resolve("tag", "v0.15.0-beta.1", "abc", branches_containing=lambda sha: [])

    def test_branch_push_is_its_own_line(self) -> None:
        module = load_module()

        self.assertEqual(module.resolve("branch", "release-v0.12", "abc", lambda sha: []), "release-v0.12")
        self.assertEqual(module.resolve("branch", "main", "abc", lambda sha: []), "main")

    def test_merge_queue_branch_resolves_to_its_base_line(self) -> None:
        """Очередь проверяет дерево базы: отчёт едет в линию базы, а не во временную ветку."""
        module = load_module()

        self.assertEqual(
            module.resolve("branch", "gh-readonly-queue/main/pr-786-edc939f6e3a824cab93e3c06dd1daf9c93ecb78c", "abc", lambda sha: []),
            "main",
        )
        self.assertEqual(module.resolve("branch", "gh-readonly-queue/release-v0.13/pr-9-0abc", "abc", lambda sha: []), "release-v0.13")
        # Похожее имя без формы очереди остаётся веткой как есть.
        self.assertEqual(module.resolve("branch", "gh-readonly-queue/main", "abc", lambda sha: []), "gh-readonly-queue/main")

    def test_tag_resolves_to_the_youngest_release_line_holding_its_commit(self) -> None:
        module = load_module()
        containing = lambda sha: ["main", "release-v0.13", "release-v0.12", "feature/x"]

        self.assertEqual(module.resolve("tag", "v0.12.4", "abc", containing), "release-v0.12")

    def test_a_button_tag_takes_the_line_its_name_allows_from_the_parent_of_its_version_commit(self) -> None:
        """Кнопки ставят тег на служебный коммит вне веток: линию называет его родитель."""
        module = load_module()
        parents = {"rc": "slice", "patch": "line-head", "old-rc": "old-slice"}
        holders = {"slice": ["main"], "line-head": ["release-v0.15"], "old-slice": ["main", "release-v0.15"]}
        resolve = lambda tag, sha: module.resolve(
            "tag", tag, sha, lambda commit: holders.get(commit, []), lambda commit: parents.get(commit))

        self.assertEqual(resolve("v0.16.0-rc.2", "rc"), "main")
        self.assertEqual(resolve("v0.15.1", "patch"), "release-v0.15")
        # Кандидат на срезе, который унаследовала и линия, остаётся на main.
        self.assertEqual(resolve("v0.16.0-rc.1", "old-rc"), "main")

    def test_a_final_version_lives_on_the_line_it_opened(self) -> None:
        module = load_module()
        self.assertEqual(
            module.resolve("tag", "v0.16.0", "final", lambda commit: ["release-v0.16"] if commit == "final" else [],
                           lambda commit: "rc"),
            "release-v0.16",
        )

    def test_a_tag_whose_name_does_not_allow_the_line_beneath_it_is_refused(self) -> None:
        module = load_module()
        parents = {"a": "b", "m": "slice", "c": "a"}
        holders = {"b": ["release-v0.14"], "slice": ["main"]}
        cases = {
            "patch over another line": ("v0.15.1", "a"),
            "patch over main": ("v0.15.1", "m"),
            "candidate over a line": ("v0.15.0-rc.1", "a"),
            "two steps from a branch": ("v0.15.0-rc.1", "c"),
            "measurement build": ("v0.15.0-beta.1", "m"),
        }
        for name, (tag, sha) in cases.items():
            with self.subTest(name=name), self.assertRaises(SystemExit):
                module.resolve("tag", tag, sha, lambda commit: holders.get(commit, []),
                               lambda commit: parents.get(commit))
        with self.assertRaises(SystemExit):
            module.resolve("tag", "v0.15.1", "a", lambda commit: [], lambda commit: None)

    def test_tag_outside_any_release_line_is_a_refusal(self) -> None:
        """Теги только на релизных линиях: тег на main или в воздухе — отказ."""
        module = load_module()

        with self.assertRaises(SystemExit) as refused:
            module.resolve("tag", "v0.14.0", "abc1234", lambda sha: ["main"])
        self.assertIn("релизной линии", str(refused.exception))


if __name__ == "__main__":
    unittest.main()
