from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


def load_build_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "build-unica-tools.py"
    spec = importlib.util.spec_from_file_location("build_unica_tools", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuildUnicaToolsTests(unittest.TestCase):
    def assert_external_toolchain_contract(self, external_tools: list[dict]) -> None:
        expected_names = {
            "bsl-analyzer",
            "v8-runner",
            "rlm-tools-bsl",
            "rlm-bsl-index",
        }
        self.assertEqual({tool["name"] for tool in external_tools}, expected_names)

        tags_by_source: dict[tuple[str, str, str], str] = {}
        for tool in external_tools:
            source_identity = (
                tool["repository"],
                tool["sourceTag"],
                tool["sourceCommit"],
            )
            release_tag, separator, build_revision = tool["assetTag"].rpartition("-build.")
            self.assertEqual(separator, "-build.")
            self.assertRegex(build_revision, r"^[1-9][0-9]*$")

            source_suffix = f"-{tool['sourceTag']}"
            self.assertTrue(release_tag.endswith(source_suffix))
            release_name = release_tag[: -len(source_suffix)]
            self.assertTrue(
                any(
                    candidate["name"] == release_name
                    and (
                        candidate["repository"],
                        candidate["sourceTag"],
                        candidate["sourceCommit"],
                    )
                    == source_identity
                    for candidate in external_tools
                )
            )

            existing_tag = tags_by_source.setdefault(source_identity, tool["assetTag"])
            self.assertEqual(tool["assetTag"], existing_tag)
            self.assertEqual(tool["assetStrategy"], "direct-release-asset")
            self.assertEqual(
                tool["assetRepository"], "https://github.com/IngvarConsulting/unica-toolchain"
            )
            for target, asset in tool["assets"].items():
                exe = ".exe" if target == "win-x64" else ""
                self.assertEqual(asset["assetName"], f"{tool['binaryName']}-{target}{exe}")
                self.assertRegex(asset["sha256"], r"^[0-9a-f]{64}$")
                self.assertNotIn("archiveBinary", asset)

        self.assertEqual(len(set(tags_by_source.values())), len(tags_by_source))

    def test_release_asset_url_can_differ_from_upstream_source(self) -> None:
        module = load_build_module()

        url = module.release_asset_url(
            {
                "repository": "https://github.com/example/upstream",
                "sourceTag": "v1.2.3",
                "assetRepository": "https://github.com/IngvarConsulting/unica-toolchain",
                "assetTag": "example-v1.2.3-build.7",
            },
            {"assetName": "example-linux-x64"},
        )

        self.assertEqual(
            url,
            "https://github.com/IngvarConsulting/unica-toolchain/releases/download/"
            "example-v1.2.3-build.7/example-linux-x64",
        )

    def test_all_checked_in_external_tools_use_independent_toolchain_assets(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        lock = json.loads(
            (repo_root / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        external_tools = [tool for tool in lock["tools"] if tool["name"] != "unica"]
        self.assert_external_toolchain_contract(external_tools)

        updated_tools = json.loads(json.dumps(external_tools))
        for tool in updated_tools:
            version = tool["sourceTag"].removeprefix("v").split(".")
            version[-1] = str(int(version[-1]) + 1)
            tool["sourceTag"] = f"v{'.'.join(version)}"
            release_name = tool["assetTag"].rsplit("-v", 1)[0]
            build_revision = int(tool["assetTag"].rsplit("-build.", 1)[1]) + 1
            tool["assetTag"] = f"{release_name}-{tool['sourceTag']}-build.{build_revision}"
        self.assert_external_toolchain_contract(updated_tools)

    def test_release_asset_checksum_mismatch_fails_before_use(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            downloaded = Path(tmp) / "asset.bin"
            downloaded.write_bytes(b"unexpected")

            with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
                module.verify_asset_checksum(
                    downloaded,
                    {"assetName": "asset.bin", "sha256": "0" * 64},
                    tool_name="v8-runner",
                    target="linux-x64",
                )

    def test_bundle_builder_has_no_archive_asset_dependency_path(self) -> None:
        module = load_build_module()
        source = (
            Path(__file__).resolve().parents[2] / "scripts" / "ci" / "build-unica-tools.py"
        ).read_text(encoding="utf-8")

        self.assertFalse(hasattr(module, "extract_v8_runner"))
        self.assertNotIn("archive-release-asset", source)
        self.assertNotIn("archiveBinary", source)

    def test_cargo_workspace_tool_builds_from_repo_root(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            repo_root.mkdir()
            out_dir = root / "out"
            out_dir.mkdir()
            target_dir = root / "cargo-target"
            produced = target_dir / "release" / "unica"
            produced.parent.mkdir(parents=True)
            produced.write_bytes(b"rust mcp")
            calls = []

            def fake_run(args, *, cwd=None):
                calls.append((args, cwd))

            with patch.object(module, "run", side_effect=fake_run):
                dest = module.build_cargo_workspace_tool(
                    {
                        "name": "unica",
                        "binaryName": "unica",
                        "cargoPackage": "unica-coder",
                        "cargoBin": "unica",
                    },
                    repo_root,
                    target_dir,
                    out_dir,
                    "",
                )

            self.assertEqual(dest, out_dir / "unica")
            self.assertEqual(dest.read_bytes(), b"rust mcp")
            self.assertEqual(calls[0][1], repo_root)
            self.assertEqual(
                calls[0][0],
                [
                    "cargo",
                    "build",
                    "--release",
                    "--package",
                    "unica-coder",
                    "--bin",
                    "unica",
                    "--target-dir",
                    str(target_dir),
                ],
            )

    def test_bootstrap_build_is_staged_outside_the_runtime_tool_manifest(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            repo_root.mkdir()
            bundle_root = root / "bundle"
            target_dir = root / "cargo-target"
            produced = target_dir / "release" / "unica-bootstrap.exe"
            produced.parent.mkdir(parents=True)
            produced.write_bytes(b"native bootstrap")
            calls = []

            with patch.object(module, "run", side_effect=lambda args, cwd=None: calls.append((args, cwd))):
                destination = module.build_bootstrap(
                    repo_root=repo_root,
                    target_dir=target_dir,
                    bundle_root=bundle_root,
                    target="win-x64",
                    exe=".exe",
                )

            self.assertEqual(
                destination,
                bundle_root / "bootstrap" / "bin" / "win-x64" / "unica-bootstrap.exe",
            )
            self.assertEqual(destination.read_bytes(), b"native bootstrap")
            self.assertEqual(calls[0][1], repo_root)
            self.assertIn("unica-bootstrap", calls[0][0])


if __name__ == "__main__":
    unittest.main()
