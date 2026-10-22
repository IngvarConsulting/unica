"""Шаг в фоне: ожидание несёт исход и вывод команды, пропущенный старт — отказ."""
from __future__ import annotations

import io
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import background_step as background  # noqa: E402


class BackgroundStepTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.state = Path(temporary.name) / "state"
        self.cwd = Path(temporary.name)

    def run_and_wait(self, name, script):
        with redirect_stdout(io.StringIO()):
            background.start(name, ["bash", "-c", script], state=self.state, cwd=self.cwd)
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            code = background.wait(name, state=self.state, timeout=30, interval=0.05)
        return code, stdout.getvalue()

    def test_wait_returns_the_command_exit_code_and_its_output(self):
        self.assertEqual(self.run_and_wait("green", "echo linted; exit 0"), (0, "linted\n"))
        code, output = self.run_and_wait("red", "echo warning >&2; exit 101")
        self.assertEqual(code, 101)
        self.assertIn("warning", output)

    def test_start_returns_before_the_command_finishes(self):
        with redirect_stdout(io.StringIO()):
            background.start("slow", ["bash", "-c", "sleep 2; exit 3"], state=self.state, cwd=self.cwd)
        self.assertFalse(background.paths("slow", self.state)[2].exists())
        with redirect_stdout(io.StringIO()):
            self.assertEqual(background.wait("slow", state=self.state, timeout=30, interval=0.05), 3)

    def test_a_missing_start_or_an_unfinished_command_is_a_failure(self):
        with redirect_stdout(io.StringIO()):
            self.assertEqual(background.wait("never-started", state=self.state, timeout=1, interval=0.05), 1)
            background.start("hang", ["bash", "-c", "sleep 30"], state=self.state, cwd=self.cwd)
            self.assertEqual(background.wait("hang", state=self.state, timeout=0.2, interval=0.05), 1)

    def test_a_restart_forgets_the_previous_outcome(self):
        self.assertEqual(self.run_and_wait("again", "exit 0")[0], 0)
        self.assertEqual(self.run_and_wait("again", "exit 7")[0], 7)


if __name__ == "__main__":
    unittest.main()
