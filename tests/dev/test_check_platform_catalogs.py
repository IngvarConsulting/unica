"""Сверка каталогов справки с установками: выбор сборки, исходы, коды выхода.

Оракулы не запускаются: прогон подменён. Проверяется то, что решает скрипт, —
какую установку взять под каталог и как назвать исход. Платформа не нужна.
"""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "check-platform-catalogs.py"
SPEC = importlib.util.spec_from_file_location("check_platform_catalogs", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
sys.modules["check_platform_catalogs"] = CHECK
SPEC.loader.exec_module(CHECK)


def catalog(line: str, build: str) -> "CHECK.Catalog":
    return CHECK.Catalog(line, build, Path(f"platform-event-catalog-{line}.json"))


class Lines(unittest.TestCase):
    def test_line_is_the_first_three_numbers_of_a_build(self):
        self.assertEqual(CHECK.line_of("8.3.27.2342"), "8.3.27")
        self.assertEqual(CHECK.line_of("8.5.4.1878"), "8.5.4")

    def test_a_directory_that_is_not_a_build_has_no_line(self):
        for name in ("common", "conf", "8.3.27", "8.3.27.2342.1"):
            self.assertIsNone(CHECK.line_of(name), name)


class Catalogs(unittest.TestCase):
    def test_catalog_is_read_with_its_line_and_pinned_build(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            directory = root / CHECK.CATALOG_DIR
            directory.mkdir(parents=True)
            (directory / "platform-event-catalog-8.3.27.json").write_text(
                json.dumps({"source": {"installationVersion": "8.3.27.2342"}}), encoding="utf-8"
            )
            # Посторонний файл того же каталога не должен попадать в список.
            (directory / "event_catalog.rs").write_text("", encoding="utf-8")

            found = CHECK.catalogs(root)

        self.assertEqual([(entry.line, entry.pinned_build) for entry in found], [("8.3.27", "8.3.27.2342")])


class Plan(unittest.TestCase):
    """Какую установку взять под каталог и почему именно её."""

    def test_pinned_build_is_preferred_when_installed(self):
        installed = {"8.3.27.2074": Path("/p/2074"), "8.3.27.2342": Path("/p/2342")}
        (_, installation, note) = CHECK.plan([catalog("8.3.27", "8.3.27.2342")], installed)[0]
        self.assertEqual(installation, Path("/p/2342"))
        self.assertEqual(note, "8.3.27.2342")

    def test_another_build_of_the_line_is_named_as_unverified(self):
        installed = {"8.3.27.9000": Path("/p/9000")}
        (_, installation, note) = CHECK.plan([catalog("8.3.27", "8.3.27.2342")], installed)[0]
        self.assertEqual(installation, Path("/p/9000"))
        self.assertIn("закреплена 8.3.27.2342", note)
        self.assertIn("установлена 8.3.27.9000", note)

    def test_a_line_without_an_installation_has_nothing_to_check(self):
        (_, installation, note) = CHECK.plan([catalog("8.5.1", "8.5.1.1529")], {"8.3.27.2342": Path("/p")})[0]
        self.assertIsNone(installation)
        self.assertIn("8.5.1", note)

    def test_a_build_of_another_line_is_never_taken(self):
        installed = {"8.3.26.1656": Path("/p/26"), "8.5.4.2028": Path("/p/54")}
        (_, installation, _) = CHECK.plan([catalog("8.3.27", "8.3.27.2342")], installed)[0]
        self.assertIsNone(installation)


class Report(unittest.TestCase):
    def test_everything_checked_and_passed_is_zero(self):
        lines = []
        code = CHECK.report([CHECK.Outcome("8.3.27", CHECK.PASSED, "сборка 8.3.27.2342")], [], lines.append)
        self.assertEqual(code, 0)
        self.assertIn("Расхождений: нет", "\n".join(lines))

    def test_a_divergence_is_one(self):
        code = CHECK.report([CHECK.Outcome("8.3.27", CHECK.DIVERGED, "оракул упал")], [], lambda line: None)
        self.assertEqual(code, 1)

    def test_a_catalog_left_unchecked_is_two(self):
        code = CHECK.report([CHECK.Outcome("8.5.1", CHECK.UNCHECKED, "установки нет")], [], lambda line: None)
        self.assertEqual(code, 2)

    def test_a_line_without_a_catalog_is_named_but_does_not_fail_the_run(self):
        """Иначе прогон красный всегда, пока на хосте стоит старая платформа."""
        lines = []
        code = CHECK.report(
            [CHECK.Outcome("8.3.27", CHECK.PASSED, "сборка 8.3.27.2342")], ["8.3.24", "8.5.4"], lines.append
        )
        self.assertEqual(code, 0)
        self.assertIn("8.3.24, 8.5.4", "\n".join(lines))

    def test_a_divergence_outweighs_an_unchecked_catalog(self):
        code = CHECK.report(
            [CHECK.Outcome("8.3.27", CHECK.DIVERGED, "упал"), CHECK.Outcome("8.5.1", CHECK.UNCHECKED, "нет")],
            [],
            lambda line: None,
        )
        self.assertEqual(code, 1)


class Oracles(unittest.TestCase):
    def test_the_installation_is_handed_to_the_oracles(self):
        seen = {}

        def runner(command, cwd, env):
            seen["command"] = command
            seen["dir"] = env["UNICA_PLATFORM_HELP_DIR"]
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        passed, _ = CHECK.run_oracles(Path("/repo"), Path("/p/2342"), {}, runner)

        self.assertTrue(passed)
        self.assertEqual(seen["dir"], str(Path("/p/2342")))
        self.assertIn("--ignored", seen["command"])
        self.assertIn(CHECK.ORACLE_CRATE, seen["command"])

    def test_a_failing_oracle_is_reported_by_name(self):
        output = "\n".join(
            [
                "test platform_help_oracle::alpha ... ok",
                "test platform_help_oracle::beta ... FAILED",
                "thread 'beta' panicked at src/lib.rs:1:1:",
            ]
        )
        named = CHECK.failing_lines(output)
        self.assertTrue(any("beta" in line and "FAILED" in line for line in named))
        self.assertFalse(any("alpha" in line for line in named))


if __name__ == "__main__":
    unittest.main()
