"""Действия workflow закреплены хешем, checkout не оставляет токен в дереве.

Находки zizmor едут в Code Scanning и гейт не красят — поэтому то, что
однажды вычищено, держит обычный тест: чужое действие называется хешем
коммита с версией в комментарии, своё — веткой; каждый `checkout` снимает
`persist-credentials`, а Dependabot двигает хеши вместе с комментарием.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import textwrap
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


def posix_bash() -> str | None:
    """bash, исполняющий шаги действий: на Windows — Git Bash, а не WSL из System32."""
    if os.name == "nt":
        git = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git" / "bin" / "bash.exe"
        return str(git) if git.is_file() else None
    return shutil.which("bash")


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
        # На PVE версия стоит заранее: шаг только проверяет и не скачивает
        # (поведение проверяет `test_the_pve_check_never_installs`).
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

    @unittest.skipUnless(posix_bash(), "нужен bash")
    def test_the_pve_check_never_installs(self) -> None:
        """Проверка на PVE отвечает по установленному и ничего не ставит."""
        action = yaml.safe_load(
            (REPO_ROOT / ".github" / "actions" / "rust-toolchain" / "action.yml").read_text(encoding="utf-8")
        )
        (pve,) = [step for step in action["runs"]["steps"] if step.get("if") == "runner.environment == 'self-hosted'"]
        stub = textwrap.dedent(
            """\
            #!/usr/bin/env bash
            echo "$*" >> "$RUSTUP_CALLS"
            case "$*" in
              "toolchain list") printf '%s\\n' $STUB_TOOLCHAINS ;;
              "component list --installed --toolchain 1.100.0") printf '%s\\n' $STUB_COMPONENTS ;;
              *) exit 3 ;;
            esac
            """
        )
        cases = [
            ("1.100.0-x86_64-unknown-linux-gnu", "rustfmt-x86_64-unknown-linux-gnu clippy-x86_64-unknown-linux-gnu", 0),
            ("1.100.0-x86_64-unknown-linux-gnu", "rustfmt-x86_64-unknown-linux-gnu", 1),
            ("stable-x86_64-unknown-linux-gnu", "", 1),
        ]
        bash = posix_bash()
        assert bash is not None
        for toolchains, components, expected in cases:
            with self.subTest(toolchains=toolchains, components=components), tempfile.TemporaryDirectory() as temp:
                bin_dir = Path(temp) / "bin"
                bin_dir.mkdir()
                (bin_dir / "rustup").write_text(stub, encoding="utf-8", newline="\n")
                (bin_dir / "rustup").chmod(0o755)
                script = Path(temp) / "pve.sh"
                script.write_text(pve["run"], encoding="utf-8", newline="\n")
                calls = Path(temp) / "calls"
                calls.touch()
                env = dict(os.environ, toolchain="1.100.0", components="rustfmt,clippy", RUSTUP_AUTO_INSTALL="0",
                           RUNNER_NAME="stub", RUSTUP_CALLS=str(calls), STUB_TOOLCHAINS=toolchains,
                           STUB_COMPONENTS=components, STUB_BIN=str(bin_dir))
                # Git Bash на Windows понимает PATH только в своей записи пути.
                result = subprocess.run(
                    [bash, "-c", 'PATH="$(cygpath -u "$STUB_BIN" 2>/dev/null || echo "$STUB_BIN"):$PATH" '
                                 'exec bash "$0"', str(script)],
                    env=env, capture_output=True, text=True, cwd=temp,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                recorded = calls.read_text(encoding="utf-8").splitlines()
                self.assertTrue(recorded, result.stderr)
                for call in recorded:
                    self.assertNotRegex(call, r"\b(install|add|update)\b", recorded)
                if expected:
                    self.assertIn("rustup toolchain install 1.100.0", result.stderr)

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
