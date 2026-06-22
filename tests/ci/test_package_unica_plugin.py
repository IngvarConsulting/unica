from __future__ import annotations

import importlib.util
import json
import os
import stat
import subprocess
from unittest.mock import patch
import tempfile
import unittest
from pathlib import Path


def load_package_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "package-unica-plugin.py"
    spec = importlib.util.spec_from_file_location("package_unica_plugin", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PackageUnicaPluginTests(unittest.TestCase):
    def make_lock(self) -> dict:
        return {
            "schemaVersion": 1,
            "targets": {
                "darwin-arm64": {"targetTriple": "aarch64-apple-darwin"},
                "linux-x64": {"targetTriple": "x86_64-unknown-linux-musl"},
            },
            "tools": [
                {
                    "name": "v8-runner",
                    "version": "0.3.0",
                    "repository": "https://example.invalid/v8-runner",
                    "sourceTag": "v0.3.0",
                    "sourceCommit": "abc",
                    "license": "MIT",
                    "assets": {
                        "darwin-arm64": {"assetName": "v8-runner"},
                        "linux-x64": {"assetName": "v8-runner"},
                    },
                }
            ],
        }

    def test_source_mcp_declares_single_unica_orchestrator(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        mcp = json.loads((repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"))

        self.assertEqual(sorted(mcp["mcpServers"]), ["unica"])

        server = mcp["mcpServers"]["unica"]

        self.assertEqual(server["command"], "bash")
        self.assertIn("run-unica.sh", " ".join(server["args"]))
        self.assertIn("orchestrator", server["note"])
        self.assertNotIn("unica-coder", json.dumps(server))

    def write_bundle(self, root: Path, target: str, module) -> Path:
        bundle = root / f"unica-tools-{target}"
        bin_dir = bundle / "bin" / target
        bin_dir.mkdir(parents=True)
        binary = bin_dir / "v8-runner"
        binary.write_text(f"binary for {target}", encoding="utf-8")
        target_triples = {
            "darwin-arm64": "aarch64-apple-darwin",
            "linux-x64": "x86_64-unknown-linux-musl",
        }
        (bundle / "tools.json").write_text(
            json.dumps(
                {
                    "target": target,
                    "targetTriple": target_triples[target],
                    "tools": [
                        {
                            "name": "v8-runner",
                            "version": "0.3.0",
                            "repository": "https://example.invalid/v8-runner",
                            "upstreamUrl": "https://example.invalid/v8-runner/releases/tag/v0.3.0",
                            "sourceTag": "v0.3.0",
                            "sourceCommit": "abc",
                            "license": "MIT",
                            "targetTriple": target_triples[target],
                            "binaryPath": f"bin/{target}/v8-runner",
                            "sha256": module.sha256(binary),
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        return bundle

    def test_load_tool_bundles_allows_current_target_only_for_local_debug_package(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = self.write_bundle(root, "darwin-arm64", module)

            grouped, bin_roots = module.load_tool_bundles(root, self.make_lock(), allow_partial_targets=True)

        self.assertEqual(bin_roots, [bundle / "bin"])
        self.assertEqual(sorted(grouped["v8-runner"]["binaries"]), ["darwin-arm64"])

    def test_load_tool_bundles_can_filter_one_release_target(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            darwin_bundle = self.write_bundle(root, "darwin-arm64", module)
            self.write_bundle(root, "linux-x64", module)

            grouped, bin_roots = module.load_tool_bundles(
                root,
                self.make_lock(),
                allow_partial_targets=True,
                target="darwin-arm64",
            )

        self.assertEqual(bin_roots, [darwin_bundle / "bin"])
        self.assertEqual(sorted(grouped["v8-runner"]["binaries"]), ["darwin-arm64"])

    def test_archive_base_name_is_platform_specific_for_release_packages(self) -> None:
        module = load_package_module()

        self.assertEqual(
            module.archive_base_name("0.3.3", target="darwin-arm64"),
            "unica-codex-marketplace-darwin-arm64",
        )
        self.assertEqual(module.archive_base_name("0.3.3", target=None), "unica-codex-marketplace-0.3.3")

    def test_write_marketplace_can_use_local_debug_name(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "marketplace.json"
            dest = root / "out.json"
            source.write_text(
                json.dumps(
                    {
                        "name": "unica",
                        "interface": {"displayName": "Unica"},
                        "plugins": [
                            {
                                "name": "unica",
                                "source": {"source": "local", "path": "./plugins/unica"},
                                "category": "Coding",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            module.write_official_marketplace(source, dest, marketplace_name="unica-local")

            data = json.loads(dest.read_text(encoding="utf-8"))
            self.assertEqual(data["name"], "unica-local")
            self.assertEqual(data["plugins"][0]["name"], "unica")

    def test_installer_prompt_verification_uses_current_skill_markers(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        installer = (repo_root / "scripts" / "install-unica.sh").read_text(encoding="utf-8")

        self.assertIn('"unica:meta-compile"', installer)
        self.assertIn('"unica:v8-runner"', installer)
        self.assertIn('"unica:db-auth-check"', installer)
        self.assertIn("grep -Fq", installer)
        self.assertNotIn('"workspace-init"', installer)
        self.assertNotIn('for needle in "Unica"', installer)

    @unittest.skipIf(os.name == "nt", "POSIX executable bits are validated on POSIX CI")
    def test_copy_binary_tree_marks_files_executable(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            dest = root / "dest"
            source.mkdir()
            binary = source / "v8-runner"
            binary.write_text("binary", encoding="utf-8")
            binary.chmod(0o644)

            module.copy_binary_tree(source, dest)

            copied_mode = (dest / "v8-runner").stat().st_mode
            self.assertTrue(copied_mode & stat.S_IXUSR)

    @unittest.skipIf(os.name == "nt", "generated shell launcher smoke is POSIX-only")
    def test_generated_marketplace_runs_packaged_unica_help(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]
        target = "darwin-arm64" if os.uname().sysname == "Darwin" else "linux-x64"
        target_triple = {
            "darwin-arm64": "aarch64-apple-darwin",
            "linux-x64": "x86_64-unknown-linux-gnu",
        }[target]

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            tools_root = root / "tools"
            bundle = tools_root / f"unica-tools-{target}"
            bin_dir = bundle / "bin" / target
            bin_dir.mkdir(parents=True)
            binary = bin_dir / "unica"
            binary.write_text(
                "#!/usr/bin/env sh\n"
                "if [ \"$1\" = \"--help\" ]; then\n"
                "  echo 'unica 0.4.3'\n"
                "  echo 'stdio MCP orchestrator for Unica workflows'\n"
                "  exit 0\n"
                "fi\n"
                "exit 64\n",
                encoding="utf-8",
            )
            binary.chmod(0o755)
            (bundle / "tools.json").write_text(
                json.dumps(
                    {
                        "target": target,
                        "targetTriple": target_triple,
                        "tools": [
                            {
                                "name": "unica",
                                "version": "0.4.3",
                                "repository": "https://github.com/IngvarConsulting/unica",
                                "upstreamUrl": "https://github.com/IngvarConsulting/unica/releases/tag/workspace",
                                "sourceTag": "workspace",
                                "sourceCommit": "workspace",
                                "license": "LGPL-3.0-or-later",
                                "targetTriple": target_triple,
                                "binaryPath": f"bin/{target}/unica",
                                "sha256": module.sha256(binary),
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            lock_file = root / "tools.lock.json"
            lock_file.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "targets": {target: {"targetTriple": target_triple}},
                        "tools": [
                            {
                                "name": "unica",
                                "version": "0.4.3",
                                "repository": "https://github.com/IngvarConsulting/unica",
                                "sourceTag": "workspace",
                                "sourceCommit": "workspace",
                                "license": "LGPL-3.0-or-later",
                                "assets": {target: {"assetName": "unica"}},
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            out_dir = root / "out"

            argv = [
                "package-unica-plugin.py",
                "--repo-root",
                str(repo_root),
                "--tools-root",
                str(tools_root),
                "--lock-file",
                str(lock_file),
                "--out-dir",
                str(out_dir),
                "--target",
                target,
                "--allow-partial-targets",
                "--no-archives",
            ]
            with patch("sys.argv", argv):
                module.main()

            packaged_mcp = json.loads(
                (out_dir / "marketplace" / "plugins" / "unica" / ".mcp.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(sorted(packaged_mcp["mcpServers"]), ["unica"])
            provenance = out_dir / "marketplace" / "plugins" / "unica" / "provenance" / "skill-upstreams.json"
            self.assertTrue(provenance.is_file())
            self.assertIn("v8-runner-rust", provenance.read_text(encoding="utf-8"))
            upstream_review = (
                out_dir
                / "marketplace"
                / "plugins"
                / "unica"
                / "provenance"
                / "reviews"
                / "2026-06-15-upstream-review.json"
            )
            self.assertTrue(upstream_review.is_file())
            self.assertIn("needs-adaptation-decision", upstream_review.read_text(encoding="utf-8"))
            product_backlog = (
                out_dir
                / "marketplace"
                / "plugins"
                / "unica"
                / "provenance"
                / "reviews"
                / "2026-06-18-product-update-backlog.json"
            )
            self.assertTrue(product_backlog.is_file())
            self.assertIn("bsl-analyzer", product_backlog.read_text(encoding="utf-8"))

            result = subprocess.run(
                [
                    str(
                        out_dir
                        / "marketplace"
                        / "plugins"
                        / "unica"
                        / "scripts"
                        / "run-unica.sh"
                    ),
                    "--help",
                ],
                cwd=out_dir / "marketplace",
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=True,
            )
            self.assertIn("unica 0.4.3", result.stdout)


if __name__ == "__main__":
    unittest.main()
