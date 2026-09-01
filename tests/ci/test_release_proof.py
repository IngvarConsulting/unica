from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = REPO_ROOT / "tests" / "fixtures" / "migration" / "v0.12.3-baseline.json"


def load_module():
    module_path = REPO_ROOT / "scripts" / "ci" / "release-proof.py"
    spec = importlib.util.spec_from_file_location("release_proof", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.baseline = json.loads(BASELINE_PATH.read_text(encoding="utf-8"))
        self.native_names = {
            "unica.view",
            "unica.apply",
            "unica.find",
            "unica.search",
            "unica.check",
            "unica.diff",
            "unica.run",
            "unica.docs",
        }
        self.compatibility_names = self.native_names | {
            "unica.task.get",
            "unica.task.result",
            "unica.task.cancel",
        }

    def wire(self, profile: str, names: set[str]) -> dict:
        return {
            "schemaVersion": 1,
            "profile": profile,
            "protocolVersion": "2026-07-28" if profile == "native" else "2025-06-18",
            "serverProtocolVersion": "2026-07-28" if profile == "native" else "2025-06-18",
            "serverInfo": {"name": "unica", "version": "0.12.0"},
            "tasksCapability": "on" if profile == "native" else "off",
            "toolCount": len(names),
            "toolNames": sorted(names),
        }

    def assessment(self, *, lifecycle: dict[str, dict] | None = None) -> dict:
        return {
            "schemaVersion": 1,
            "summary": {"status": "passed"},
            "scenarios": [{"id": "mcp-tools-list", "status": "passed"}],
            "lifecycle": lifecycle or {
                name: {
                    "status": "deferred",
                    "supported": False,
                    "evidence": [f"dry:{name}"],
                }
                for name in self.module.LIFECYCLE_SCENARIOS
            },
        }

    def package(self, **overrides: object) -> dict:
        package = {
            "schemaVersion": 1,
            "pluginVersion": "0.12.0",
            "sourceCommit": "a" * 40,
            "packageSha256": "b" * 64,
            "runtimeManifestSha256": "c" * 64,
            "versionBumped": False,
            "published": False,
            "tag": None,
        }
        package.update(overrides)
        return package

    def evaluate(self, **overrides: object) -> dict:
        values = {
            "native_wire": self.wire("native", self.native_names),
            "compatibility_wire": self.wire("compatibility", self.compatibility_names),
            "baseline": self.baseline,
            "assessment": self.assessment(),
            "package": self.package(),
            "release_tag": "v0.12.0",
            "mode": "dry",
        }
        values.update(overrides)
        return self.module.evaluate_proof(**values)

    def test_dry_proof_accepts_exact_profiles_and_records_all_lifecycle_outcomes(self) -> None:
        report = self.evaluate()

        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["surfaces"]["native"]["toolCount"], 8)
        self.assertEqual(report["surfaces"]["compatibility"]["toolCount"], 11)
        self.assertEqual(report["legacyBaseline"]["overlap"], [])
        self.assertEqual(
            set(report["lifecycle"]),
            {
                "fresh_install",
                "upgrade",
                "offline_prefetch",
                "restart",
                "rollback",
            },
        )
        self.assertTrue(all(item["status"] == "deferred" for item in report["lifecycle"].values()))
        self.assertEqual(
            report["guards"],
            {"noVersionBump": True, "noTag": True, "noPublication": True},
        )

    def test_proof_rejects_any_v0123_tool_name_in_the_rc_surface(self) -> None:
        baseline = json.loads(json.dumps(self.baseline))
        baseline["wire"]["toolNames"] = [
            "unica.view",
            *baseline["wire"]["toolNames"][1:],
        ]

        with self.assertRaisesRegex(self.module.ProofError, "legacy baseline overlap.*unica.view"):
            self.evaluate(baseline=baseline)

    def test_proof_rejects_non_string_legacy_tool_names(self) -> None:
        baseline = json.loads(json.dumps(self.baseline))
        baseline["wire"]["toolNames"][0] = None

        with self.assertRaisesRegex(self.module.ProofError, "74 unique tool names"):
            self.evaluate(baseline=baseline)

    def test_rc_proof_rejects_deferred_lifecycle_outcomes(self) -> None:
        lifecycle = self.assessment()["lifecycle"]
        for name in ("fresh_install", "upgrade"):
            lifecycle[name] = {
                "status": "passed",
                "supported": True,
                "evidence": [f"rc:{name}"],
            }
        with self.assertRaisesRegex(self.module.ProofError, "offline_prefetch.*deferred"):
            self.evaluate(mode="rc", assessment=self.assessment(lifecycle=lifecycle))

    def test_dry_proof_rejects_version_tag_or_publication_mutation(self) -> None:
        with self.assertRaisesRegex(self.module.ProofError, "must not publish"):
            self.evaluate(package=self.package(published=True))

        with self.assertRaisesRegex(self.module.ProofError, "must not create a tag"):
            self.evaluate(package=self.package(tag="v0.12.0"))

    def test_prerelease_is_explicitly_non_promotable(self) -> None:
        report = self.evaluate(release_tag="v0.13.0-rc.1")

        self.assertEqual(
            report["promotion"],
            {
                "releaseTag": "v0.13.0-rc.1",
                "promote": False,
                "reason": "P0 proof is never a publication or promotion action",
            },
        )


if __name__ == "__main__":
    unittest.main()
