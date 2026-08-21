from __future__ import annotations

import importlib.util
import json
import re
import tomllib
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
    def test_public_runtime_binary_name_is_unica(self) -> None:
        manifest = tomllib.loads(
            (REPO_ROOT / "crates/unica-coder/Cargo.toml").read_text(encoding="utf-8")
        )
        lock = json.loads(
            (
                REPO_ROOT / "plugins/unica/third-party/tools.lock.json"
            ).read_text(encoding="utf-8")
        )
        public_tools = [tool for tool in lock["tools"] if tool["name"] == "unica"]

        self.assertEqual(
            manifest["bin"],
            [{"name": "unica", "path": "src/main.rs"}],
        )
        self.assertEqual(len(public_tools), 1)
        self.assertEqual(
            {
                key: public_tools[0][key]
                for key in ("binaryName", "cargoPackage", "cargoBin")
            },
            {
                "binaryName": "unica",
                "cargoPackage": "unica-coder",
                "cargoBin": "unica",
            },
        )

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
        self.assertRegex(next(iter(values.values())), load_module().RELEASE_VERSION)

    def test_meta_surface_delivery_is_versioned_across_the_012_line(self) -> None:
        module = load_module()
        values = module.read_version_contract(REPO_ROOT)
        lock = (REPO_ROOT / "Cargo.lock").read_text(encoding="utf-8")
        workspace_packages = {}
        for block in lock.split("[[package]]"):
            name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
            version = re.search(r'^version = "([^"]+)"$', block, re.MULTILINE)
            if name and version and name.group(1) in {"unica-bootstrap", "unica-coder"}:
                workspace_packages[name.group(1)] = version.group(1)

        # Ветка сопровождения выпускает исправительные версии одну за другой,
        # поэтому пин держит линию поставки и согласие всех мест, а не один
        # конкретный патч.
        delivered = set(values.values())
        self.assertEqual(len(delivered), 1, values)
        version = next(iter(delivered))
        # Пин держит линию поставки; суффикс предвыпуска ей не противоречит.
        self.assertRegex(version, r"^0\.12\.\d+(?:-[0-9A-Za-z.]+)?$")
        self.assertEqual(
            workspace_packages,
            {"unica-bootstrap": version, "unica-coder": version},
        )

    def test_a_prerelease_is_a_legal_release_version(self) -> None:
        """Ранбук описывает предвыпуск, и контракт обязан его пропускать.

        Замерить доставку можно только на настоящем релизе: адрес ассета прибит
        к тегу репозитория. Запрет суффикса делал описанную процедуру
        невыполнимой.
        """
        module = load_module()

        for version in ("0.13.0-rc.1", "0.13.0-beta.2", "0.13.0-alpha-1", "1.0.0"):
            with self.subTest(version=version):
                self.assertRegex(version, module.RELEASE_VERSION)

    def test_a_version_that_is_not_a_release_is_refused(self) -> None:
        # Разрешить суффикс — не значит разрешить что угодно: контракт остаётся
        # тем, кто ловит мусор в пяти файлах сразу.
        module = load_module()

        for version in ("0.13", "banana", "0.13.0-", "v0.13.0", "0.13.0 rc1"):
            with self.subTest(version=version):
                self.assertNotRegex(version, module.RELEASE_VERSION)

    def test_the_contract_refuses_a_malformed_version_everywhere(self) -> None:
        module = load_module()

        errors = module.validate_version_contract({"cargo": "banana", "plugin": "banana"})

        self.assertTrue(any("banana" in error for error in errors), errors)

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




class PrereleaseVersionTests(unittest.TestCase):
    """Предвыпуск обязан быть версией, а не пометкой рядом с ней.

    Манифест поставки требует, чтобы тег совпадал с версией плагина буквально:
    `tag == "v" + pluginVersion`. Значит выпуск, который не должен доехать до
    пользователей, отличается именно версией — суффиксом по SemVer, — и контракт
    версий обязан его принимать.
    """

    def bumper(self):
        import importlib.util

        path = REPO_ROOT / "scripts" / "dev" / "bump-version.py"
        spec = importlib.util.spec_from_file_location("bump_version", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_a_prerelease_suffix_is_a_valid_version(self) -> None:
        semver = self.bumper().SEMVER
        for version in (
            "0.13.0-rc.1",
            "1.0.0-probe.2",
            "0.13.0-delivery.1",
            "0.13.0-alpha-1",
        ):
            self.assertTrue(semver.fullmatch(version), version)

    def test_bumper_and_gate_accept_the_same_version_grammar(self) -> None:
        gate = load_module().RELEASE_VERSION
        bumper = self.bumper().SEMVER
        corpus = (
            "0.13.0",
            "0.13.0-alpha-1",
            "0.13.0-rc.1",
            "0.13.0-",
            "0.13.0-alpha..1",
            "0.13.0+build.1",
        )
        self.assertEqual(
            {version: bool(gate.fullmatch(version)) for version in corpus},
            {version: bool(bumper.fullmatch(version)) for version in corpus},
        )

    def test_a_plain_version_stays_valid(self) -> None:
        semver = self.bumper().SEMVER
        for version in ("0.13.0", "1.2.3"):
            self.assertTrue(semver.fullmatch(version), version)

    def test_what_is_not_a_version_is_still_refused(self) -> None:
        semver = self.bumper().SEMVER
        for version in ("0.13", "v0.13.0", "0.13.0-", "0.13.0 rc1", "next"):
            self.assertIsNone(semver.fullmatch(version), version)


if __name__ == "__main__":
    unittest.main()
