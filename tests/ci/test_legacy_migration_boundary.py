from __future__ import annotations

import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class LegacyMigrationBoundaryTests(unittest.TestCase):
    def test_v080_source_tree_has_no_executable_legacy_migration_surface(self) -> None:
        removed_paths = (
            "crates/unica-bootstrap/src/migration.rs",
            "crates/unica-bootstrap/src/codex.rs",
            "crates/unica-bootstrap/tests/migration_contract.rs",
            "crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/README.md",
            "crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/marketplaces-local.json",
            "crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/metadata.json",
            "crates/unica-bootstrap/tests/fixtures/codex-0.145.0-alpha.18/plugins-installed.json",
            "crates/unica-bootstrap/tests/fixtures/marketplaces-empty.json",
            "crates/unica-bootstrap/tests/fixtures/plugins-empty.json",
            "scripts/ci/capture-codex-contract.py",
            "scripts/ci/test-unica-upgrade.ps1",
            "scripts/install-unica.sh",
            "scripts/install-unica.ps1",
            "tests/ci/test_capture_codex_contract.py",
            "tests/ci/test_install_unica_script.py",
            "tests/ci/test_unica_upgrade_script.py",
        )

        remaining = [path for path in removed_paths if (REPO_ROOT / path).exists()]
        self.assertEqual([], remaining)

    def test_bootstrap_and_release_workflow_expose_only_current_runtime_paths(self) -> None:
        main = (REPO_ROOT / "crates/unica-bootstrap/src/main.rs").read_text(
            encoding="utf-8"
        )
        library = (REPO_ROOT / "crates/unica-bootstrap/src/lib.rs").read_text(
            encoding="utf-8"
        )
        release = (
            REPO_ROOT / ".github/workflows/unica-plugin-release.yml"
        ).read_text(encoding="utf-8")

        for marker in (
            "MigrationEngine",
            "MigratePreflight",
            "migrate-preflight",
            "Command::Migrate",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, main + library)
        for marker in (
            "CodexDiscovery",
            "MarketplaceRecord",
            "PluginRecord",
            "pub fn discover",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, library)
        for marker in (
            "legacy-migration-preflight:",
            "  installer:",
            "install-unica.sh",
            "install-unica.ps1",
            "test-unica-upgrade.ps1",
            "unica-installer",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, release)

    def test_post_v080_pre_v1_metadata_keeps_only_the_frozen_v078_bridge(self) -> None:
        metadata = json.loads(
            (REPO_ROOT / "plugins/unica/.codex-plugin/plugin.json").read_text(
                encoding="utf-8"
            )
        )
        readme = (REPO_ROOT / "README.md").read_text(encoding="utf-8")
        plugin_readme = (REPO_ROOT / "plugins/unica/README.md").read_text(
            encoding="utf-8"
        )
        internal_package = (
            REPO_ROOT / "plugins/unica/references/tooling/internal-package.md"
        ).read_text(encoding="utf-8")
        marketplace_adr = (
            REPO_ROOT / "spec/decisions/0008-public-marketplace-thin-runtime.md"
        ).read_text(encoding="utf-8")
        version = metadata["version"]
        self.assertRegex(version, r"^0\.\d+\.\d+$")
        self.assertGreaterEqual(tuple(map(int, version.split("."))), (0, 8, 0))
        for filename in ("install-unica.sh", "install-unica.ps1"):
            frozen_url = f"releases/download/v0.7.8/{filename}"
            obsolete_url = f"releases/download/v0.7.7/{filename}"
            with self.subTest(filename=filename):
                self.assertIn(frozen_url, readme)
                self.assertIn(frozen_url, plugin_readme)
                self.assertNotIn(obsolete_url, readme)
                self.assertNotIn(obsolete_url, plugin_readme)
        for text in (internal_package, marketplace_adr):
            self.assertIn("v0.7.8", text)
            self.assertIn("v0.8.0", text)
            self.assertIn("ordinary marketplace update", text)


if __name__ == "__main__":
    unittest.main()
