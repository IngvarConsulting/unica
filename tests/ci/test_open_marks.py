"""Пометки незакрытого: вид, поиск по исходникам, ворота PR и выпуска."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Слово и скобка разделены, чтобы этот файл сам не нёс пометок.
TODO = "TODO" + "("


def load():
    spec = importlib.util.spec_from_file_location("open_marks", ROOT / "scripts/ci/open_marks.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["open_marks"] = module
    spec.loader.exec_module(module)
    return module


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True, text=True,
                          encoding="utf-8").stdout


class FormTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_a_mark_carries_the_number_of_a_task(self) -> None:
        [mark] = self.module.parse("a.rs", 3, f"    // {TODO}#3014): снять, когда появится каталог 8.5.4")
        self.assertEqual((mark.place, mark.number), ("a.rs:3", 3014))

    def test_anything_else_in_the_brackets_is_a_mark_without_a_number(self) -> None:
        for text in (f"{TODO})", f"{TODO}igor)", f"{TODO}#)", f"{TODO}#12 later)", f"{TODO}#0)", f"{TODO}#12",
                     f"{TODO}12)"):
            with self.subTest(text=text):
                [mark] = self.module.parse("a.rs", 1, text)
                self.assertIsNone(mark.number)

    def test_the_bare_word_is_not_a_mark(self) -> None:
        # Templates of 1C code, the BSL diagnostic help and fixtures write the word alone.
        for text in ("// TODO", "// TODO: заполнить", "TODO — запрещённый тег", "NOTODO(#1)"):
            with self.subTest(text=text):
                self.assertEqual(self.module.parse("a.rs", 1, text), [])

    def test_closing_keywords_of_the_description_name_the_tasks_it_closes(self) -> None:
        description = "Closes #12\nfixes: #3\nResolved #4\nсм. #5\nprefix#6 closes #7"
        self.assertEqual(self.module.closing_numbers(description), {12, 3, 4, 7})
        self.assertEqual(self.module.closing_numbers(""), set())


class SearchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()

    def test_only_tracked_sources_are_searched_at_the_revision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            git(root, "init", "-q")
            (root / "src").mkdir()
            (root / "src/lib.rs").write_text(f"fn a() {{}}\n// {TODO}#7): снять\n// TODO\n", encoding="utf-8")
            (root / "tool.py").write_text(f"# {TODO}later)\n", encoding="utf-8")
            # Documentation is not code: an example of the mark there is no mark.
            (root / "README.md").write_text(f"`{TODO}#7)`\n", encoding="utf-8")
            git(root, "add", "-A")
            git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "one")
            sha = git(root, "rev-parse", "HEAD").strip()
            (root / "src/lib.rs").write_text("fn a() {}\n", encoding="utf-8")
            (root / "untracked.rs").write_text(f"// {TODO}#9)\n", encoding="utf-8")

            at_revision = sorted((mark.place, mark.number) for mark in self.module.marks(root, sha))
            self.assertEqual(at_revision, [("src/lib.rs:2", 7), ("tool.py:1", None)])
            # The working tree: the edit is seen, untracked files are not.
            self.assertEqual([(mark.place, mark.number) for mark in self.module.marks(root)], [("tool.py:1", None)])

    def test_a_tree_without_marks_has_none(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            git(root, "init", "-q")
            (root / "a.rs").write_text("// TODO\n", encoding="utf-8")
            git(root, "add", "-A")
            self.assertEqual(self.module.marks(root), [])


class GateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load()
        self.states = {1: self.module.OPEN, 2: self.module.CLOSED, 3: self.module.PULL, 4: self.module.MISSING}

    def mark(self, number, text="// mark"):
        return self.module.Mark("a.rs", 5, text, number)

    def problems(self, mark, closing=frozenset()):
        return self.module.pr_problems([mark], self.states.__getitem__, set(closing))

    def test_a_mark_on_an_open_task_passes_the_pull_request(self) -> None:
        self.assertEqual(self.problems(self.mark(1)), [])

    def test_each_lost_mark_is_refused_on_the_pull_request(self) -> None:
        form = f"`{TODO}#<номер задачи>): <что снять и при каком условии>`"
        cases = {
            None: f"Пометка без номера задачи: a.rs:5 — // {TODO}x). Пометка пишется так: {form}.",
            2: "Пометка ссылается на закрытую задачу #2: a.rs:5. Снимите пометку или откройте задачу заново.",
            3: "Пометка ссылается на #3, а это pull request, не задача: a.rs:5.",
            4: "Пометка ссылается на #4, а такой задачи нет: a.rs:5.",
        }
        for number, expected in cases.items():
            with self.subTest(number=number):
                self.assertEqual(self.problems(self.mark(number, f"// {TODO}x)")), [expected])

    def test_a_task_this_pull_request_closes_cannot_keep_its_mark(self) -> None:
        self.assertEqual(self.problems(self.mark(1), closing={1}),
                         ["Этот PR закрывает #1, а пометка на неё остаётся: a.rs:5. Снимите пометку в этом же PR."])

    def test_a_release_has_no_marks_at_all(self) -> None:
        sha = "a" * 40
        self.assertIsNone(self.module.release_refusal([], sha))
        refusal = self.module.release_refusal([self.mark(1), self.mark(None)], sha)
        self.assertEqual(refusal, "Кандидат не выпускается: в срезе aaaaaaaaaaaa остались пометки незакрытого (2): "
                                  "a.rs:5 — #1; a.rs:5 — без номера. "
                                  "Каждую решите до кандидата: доделайте работу или снимите пометку.")
        patch = self.module.release_refusal([self.mark(1)], sha, self.module.PATCH)
        self.assertTrue(patch.startswith("Исправительная версия не выпускается: в голове линии aaaaaaaaaaaa"))
        self.assertTrue(patch.endswith("Каждую решите до выпуска: доделайте работу или снимите пометку."))


class RepositoryTests(unittest.TestCase):
    def test_the_repository_has_no_mark_without_a_number(self) -> None:
        module = load()
        self.assertEqual([mark.place for mark in module.marks(ROOT) if mark.number is None], [])


if __name__ == "__main__":
    unittest.main()
