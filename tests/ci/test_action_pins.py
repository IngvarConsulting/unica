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
OWN_ORG = "IngvarConsulting/"
PINNED = re.compile(r"^\s*(?:- )?uses: (?P<action>\S+)@(?P<ref>\S+)(?: # (?P<version>\S+))?\s*$")


def steps(workflow: dict) -> list[dict]:
    return [step for job in workflow["jobs"].values() for step in job.get("steps", [])]


class ActionPinTests(unittest.TestCase):
    def test_every_foreign_action_is_pinned_to_a_commit_with_its_version_named(self) -> None:
        for path in WORKFLOWS:
            for line in path.read_text(encoding="utf-8").splitlines():
                found = PINNED.match(line)
                if not found:
                    continue
                with self.subTest(workflow=path.name, action=found["action"]):
                    if found["action"].startswith(OWN_ORG):
                        continue
                    self.assertRegex(found["ref"], r"^[0-9a-f]{40}$", line)
                    self.assertIsNotNone(found["version"], line)

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
