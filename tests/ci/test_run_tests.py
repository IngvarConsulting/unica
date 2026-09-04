"""Шов прогона тестов повторяет прежние команды и не пропускает ничего."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "run-tests.py"


def load_module():
    spec = importlib.util.spec_from_file_location("run_tests", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RunTestsSeamTests(unittest.TestCase):
    def test_profile_all_repeats_the_former_workflow_commands_verbatim(self) -> None:
        """Команды закреплены дословно: правка гейта не должна прятаться в шве.

        Rust идёт через nextest одним профилем; число потоков и отчёт живут в
        `.config/nextest.toml`. Python — те самые строки, что стояли в workflow.
        """
        module = load_module()

        rust = module.commands("all", "rust", interpreter="python")
        python = module.commands("all", "python", interpreter="python")

        self.assertEqual(
            rust, [["cargo", "nextest", "run", "--workspace", "--profile", "default"]]
        )
        self.assertEqual(
            python,
            [
                ["python", "-m", "unittest", "discover", "-s", "tests/ci", "--durations", "20"],
                ["python", "-m", "unittest", "discover", "-s", "tests/arch"],
                ["python", "-m", "unittest", "discover", "-s", "tests/dev", "--durations", "20"],
            ],
        )

    def test_arch_suite_is_discovered_without_a_top_level_directory(self) -> None:
        """Модули `tests/arch` — не пакет: `-t .` сломал бы их импорт."""
        module = load_module()

        arch = next(
            command
            for command in module.commands("all", "python", interpreter="python")
            if "tests/arch" in command
        )

        self.assertNotIn("-t", arch)

    def test_all_ecosystems_keep_rust_before_python(self) -> None:
        module = load_module()

        planned = module.commands("all", "all", interpreter="python")

        self.assertEqual(planned[0][0], "cargo")
        self.assertTrue(all(command[0] == "python" for command in planned[1:]))
        self.assertEqual(len(planned), 4)

    def test_unknown_profile_or_ecosystem_is_a_refusal_not_an_empty_run(self) -> None:
        """Пустой прогон зелёный по устройству; отказ — единственный честный ответ."""
        module = load_module()

        with self.assertRaises(ValueError):
            module.commands("pr", "rust")
        with self.assertRaises(ValueError):
            module.commands("all", "go")

    def test_dry_run_prints_commands_and_runs_nothing(self) -> None:
        module = load_module()
        import contextlib
        import io

        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            code = module.main(["--profile", "all", "--ecosystem", "rust", "--dry-run"])

        self.assertEqual(code, 0)
        self.assertEqual(
            captured.getvalue().strip(), "cargo nextest run --workspace --profile default"
        )


if __name__ == "__main__":
    unittest.main()
