from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "test-unica-upgrade.ps1"


class UnicaUpgradeScriptContractTests(unittest.TestCase):
    def script_text(self) -> str:
        return SCRIPT.read_text(encoding="utf-8")

    def test_harness_uses_explicit_artifacts_and_isolated_codex_home(self) -> None:
        text = self.script_text()

        for parameter in (
            "$CodexPath",
            "$LegacyMarketplaceRoot",
            "$CandidatePluginRoot",
            "$ReportPath",
            "$LegacyManagedName",
        ):
            self.assertIn(parameter, text)
        self.assertIn('[ValidateSet("Preflight", "Full")]', text)
        self.assertIn('[ValidateSet("unica-local", "unica")]', text)
        self.assertIn("[System.IO.Path]::GetTempPath()", text)
        self.assertIn('$env:CODEX_HOME = $codexHome', text)
        self.assertIn("Copy-Item", text)
        self.assertNotIn("Invoke-WebRequest", text)
        self.assertNotIn("gh release download", text)

    def test_harness_proves_legacy_state_preflight_full_migration_and_idempotency(self) -> None:
        text = self.script_text()

        self.assertIn('codex-cli 0.145.0-alpha.18', text)
        self.assertIn('"plugin", "marketplace", "add"', text)
        self.assertIn('$legacyMarketplaceName = "unica"', text)
        self.assertIn('$legacyPluginSelector = "unica@$legacyMarketplaceName"', text)
        self.assertIn('"plugin", "add", $legacyPluginSelector', text)
        self.assertIn('$_.pluginId -eq $legacyPluginSelector', text)
        self.assertIn('-notcontains $legacyPluginSelector', text)
        self.assertIn('-notcontains $legacyMarketplaceName', text)
        self.assertIn('"migrate-preflight"', text)
        self.assertIn('"migrate"', text)
        self.assertIn('$candidateRef = "v$candidateVersion"', text)
        self.assertGreaterEqual(text.count('"--marketplace-ref", $candidateRef'), 3)
        self.assertIn('"plugin", "list", "--available", "--json"', text)
        self.assertIn("removePluginIds", text)
        self.assertIn("addCanonicalMarketplace", text)
        self.assertIn("changed", text)
        self.assertIn("idempotent", text)
        self.assertIn("ConvertTo-Json", text)
        self.assertIn("legacyManagedName", text)
        self.assertIn('$legacyManagedPathContract = "marketplaces/$LegacyManagedName"', text)
        self.assertIn("legacyManagedPathContract", text)
        self.assertIn("legacyMarketplaceName", text)
        self.assertIn("legacyPluginSelector", text)


if __name__ == "__main__":
    unittest.main()
