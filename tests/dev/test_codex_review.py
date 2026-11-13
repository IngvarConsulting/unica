"""Обёртка независимого ревью через Codex: поиск бинаря, отказы, запуск, вывод.

Codex подменён скриптом Python: сеть, вход и настоящее ревью не нужны.
Поддельный Codex печатает свои аргументы и рабочий каталог, поэтому тесты
видят и команду, и место запуска.
"""

from __future__ import annotations

import importlib.util
import io
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "codex-review.py"
SPEC = importlib.util.spec_from_file_location("codex_review", SCRIPT)
REVIEW = importlib.util.module_from_spec(SPEC)
sys.modules["codex_review"] = REVIEW
SPEC.loader.exec_module(REVIEW)

FAKE_CODEX = """\
import os, sys, time
print("fake codex", *sys.argv[1:])
print("cwd", os.getcwd())
print("git", os.popen("git rev-parse --abbrev-ref HEAD").read().strip())
sys.stdout.flush()
time.sleep(float(os.environ.get("FAKE_CODEX_SLEEP", "0")))
sys.exit(int(os.environ.get("FAKE_CODEX_EXIT", "0")))
"""


def fake_command(directory: Path) -> list:
    """Та же подделка без обёртки .cmd: убитый процесс не оставляет потомка."""
    script = directory / "codex-body.py"
    script.write_text(FAKE_CODEX, encoding="utf-8")
    return [sys.executable, str(script)]


def fake_codex(directory: Path, name: str = "codex") -> Path:
    """Исполняемый файл, который запускается как Codex на любой ОС."""
    script = directory / f"{name}-body.py"
    script.write_text(FAKE_CODEX, encoding="utf-8")
    if sys.platform == "win32":
        launcher = directory / f"{name}.cmd"
        launcher.write_text(f'@echo off\r\n"{sys.executable}" "{script}" %*\r\n', encoding="utf-8")
    else:
        launcher = directory / name
        launcher.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{script}" "$@"\n', encoding="utf-8")
        launcher.chmod(0o755)
    return launcher


