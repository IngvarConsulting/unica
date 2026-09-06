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
        runner_script = str(MODULE_PATH.with_name("run-unittest.py"))
        self.assertEqual(
            python,
            [
                ["python", runner_script, "-s", "tests/ci", "--durations", "20"],
                ["python", runner_script, "-s", "tests/arch"],
                ["python", runner_script, "-s", "tests/dev", "--durations", "20"],
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
        self.assertTrue(all(command[1].endswith("run-unittest.py") for command in planned[1:]))
        self.assertEqual(len(planned), 4)

    def test_results_directory_turns_emission_on_for_python_suites(self) -> None:
        """Без `--results` набор идёт как раньше; с ним пишет результаты и знает раннер."""
        module = load_module()
        from pathlib import Path as P

        quiet = module.commands("all", "python", interpreter="python")
        emitting = module.commands("all", "python", interpreter="python", results=P("out"), runner="macos-14")

        self.assertTrue(all("--results" not in command for command in quiet))
        for command in emitting:
            self.assertIn("--results", command)
            self.assertIn("macos-14", command)

    def test_own_runner_imports_every_module_the_module_runner_could(self) -> None:
        """`python -m unittest` видит `scripts.ci.*` от корня; наш раннер обязан тоже.

        Иначе модуль, импортирующий скрипт конвейера, падает на импорте и
        считается упавшим тестом — так и было на первом прогоне.
        """
        import subprocess
        import sys

        runner = MODULE_PATH.with_name("run-unittest.py")
        listing = subprocess.run(
            [sys.executable, str(runner), "-s", "tests/ci", "--plan-only"],
            cwd=MODULE_PATH.parents[2], capture_output=True, text=True, check=True,
        ).stdout

        self.assertNotIn("_FailedTest", listing)
        self.assertIn("test_donor_parity_contract", listing)

    def test_run_signature_is_written_once_per_invocation_and_names_the_ecosystem(self) -> None:
        """`--ecosystem all` — одна подпись с `all`, а не две, где вторая стирает первую."""
        import json
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))
        calls = []

        code = module.execute("all", "all", out, "ubuntu-latest",
                              run_commands=lambda planned: calls.append(planned) or 0,
                              junit=out / "no-such-junit.xml")

        self.assertEqual(code, 0)
        self.assertEqual(len(calls), 2)
        signature = json.loads((out / "run.json").read_text(encoding="utf-8"))
        self.assertEqual(signature["ecosystem"], "all")
        self.assertEqual(signature["runner"], "ubuntu-latest")

    def test_rust_failure_before_junit_still_leaves_the_signature(self) -> None:
        """Подпись описывает вызов, а не исход: nextest упал до JUnit — она всё равно есть."""
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))

        code = module.execute("all", "rust", out, "macos-14",
                              run_commands=lambda planned: 101,
                              junit=out / "no-such-junit.xml")

        self.assertEqual(code, 101)
        self.assertTrue((out / "run.json").is_file())
        self.assertEqual(list(out.glob("*-result.json")), [])

    def test_stale_junit_from_an_earlier_run_is_never_emitted_as_fresh(self) -> None:
        """nextest упал до отчёта, а от прошлого прогона остался JUnit — он не результат."""
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))
        stale = out / "junit.xml"
        stale.write_text(
            '<testsuites name="unica"><testsuite name="unica-coder">'
            '<testcase name="old::passed" classname="unica-coder" time="0.1"/>'
            "</testsuite></testsuites>",
            encoding="utf-8",
        )

        code = module.execute("all", "rust", out, "ubuntu-latest",
                              run_commands=lambda planned: 101, junit=stale)

        self.assertEqual(code, 101)
        self.assertFalse(stale.exists(), "старый отчёт обязан быть снят до прогона")
        self.assertEqual(list(out.glob("*-result.json")), [])

    def test_unknown_profile_or_ecosystem_is_a_refusal_not_an_empty_run(self) -> None:
        """Пустой прогон зелёный по устройству; отказ — единственный честный ответ."""
        module = load_module()

        with self.assertRaises(ValueError):
            module.commands("weekly", "rust")
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


class GateProfileTests(unittest.TestCase):
    """Ворота ложатся на одноимённые профили nextest; Python отбирает наборы по размеру."""

    def test_each_gate_maps_to_its_own_nextest_profile_and_junit_directory(self) -> None:
        module = load_module()

        for gate in ("pr", "queue", "main", "release"):
            with self.subTest(gate=gate):
                self.assertEqual(
                    module.rust_commands(gate),
                    [["cargo", "nextest", "run", "--workspace", "--profile", gate]],
                )
                self.assertEqual(module.nextest_junit(gate).parts[-3:], ("nextest", gate, "junit.xml"))
        self.assertEqual(module.nextest_junit("all").parts[-3:], ("nextest", "default", "junit.xml"))

    def test_every_gate_runs_every_python_suite_while_all_suites_are_small(self) -> None:
        """Отбора пока нет: все наборы `small`, и любые ворота гоняют все три."""
        module = load_module()

        for gate in module.PROFILES:
            if gate == "large":
                continue
            with self.subTest(gate=gate):
                suites = [command[3] for command in module.python_commands(gate, "python3")]
                self.assertEqual(suites, [suite for suite, _, _ in module.PYTHON_SUITES])
        # Ночной ярус принимает только `large`, а таких наборов пока нет.
        self.assertEqual(module.python_commands("large", "python3"), [])


if __name__ == "__main__":
    unittest.main()
