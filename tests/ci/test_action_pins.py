"""Действия workflow закреплены хешем, checkout не оставляет токен в дереве.

Находки zizmor едут в Code Scanning и гейт не красят — поэтому то, что
однажды вычищено, держит обычный тест: чужое действие называется хешем
коммита с версией в комментарии, своё — веткой; каждый `checkout` снимает
`persist-credentials`, а Dependabot двигает хеши вместе с комментарием.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml"))
# Свои составные действия: их шаги зовут чужие действия так же, как workflow.
LOCAL_ACTIONS = sorted((REPO_ROOT / ".github" / "actions").glob("*/action.yml"))
OWN_ORG = "IngvarConsulting/"
USES_LINE = re.compile(r"^\s*(?:- )?uses:\s*(?P<spec>.*?)\s*$")
PINNED = re.compile(r"^(?P<action>\S+)@(?P<ref>\S+)(?:\s+#\s*(?P<version>\S+).*)?$")


def steps(workflow: dict) -> list[dict]:
    return [step for job in workflow["jobs"].values() for step in job.get("steps", [])]


class ActionPinTests(unittest.TestCase):
    def test_every_foreign_action_is_pinned_to_a_commit_with_its_version_named(self) -> None:
        self.assertTrue(LOCAL_ACTIONS)
        for path in WORKFLOWS + LOCAL_ACTIONS:
            for line in path.read_text(encoding="utf-8").splitlines():
                uses = USES_LINE.match(line)
                if not uses:
                    continue
                # Свой workflow в синтаксисе своего репозитория: чужого кода нет.
                if uses["spec"].startswith("$/.github/workflows/"):
                    continue
                # Своё составное действие из этого дерева: его шаги проверяются здесь же.
                if uses["spec"].startswith("./.github/actions/"):
                    action = REPO_ROOT / uses["spec"].removeprefix("./") / "action.yml"
                    self.assertIn(action, LOCAL_ACTIONS, line)
                    continue
                # Строка с `uses:`, которую страж не разобрал, — отказ, а не пропуск:
                # иначе лишний пробел перед комментарием выводил бы пин из-под проверки.
                found = PINNED.match(uses["spec"])
                self.assertIsNotNone(found, f"строка не разобрана: {line.strip()}")
                assert found is not None
                with self.subTest(workflow=path.name, action=found["action"]):
                    if found["action"].startswith(OWN_ORG):
                        continue
                    self.assertRegex(found["ref"], r"^[0-9a-f]{40}$", line)
                    self.assertIsNotNone(found["version"], line)

    def test_rust_comes_from_the_pinned_file_through_the_own_action(self) -> None:
        # Плавающий `stable` и чужое действие каждый прогон спрашивали
        # static.rust-lang.org: на раннерах PVE обрыв загрузки манифеста
        # выбивал PR из очереди (#3269). Версию называет только
        # rust-toolchain.toml, ставит её своё действие.
        toolchain = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
        self.assertRegex(toolchain, r'(?m)^channel = "\d+\.\d+\.\d+"$')
        self.assertRegex(toolchain, r'(?m)^components = \[.*"rustfmt".*"clippy".*\]$')
        uses = 0
        for path in WORKFLOWS:
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("dtolnay/rust-toolchain", text, path.name)
            self.assertNotRegex(text, r"toolchain:\s*stable", path.name)
            for step in steps(yaml.safe_load(text)):
                if step.get("uses") == "./.github/actions/rust-toolchain":
                    uses += 1
                    self.assertNotIn("with", step, path.name)
        self.assertEqual(uses, 11)

    def test_the_rust_action_stays_offline_on_pve_and_retries_on_github(self) -> None:
        action = yaml.safe_load(
            (REPO_ROOT / ".github" / "actions" / "rust-toolchain" / "action.yml").read_text(encoding="utf-8")
        )
        by_if: dict[str, list[dict]] = {}
        for step in action["runs"]["steps"]:
            by_if.setdefault(step.get("if", ""), []).append(step)
        (pve,) = by_if["runner.environment == 'self-hosted'"]
        (hosted,) = by_if["runner.environment != 'self-hosted'"]
        # На PVE версия стоит заранее: шаг только проверяет и не скачивает.
        self.assertNotIn("toolchain install", pve["run"].split("echo", 1)[0])
        self.assertNotIn("curl", pve["run"])
        self.assertEqual(pve["env"]["RUSTUP_AUTO_INSTALL"], "0")
        # В рабочей копии rustup доставил бы компоненты из rust-toolchain.toml.
        self.assertEqual(pve["working-directory"], "${{ runner.temp }}")
        self.assertIn("exit 1", pve["run"])
        # На GitHub-hosted любой сбой установки повторяется.
        self.assertIn("rustup toolchain install", hosted["run"])
        self.assertIn("for attempt in 1 2 3 4 5", hosted["run"])
        self.assertIn("sleep", hosted["run"])
        # Общие шаги после установки тоже не зовут сеть.
        for step in by_if[""]:
            self.assertNotIn("toolchain install", step["run"])
        # `CARGO*` входят в ключ rust-cache: те же переменные, что ставил dtolnay.
        exported = "\n".join(step["run"] for step in by_if[""])
        for variable in ("CARGO_HOME", "CARGO_INCREMENTAL=0", "CARGO_TERM_COLOR=always"):
            self.assertIn(variable, exported)

    def test_every_checkout_leaves_no_token_in_the_tree(self) -> None:
        for path in WORKFLOWS:
            workflow = yaml.safe_load(path.read_text(encoding="utf-8"))
            for step in steps(workflow):
                uses = step.get("uses", "")
                if not uses.startswith("actions/checkout@"):
                    continue
                with self.subTest(workflow=path.name):
                    self.assertIs(step.get("with", {}).get("persist-credentials"), False)

    def test_dependabot_moves_the_pins_and_zizmor_knows_the_policy(self) -> None:
        dependabot = yaml.safe_load((REPO_ROOT / ".github" / "dependabot.yml").read_text(encoding="utf-8"))
        ecosystems = {update["package-ecosystem"] for update in dependabot["updates"]}
        self.assertIn("github-actions", ecosystems)

        policy = yaml.safe_load((REPO_ROOT / ".github" / "zizmor.yml").read_text(encoding="utf-8"))
        policies = policy["rules"]["unpinned-uses"]["config"]["policies"]
        self.assertEqual(policies["*"], "hash-pin")
        self.assertEqual(policies[OWN_ORG + "*"], "ref-pin")


if __name__ == "__main__":
    unittest.main()
