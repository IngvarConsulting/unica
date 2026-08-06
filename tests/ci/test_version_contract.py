from __future__ import annotations

import importlib.util
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "check-version-contract.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_version_contract", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class VersionContractTests(unittest.TestCase):
    def test_every_contract_location_declares_the_same_version(self) -> None:
        module = load_module()

        values = module.read_version_contract(REPO_ROOT)

        # Named rather than pinned to a literal: the contract is that the four
        # locations agree, and asserting the number here only added a file every
        # release had to come back and edit.
        self.assertEqual(
            sorted(values),
            ["claude-plugin", "plugin", "tools-lock-unica", "workspace"],
        )
        self.assertEqual(len(set(values.values())), 1, values)
        self.assertRegex(next(iter(values.values())), r"^\d+\.\d+\.\d+$")

    def test_meta_surface_delivery_is_versioned_as_0120_everywhere(self) -> None:
        module = load_module()
        values = module.read_version_contract(REPO_ROOT)
        lock = (REPO_ROOT / "Cargo.lock").read_text(encoding="utf-8")
        workspace_packages = {}
        for block in lock.split("[[package]]"):
            name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
            version = re.search(r'^version = "([^"]+)"$', block, re.MULTILINE)
            if name and version and name.group(1) in {"unica-bootstrap", "unica-coder"}:
                workspace_packages[name.group(1)] = version.group(1)

        self.assertEqual(set(values.values()), {"0.12.0"}, values)
        self.assertEqual(
            workspace_packages,
            {"unica-bootstrap": "0.12.0", "unica-coder": "0.12.0"},
        )

    def test_0120_meta_migration_is_complete_and_linked(self) -> None:
        migration_index = REPO_ROOT / "docs/migrations/README.md"
        migration_note = REPO_ROOT / "docs/migrations/0.12.0-meta-surface.md"
        self.assertTrue(migration_index.is_file(), migration_index)
        self.assertTrue(migration_note.is_file(), migration_note)

        index = migration_index.read_text(encoding="utf-8")
        note = migration_note.read_text(encoding="utf-8")
        root_readme = (REPO_ROOT / "README.md").read_text(encoding="utf-8")
        required_mapping = (
            (
                "meta.compile",
                "meta.add.operations[] only for ledger-supported capabilities",
            ),
            ("meta.profile", "meta.info.usage / meta.info.predefinedItems"),
            (
                "meta.validate",
                "meta.info.validation / automatic mutation validation",
            ),
            ("ObjectPath", "sourceSet + metadataPath"),
            ("ConfigDir + Object", "sourceSet + metadataPath"),
            ("Operation + Value", "operations[]"),
            ("DefinitionFile", "removed"),
        )

        self.assertIn("[0.12.0", index)
        self.assertIn("0.12.0-meta-surface.md", index)
        self.assertIn("docs/migrations/README.md", root_readme)
        documented_mapping = tuple(
            tuple(part.strip() for part in line.split("->", 1))
            for line in note.splitlines()
            if "->" in line
        )
        self.assertEqual(documented_mapping, required_mapping)
        for fragment in ("sourceSet", "kind", "name", "dryRun"):
            self.assertIn(fragment, note)
        self.assertIn("operations[]", note)
        self.assertIn("clean break", note.lower())
        self.assertIn(
            "`meta.add` не принимает прежнюю нагрузку определения из `meta.compile`",
            " ".join(note.split()),
        )

    def test_mismatch_names_the_contract_field(self) -> None:
        module = load_module()

        errors = module.validate_version_contract(
            {"workspace": "0.7.0", "plugin": "0.6.1"},
            expected="0.7.0",
        )

        self.assertEqual(errors, ["plugin version 0.6.1 != expected 0.7.0"])


if __name__ == "__main__":
    unittest.main()
