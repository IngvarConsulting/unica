from __future__ import annotations

import importlib.util
import json
import tempfile
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
        self.tempdir = tempfile.TemporaryDirectory()
        self.package_dir = Path(self.tempdir.name) / "marketplace"
        plugin_dir = self.package_dir / "plugins" / "unica"
        for host in (".codex-plugin", ".claude-plugin"):
            manifest_dir = plugin_dir / host
            manifest_dir.mkdir(parents=True, exist_ok=True)
            (manifest_dir / "plugin.json").write_text(
                json.dumps({"name": "unica", "version": "0.12.0"}),
                encoding="utf-8",
            )
        (plugin_dir / "runtime-manifest.json").write_text(
            json.dumps({"schemaVersion": 1, "tools": []}),
            encoding="utf-8",
        )
        self.asset_dir = Path(self.tempdir.name) / "asset-verification"
        self.asset_dir.mkdir()
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
            "serverInfo": (
                None
                if profile == "native"
                else {"name": "unica", "version": "0.12.0"}
            ),
            "tasksCapability": "on" if profile == "native" else "off",
            "toolCount": len(names),
            "toolNames": sorted(names),
        }

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def wire_sets(self) -> tuple[dict[str, dict], dict[str, dict]]:
        return (
            {
                target: self.wire("native", self.native_names)
                for target in self.module.TARGETS
            },
            {
                target: self.wire("compatibility", self.compatibility_names)
                for target in self.module.TARGETS
            },
        )

    def assessment(self, *, lifecycle: dict[str, dict] | None = None) -> dict:
        return {
            "schemaVersion": 1,
            "unicaVersion": "0.12.0",
            "releaseTag": "v0.12.0",
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
        version = str(overrides.get("pluginVersion", "0.12.0"))
        for host in (".codex-plugin", ".claude-plugin"):
            (self.package_dir / "plugins" / "unica" / host / "plugin.json").write_text(
                json.dumps({"name": "unica", "version": version}),
                encoding="utf-8",
            )
        package = {
            "schemaVersion": 1,
            "packageHashFormat": "sha256-u64be-path-content-v1",
            "pluginVersion": version,
            "sourceCommit": "a" * 40,
            "packageSha256": self.module.tree_sha256(self.package_dir),
            "runtimeManifestSha256": self.module.file_sha256(
                self.package_dir / "plugins" / "unica" / "runtime-manifest.json"
            ),
            "versionBumped": False,
            "published": False,
            "tag": None,
        }
        package.update(overrides)
        return package

    def asset_reports(
        self, *, source: str = "local-build", version: str = "0.12.0"
    ) -> None:
        for target in self.module.TARGETS:
            (self.asset_dir / f"asset-verification-{target}.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "status": "passed",
                        "source": source,
                        "pluginVersion": version,
                        "targets": [target],
                        "checks": {
                            "artifactSet": True,
                            "archiveChecksum": True,
                            "memberChecksums": True,
                            "memberMetadata": True,
                        },
                    }
                ),
                encoding="utf-8",
            )

    def evaluate(self, **overrides: object) -> dict:
        native_wires, compatibility_wires = self.wire_sets()
        package = overrides.get("package")
        if package is None:
            package = self.package()
        if not any(self.asset_dir.glob("asset-verification-*.json")):
            self.asset_reports(version=package["pluginVersion"])
        values = {
            "native_wires": native_wires,
            "compatibility_wires": compatibility_wires,
            "baseline": self.baseline,
            "assessment": self.assessment(),
            "package": package,
            "package_dir": self.package_dir,
            "asset_verification_dir": self.asset_dir,
            "source_commit": "a" * 40,
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

    def test_native_direct_first_proof_accepts_absent_server_info(self) -> None:
        native_wires, compatibility_wires = self.wire_sets()
        for evidence in native_wires.values():
            evidence["serverInfo"] = None

        report = self.evaluate(
            native_wires=native_wires,
            compatibility_wires=compatibility_wires,
        )

        self.assertEqual(report["status"], "passed")

    def test_proof_rejects_any_v0123_tool_name_outside_the_exact_rc_surface(self) -> None:
        legacy_name = self.baseline["wire"]["toolNames"][0]
        self.native_names.add(legacy_name)

        with self.assertRaisesRegex(
            self.module.ProofError, f"native wire surface differs.*{legacy_name}"
        ):
            self.evaluate()

    def test_proof_rejects_non_string_legacy_tool_names(self) -> None:
        baseline = json.loads(json.dumps(self.baseline))
        baseline["wire"]["toolNames"][0] = None

        with self.assertRaisesRegex(self.module.ProofError, "74 unique tool names"):
            self.evaluate(baseline=baseline)

    def test_proof_pins_the_canonical_v0123_baseline_content(self) -> None:
        baseline = json.loads(json.dumps(self.baseline))
        baseline["wire"]["toolNames"][0] = "attacker.replacement"

        with self.assertRaisesRegex(self.module.ProofError, "canonical"):
            self.evaluate(baseline=baseline)

    def test_proof_rejects_wrong_negotiated_protocol(self) -> None:
        native_wires, compatibility_wires = self.wire_sets()
        compatibility_wires["linux-x64"]["serverProtocolVersion"] = "1999-01-01"

        with self.assertRaisesRegex(self.module.ProofError, "negotiated protocol"):
            self.evaluate(
                native_wires=native_wires,
                compatibility_wires=compatibility_wires,
            )

    def test_proof_rejects_hashes_not_matching_downloaded_package(self) -> None:
        with self.assertRaisesRegex(self.module.ProofError, "packageSha256 does not match"):
            self.evaluate(package=self.package(packageSha256="b" * 64))

    def test_tree_hash_frames_path_and_content_boundaries(self) -> None:
        root = Path(self.tempdir.name)
        first = root / "first-tree"
        second = root / "second-tree"
        first.mkdir()
        second.mkdir()
        (first / "a").write_bytes(b"x\0y")
        (first / "z").write_bytes(b"w")
        (second / "a").write_bytes(b"x\0yz\0w")

        first_digest = self.module.tree_sha256(first)
        self.assertEqual(
            first_digest,
            "5188569041dcc3e6e365f6a5b95d375ba69964b3cd93a8f28c198a521c30bda2",
        )
        self.assertNotEqual(first_digest, self.module.tree_sha256(second))

    def test_proof_requires_the_framed_package_hash_format(self) -> None:
        for value in (None, "sha256-path-nul-content-v0"):
            package = self.package()
            if value is None:
                del package["packageHashFormat"]
            else:
                package["packageHashFormat"] = value
            with self.subTest(value=value):
                with self.assertRaisesRegex(self.module.ProofError, "packageHashFormat"):
                    self.evaluate(package=package)

    def test_proof_requires_assessment_release_tag_to_match_candidate(self) -> None:
        for value in (None, "646/merge"):
            assessment = self.assessment()
            if value is None:
                del assessment["releaseTag"]
            else:
                assessment["releaseTag"] = value
            with self.subTest(value=value):
                with self.assertRaisesRegex(self.module.ProofError, "assessment releaseTag"):
                    self.evaluate(assessment=assessment)

    def test_proof_requires_assessment_unica_version_to_match_candidate(self) -> None:
        for value in (None, "unknown"):
            assessment = self.assessment()
            if value is None:
                del assessment["unicaVersion"]
            else:
                assessment["unicaVersion"] = value
            with self.subTest(value=value):
                with self.assertRaisesRegex(self.module.ProofError, "assessment unicaVersion"):
                    self.evaluate(assessment=assessment)

    def test_proof_requires_all_target_wire_profiles(self) -> None:
        native_wires, compatibility_wires = self.wire_sets()
        del native_wires["win-x64"]

        with self.assertRaisesRegex(self.module.ProofError, "all targets"):
            self.evaluate(
                native_wires=native_wires,
                compatibility_wires=compatibility_wires,
            )

    def test_proof_rejects_non_boolean_asset_check(self) -> None:
        self.asset_reports()
        path = self.asset_dir / "asset-verification-linux-x64.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        report["checks"]["artifactSet"] = "false"
        path.write_text(json.dumps(report), encoding="utf-8")

        with self.assertRaisesRegex(self.module.ProofError, "checks are incomplete"):
            self.evaluate()

    def test_proof_rejects_asset_version_mismatch(self) -> None:
        self.asset_reports()
        path = self.asset_dir / "asset-verification-linux-x64.json"
        report = json.loads(path.read_text(encoding="utf-8"))
        report["pluginVersion"] = "0.99.0"
        path.write_text(json.dumps(report), encoding="utf-8")

        with self.assertRaisesRegex(self.module.ProofError, "pluginVersion does not match"):
            self.evaluate()

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
        report = self.evaluate(
            release_tag="v0.13.0-rc.1",
            package=self.package(pluginVersion="0.13.0-rc.1"),
            assessment={
                **self.assessment(),
                "unicaVersion": "0.13.0-rc.1",
                "releaseTag": "v0.13.0-rc.1",
            },
        )

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