class FindCodex(unittest.TestCase):
    def test_path_wins_over_install(self):
        found = REVIEW.find_codex({}, "win32", lambda name: "C:/bin/codex.exe")
        self.assertEqual(found, Path("C:/bin/codex.exe"))

    def test_codex_bin_wins_over_path(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "codex.exe"
            binary.write_text("", encoding="utf-8")
            found = REVIEW.find_codex({"CODEX_BIN": str(binary)}, "win32", lambda name: "other")
            self.assertEqual(found, binary)

    def test_codex_bin_without_file_refuses(self):
        with self.assertRaises(REVIEW.Refusal) as error:
            REVIEW.find_codex({"CODEX_BIN": "/nope/codex"}, "linux", lambda name: None)
        self.assertIn("CODEX_BIN", str(error.exception))

    def test_install_directory_with_build_hash_is_found(self):
        with tempfile.TemporaryDirectory() as temp:
            local = Path(temp) / "Local"
            for hashed, age in (("aaaa", 10_000), ("bbbb", 20_000)):
                directory = local / "OpenAI" / "Codex" / "bin" / hashed
                directory.mkdir(parents=True)
                binary = directory / "codex.exe"
                binary.write_text("", encoding="utf-8")
                os.utime(binary, (age, age))
            # Каталог без codex.exe установщик оставляет для других утилит.
            other = local / "OpenAI" / "Codex" / "bin" / "cccc"
            other.mkdir(parents=True)
            (other / "rg.exe").write_text("", encoding="utf-8")
            found = REVIEW.find_codex(
                {"LOCALAPPDATA": str(local)}, "win32", lambda name: None
            )
            self.assertEqual(found.parent.name, "bbbb")

    def test_missing_codex_names_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(REVIEW.Refusal) as error:
                REVIEW.find_codex({"HOME": temp, "LOCALAPPDATA": temp}, "linux", lambda name: None)
        self.assertIn("субагентом", str(error.exception))

    def test_unknown_platform_uses_posix_globs(self):
        self.assertEqual(REVIEW.install_globs("freebsd13"), REVIEW.INSTALL_GLOBS["other"])

    def test_expand_without_home_skips_pattern(self):
        self.assertIsNone(REVIEW.expand("{HOME}/.codex/bin/*/codex", {}))


class Login(unittest.TestCase):
    def test_auth_file_is_enough(self):
        with tempfile.TemporaryDirectory() as temp:
            (Path(temp) / "auth.json").write_text("{}", encoding="utf-8")
            REVIEW.check_login({"CODEX_HOME": temp})

    def test_missing_auth_refuses_with_command(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(REVIEW.Refusal) as error:
                REVIEW.check_login({"CODEX_HOME": temp})
        self.assertIn("codex login", str(error.exception))

    def test_codex_home_falls_back_to_user_profile(self):
        self.assertEqual(
            REVIEW.codex_home({"USERPROFILE": "C:/Users/x"}), Path("C:/Users/x/.codex")
        )


class Arguments(unittest.TestCase):
    def test_default_is_base_origin_main(self):
        self.assertEqual(REVIEW.review_args(None, None, None), ["review", "--base", "origin/main"])

    def test_commit_replaces_base(self):
        self.assertEqual(REVIEW.review_args(None, "abc123", None), ["review", "--commit", "abc123"])

    def test_title_is_passed_through(self):
        self.assertEqual(
            REVIEW.review_args("main", None, "PR 42"),
            ["review", "--base", "main", "--title", "PR 42"],
        )

    def test_out_path_is_safe_for_a_slashed_ref(self):
        path = REVIEW.out_path(Path("/repo"), "origin/main", 0)
        self.assertNotIn("/", path.name)
        self.assertTrue(path.name.endswith("-origin-main.md"))
        self.assertEqual(path.parent, Path("/repo") / REVIEW.OUT_DIR)


class Run(unittest.TestCase):
    def test_output_goes_to_file_and_echo(self):
        with tempfile.TemporaryDirectory() as temp:
            codex = fake_command(Path(temp))
            destination = Path(temp) / "out" / "review.md"
            seen = []
            code = REVIEW.run(
                [*codex, "review", "--base", "main"],
                Path(temp),
                dict(os.environ),
                destination,
                seen.append,
                60,
            )
            self.assertEqual(code, 0)
            text = destination.read_text(encoding="utf-8")
            self.assertIn("fake codex review --base main", text)
            self.assertIn("fake codex review --base main", seen)

    def test_codex_exit_code_is_returned(self):
        with tempfile.TemporaryDirectory() as temp:
            codex = fake_command(Path(temp))
            destination = Path(temp) / "review.md"
            code = REVIEW.run(
                codex,
                Path(temp),
                {**os.environ, "FAKE_CODEX_EXIT": "3"},
                destination,
                lambda line: None,
                60,
            )
            self.assertEqual(code, 3)

    def test_silent_hang_is_killed_by_the_limit(self):
        with tempfile.TemporaryDirectory() as temp:
            codex = fake_command(Path(temp))
            destination = Path(temp) / "review.md"
            code = REVIEW.run(
                codex,
                Path(temp),
                {**os.environ, "FAKE_CODEX_SLEEP": "60"},
                destination,
                lambda line: None,
                1,
            )
            self.assertEqual(code, 124)
            self.assertIn("пределу времени", destination.read_text(encoding="utf-8"))


class Worktree(unittest.TestCase):
    """Исполнители живут в worktree: каталог .git лежит выше корня копии."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.main = self.root / "clone"
        self.main.mkdir()
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.com",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.com",
        }
        self.env = env
        self.git("init", "--initial-branch=main")
        (self.main / "a.txt").write_text("one\n", encoding="utf-8")
        self.git("add", "a.txt")
        self.git("commit", "-m", "first")
        self.worktree = self.root / "wt"
        self.git("worktree", "add", "-b", "work", str(self.worktree))

    def git(self, *args: str) -> str:
        done = subprocess.run(
            ["git", *args],
            cwd=str(self.main),
            env=self.env,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        return done.stdout

    def test_review_runs_at_the_worktree_root(self):
        codex = fake_codex(self.root)
        (self.worktree / "b.txt").write_text("two\n", encoding="utf-8")
        inner = self.worktree / "deep"
        inner.mkdir()
        home = self.root / "home"
        home.mkdir()
        (home / "auth.json").write_text("{}", encoding="utf-8")
        out = self.root / "review.md"
        environ = {**self.env, "CODEX_BIN": str(codex), "CODEX_HOME": str(home)}
        # Скрипт зовут из подкаталога копии: корнем должен стать корень worktree.
        with mock.patch.object(REVIEW, "REPO_ROOT", inner), mock.patch.dict(
            os.environ, environ, clear=False
        ):
            captured = io.StringIO()
            with redirect_stdout(captured), redirect_stderr(io.StringIO()):
                code = REVIEW.main(["--base", "main", "--out", str(out)])
        self.assertEqual(code, 0)
        self.assertEqual(captured.getvalue().strip(), str(out))
        text = out.read_text(encoding="utf-8")
        self.assertIn("fake codex review --base main", text)
        self.assertIn(str(self.worktree.resolve()), text.replace("/", os.sep))
        # Поддельный Codex выполнил git внутри копии: .git там файл со ссылкой.
        self.assertIn("git work", text)

    def test_unknown_base_refuses_before_codex(self):
        codex = fake_codex(self.root)
        home = self.root / "home"
        home.mkdir()
        (home / "auth.json").write_text("{}", encoding="utf-8")
        out = self.root / "never.md"
        environ = {**self.env, "CODEX_BIN": str(codex), "CODEX_HOME": str(home)}
        with mock.patch.object(REVIEW, "REPO_ROOT", self.worktree), mock.patch.dict(
            os.environ, environ, clear=False
        ):
            errors = io.StringIO()
            with redirect_stdout(io.StringIO()), redirect_stderr(errors):
                code = REVIEW.main(["--base", "origin/main", "--out", str(out)])
        self.assertEqual(code, 2)
        self.assertIn("origin/main", errors.getvalue())
        self.assertFalse(out.exists())

    def test_findings_do_not_fail_the_wrapper_but_a_crash_does(self):
        codex = fake_codex(self.root)
        home = self.root / "home"
        home.mkdir()
        (home / "auth.json").write_text("{}", encoding="utf-8")
        environ = {**self.env, "CODEX_BIN": str(codex), "CODEX_HOME": str(home)}
        with mock.patch.object(REVIEW, "REPO_ROOT", self.worktree), mock.patch.dict(
            os.environ, environ, clear=False
        ):
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                clean = REVIEW.main(["--base", "main", "--out", str(self.root / "ok.md")])
            with mock.patch.dict(os.environ, {"FAKE_CODEX_EXIT": "1"}, clear=False):
                with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                    broken = REVIEW.main(["--base", "main", "--out", str(self.root / "bad.md")])
        self.assertEqual(clean, 0)
        self.assertEqual(broken, 1)


if __name__ == "__main__":
    unittest.main()
