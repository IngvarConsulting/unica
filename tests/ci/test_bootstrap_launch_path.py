"""Разделение путей `run` и `verify` в bootstrap.

REQ-PERF-VERIFIED-HANDOFF говорит про релизный шлюз, а не про каждый запуск, и
это утверждение легко испортить в обе стороны. Убрать рукопожатие из `verify` —
и релиз перестанет ловить runtime, который не отвечает. Добавить его в `run` —
и каждая сессия хоста начнёт поднимать runtime дважды, ничего при этом не
доказав о том процессе, который в итоге получает хост.

Тест закрепляет обе стороны и заодно то, что упакованный селектор зовёт именно
`run`: без этого запись реестра описывала бы путь, которым продукт не ходит.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MAIN_RS = REPO_ROOT / "crates" / "unica-bootstrap" / "src" / "main.rs"
VERIFICATION_RS = REPO_ROOT / "crates" / "unica-bootstrap" / "src" / "verification.rs"
LAUNCHER = REPO_ROOT / "plugins" / "unica" / "bootstrap" / "launch.sh"


def command_arm(source: str, variant: str) -> str:
    """Тело ветки `Command::<variant>` из `match` в `run`."""
    match = re.search(
        rf"Command::{variant} => \{{(.*?)\n        \}}", source, re.DOTALL
    )
    if match is None:
        raise AssertionError(f"ветка Command::{variant} не найдена в {MAIN_RS}")
    return match.group(1)


class BootstrapLaunchPathTests(unittest.TestCase):
    def setUp(self) -> None:
        self.main = MAIN_RS.read_text(encoding="utf-8")

    def test_packaged_launcher_starts_the_run_command(self) -> None:
        launcher = LAUNCHER.read_text(encoding="utf-8")

        self.assertIn('exec "$bootstrap" run --plugin-root "$plugin_root"', launcher)
        self.assertNotIn("verify", launcher)

    def test_run_hands_the_runtime_off_without_a_handshake(self) -> None:
        arm = command_arm(self.main, "Run")

        self.assertIn("launch_runtime", arm)
        self.assertNotIn("verify", arm)

    def test_verify_performs_the_handshake_within_a_fixed_budget(self) -> None:
        arm = command_arm(self.main, "Verify")
        self.assertIn("install_and_verify_runtime", arm)

        body = re.search(
            r"fn install_and_verify_runtime\(.*?\n\}", self.main, re.DOTALL
        )
        self.assertIsNotNone(body, "install_and_verify_runtime не найдена")
        self.assertIn("verify_mcp_runtime", body.group())
        self.assertRegex(
            body.group(),
            r"Duration::from_secs\(\d+\)",
            "бюджет рукопожатия должен быть явным числом секунд",
        )

    def test_the_handshake_asks_for_initialize_and_tools_list(self) -> None:
        verification = VERIFICATION_RS.read_text(encoding="utf-8")

        self.assertIn('"method": "initialize"', verification)
        self.assertIn('"method": "tools/list"', verification)


if __name__ == "__main__":
    unittest.main()
