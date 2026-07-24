from __future__ import annotations

import importlib.util
import hashlib
import json
import os
import re
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
                "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu"},
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

        self.assertEqual(server["command"], "cargo")
        self.assertEqual(
            server["args"],
            ["run", "--quiet", "--manifest-path", "../../Cargo.toml", "--bin", "unica", "--"],
        )
        manifest_index = server["args"].index("--manifest-path") + 1
        source_manifest = (repo_root / "plugins" / "unica" / server["args"][manifest_index]).resolve()
        self.assertEqual(source_manifest, repo_root / "Cargo.toml")
        self.assertIn("orchestrator", server["note"])
        self.assertNotIn("bash", json.dumps(server))
        self.assertNotIn("run-unica.sh", json.dumps(server))

    def test_source_tree_does_not_ship_runtime_shell_wrappers(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        scripts_dir = repo_root / "plugins" / "unica" / "scripts"

        wrappers = sorted(
            {
                path.relative_to(repo_root).as_posix()
                for pattern in ("run-*.sh", "run-*.cmd", "run-*.ps1", "run-tool.*")
                for path in scripts_dir.glob(pattern)
            }
        )

        self.assertEqual(wrappers, [])

    def test_parity_fixtures_do_not_contain_runtime_cache_artifacts(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        fixture_root = repo_root / "tests" / "fixtures" / "unica_mcp_script_parity"
        forbidden = []
        for path in fixture_root.rglob("*"):
            rel = path.relative_to(fixture_root).as_posix()
            if "/.build/" in f"/{rel}/" or rel.endswith((".db", ".db-wal", ".db-shm")):
                forbidden.append(rel)
        self.assertEqual(forbidden, [])

    def test_bsp_parity_manifest_matches_committed_fixture_files(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        bsp_root = repo_root / "tests" / "fixtures" / "unica_mcp_script_parity" / "bsp"
        manifest = json.loads((bsp_root / "manifest.json").read_text(encoding="utf-8"))
        manifest_targets = {entry["target"] for entry in manifest["files"]}
        fixture_files = {
            path.relative_to(bsp_root).as_posix()
            for path in bsp_root.rglob("*")
            if path.is_file() and path.name != "manifest.json"
        }
        self.assertEqual(fixture_files, manifest_targets)

    def test_bsp_parity_manifest_hashes_match_committed_fixture_files(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        bsp_root = repo_root / "tests" / "fixtures" / "unica_mcp_script_parity" / "bsp"
        manifest = json.loads((bsp_root / "manifest.json").read_text(encoding="utf-8"))

        mismatches = []
        for entry in manifest["files"]:
            path = bsp_root / entry["target"]
            payload = path.read_bytes()
            actual = {
                "size": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
            expected = {"size": entry["size"], "sha256": entry["sha256"]}
            if actual != expected:
                mismatches.append({"target": entry["target"], "expected": expected, "actual": actual})

        self.assertEqual(mismatches, [])

    def test_bsp_parity_profile_projection_preserves_both_source_and_target_identity(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        manifest_path = (
            repo_root
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "bsp"
            / "manifest.json"
        )
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

        self.assertEqual(manifest["schemaVersion"], 2)
        self.assertEqual(
            manifest["derivation"],
            {
                "exportFormat": "2.20",
                "kind": "profile-projection",
                "platformLine": "8.3.27",
                "recipe": "bsp-2.21-to-2.20-v1",
            },
        )
        projected = []
        for entry in manifest["files"]:
            self.assertIn("harvestedSha256", entry)
            self.assertIn("harvestedSize", entry)
            if (
                entry["harvestedSha256"] != entry["sha256"]
                or entry["harvestedSize"] != entry["size"]
            ):
                projected.append(entry["target"])

        self.assertEqual(len(projected), 17)
        self.assertIn("cf/Configuration.xml", projected)
        self.assertIn("meta/Languages/Русский.xml", projected)
        self.assertTrue(all(target.endswith(".xml") for target in projected))

    def test_bsp_parity_fixture_bytes_are_not_transformed_by_git(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        result = subprocess.run(
            [
                "git",
                "check-attr",
                "text",
                "whitespace",
                "--",
                "tests/fixtures/unica_mcp_script_parity/bsp/cf/Configuration.xml",
            ],
            cwd=repo_root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(": text: unset", result.stdout)
        self.assertIn(": whitespace: unset", result.stdout)

    def test_unica_coder_has_no_runtime_operation_script_fallback(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]

        self.assertFalse((repo_root / "plugins" / "unica" / "scripts" / "legacy").exists())

        rust_sources = sorted((repo_root / "crates" / "unica-coder" / "src").rglob("*.rs"))
        forbidden = (
            "ToolHandler::LegacyScript",
            "LegacyScriptAdapter",
            "legacy_scripts",
            'Command::new("python3")',
            'Command::new("python")',
            'Command::new("bash")',
            'Command::new("powershell")',
            'Command::new("pwsh")',
        )
        matches = [
            f"{path.relative_to(repo_root)}:{needle}"
            for path in rust_sources
            for needle in forbidden
            if needle in path.read_text(encoding="utf-8")
        ]

        self.assertEqual(matches, [])

    def test_source_mcp_does_not_use_runtime_shell_wrappers(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        mcp = json.loads((repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"))
        serialized = json.dumps(mcp)

        forbidden = sorted(
            {
                pattern
                for pattern in ("bash", "cmd.exe", "powershell", ".sh", ".cmd", ".ps1", "run-tool", "run-unica")
                if pattern in serialized
            }
        )

        self.assertEqual(forbidden, [])

    def test_packaged_mcp_uses_only_the_approved_git_shell_wrapper(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]

        with tempfile.TemporaryDirectory() as tmp:
            plugin_dir = Path(tmp) / "plugins" / "unica"
            plugin_dir.mkdir(parents=True)
            (plugin_dir / ".mcp.json").write_text(
                (repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            module.write_packaged_mcp_launcher(
                plugin_dir,
                {
                    "unica": {
                        "binaries": {
                            "linux-x64": {
                                "binaryPath": "bin/linux-x64/unica",
                            }
                        }
                    }
                },
            )
            mcp = json.loads((plugin_dir / ".mcp.json").read_text(encoding="utf-8"))

        serialized = json.dumps(mcp)
        forbidden = sorted(
            {
                pattern
                for pattern in ("bash", "cmd.exe", "powershell", ".cmd", ".ps1", "run-tool", "run-unica")
                if pattern in serialized
            }
        )

        self.assertEqual(forbidden, [])
        self.assertEqual(mcp["mcpServers"]["unica"]["command"], "git")
        self.assertIn("bootstrap/launch.sh", serialized)
        self.assertEqual(mcp["mcpServers"]["unica"]["cwd"], ".")

    def test_packaged_mcp_uses_command_scoped_git_shell_alias(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]

        with tempfile.TemporaryDirectory() as tmp:
            plugin_dir = Path(tmp) / "plugins" / "unica"
            plugin_dir.mkdir(parents=True)
            (plugin_dir / ".mcp.json").write_text(
                (repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            module.write_packaged_mcp_launcher(plugin_dir, {})
            server = json.loads((plugin_dir / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"][
                "unica"
            ]

        self.assertEqual(server["command"], "git")
        self.assertEqual(server["cwd"], ".")
        self.assertEqual(server["args"][0], "-c")
        self.assertTrue(server["args"][1].startswith("alias.unica-bootstrap=!"))
        self.assertIn("bootstrap/launch.sh", server["args"][1])
        self.assertEqual(server["args"][2], "unica-bootstrap")

    @unittest.skipIf(os.name == "nt", "selector fixture uses POSIX test scripts")
    def test_launch_script_selects_git_for_windows_native_bootstrap(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        source_launcher = repo_root / "plugins" / "unica" / "bootstrap" / "launch.sh"

        with tempfile.TemporaryDirectory() as tmp:
            plugin_root = Path(tmp) / "plugin"
            launcher = plugin_root / "bootstrap" / "launch.sh"
            launcher.parent.mkdir(parents=True)
            launcher.write_bytes(source_launcher.read_bytes())
            bootstrap = plugin_root / "bootstrap" / "bin" / "win-x64" / "unica-bootstrap.exe"
            bootstrap.parent.mkdir(parents=True)
            bootstrap.write_text("#!/bin/sh\nprintf 'native=%s\\n' \"$1\"\n", encoding="utf-8")
            bootstrap.chmod(0o755)
            env = os.environ.copy()
            env["UNICA_BOOTSTRAP_UNAME_S"] = "MINGW64_NT-10.0"
            env["UNICA_BOOTSTRAP_UNAME_M"] = "x86_64"

            result = subprocess.run(
                ["sh", str(launcher), str(plugin_root)],
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), f"native=run")

    def test_source_tree_does_not_reference_deleted_runtime_shell_wrappers_in_active_docs(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        active_paths = [
            repo_root / "README.md",
            repo_root / "plugins" / "unica" / "README.md",
            repo_root / "plugins" / "unica" / "references" / "tooling" / "internal-package.md",
            repo_root / "spec" / "acceptance" / "unica-mcp-validation.md",
            repo_root / "spec" / "architecture" / "arc42" / "06-runtime-view.md",
            repo_root / "spec" / "architecture" / "arc42" / "07-deployment-view.md",
            repo_root / "spec" / "architecture" / "change-checklist.md",
            repo_root / "spec" / "decisions" / "0001-edinyy-publichnyy-mcp-unica.md",
            repo_root / "spec" / "decisions" / "0004-legacy-skill-scripts-are-migration-debt.md",
        ]
        forbidden = ("run-unica.sh", "run-tool.sh", "run-tool.ps1", "run-bsl-analyzer.sh", "run-v8-runner.sh")

        matches = [
            f"{path.relative_to(repo_root)}:{needle}"
            for path in active_paths
            for needle in forbidden
            if needle in path.read_text(encoding="utf-8")
        ]

        self.assertEqual(matches, [])

    def test_packaged_mcp_entrypoint_is_target_neutral(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]

        with tempfile.TemporaryDirectory() as tmp:
            plugin_dir = Path(tmp) / "plugins" / "unica"
            plugin_dir.mkdir(parents=True)
            (plugin_dir / ".mcp.json").write_text(
                (repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            module.write_packaged_mcp_launcher(
                plugin_dir,
                {
                    "unica": {
                        "binaries": {
                            "win-x64": {
                                "binaryPath": "bin/win-x64/unica.exe",
                            }
                        }
                    }
                },
            )

            mcp = json.loads((plugin_dir / ".mcp.json").read_text(encoding="utf-8"))

        server = mcp["mcpServers"]["unica"]
        self.assertEqual(server["command"], "git")
        self.assertEqual(server["args"][2], "unica-bootstrap")
        self.assertEqual(server["cwd"], ".")
        self.assertNotIn("win-x64", json.dumps(server))
        self.assertNotIn("run-unica.sh", json.dumps(server))

    def test_packaged_mcp_does_not_require_a_full_runtime_binary(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]

        with tempfile.TemporaryDirectory() as tmp:
            plugin_dir = Path(tmp) / "plugins" / "unica"
            plugin_dir.mkdir(parents=True)
            (plugin_dir / ".mcp.json").write_text(
                (repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"),
                encoding="utf-8",
            )

            module.write_packaged_mcp_launcher(plugin_dir, {})
            server = json.loads((plugin_dir / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"][
                "unica"
            ]

        self.assertEqual(server["command"], "git")

    def test_packaged_mcp_ignores_full_runtime_target_matrix(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]

        with tempfile.TemporaryDirectory() as tmp:
            plugin_dir = Path(tmp) / "plugins" / "unica"
            plugin_dir.mkdir(parents=True)
            (plugin_dir / ".mcp.json").write_text(
                (repo_root / "plugins" / "unica" / ".mcp.json").read_text(encoding="utf-8"),
                encoding="utf-8",
            )

            module.write_packaged_mcp_launcher(
                plugin_dir,
                {
                    "unica": {
                        "binaries": {
                            "darwin-arm64": {"binaryPath": "bin/darwin-arm64/unica"},
                            "linux-x64": {"binaryPath": "bin/linux-x64/unica"},
                        }
                    }
                },
            )
            server = json.loads((plugin_dir / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"][
                "unica"
            ]

        self.assertNotIn("darwin-arm64", json.dumps(server))
        self.assertNotIn("linux-x64", json.dumps(server))

    def test_plugin_source_copy_uses_git_tracked_files_only(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            plugin_src = repo_root / "plugins" / "unica"
            plugin_src.mkdir(parents=True)
            (plugin_src / ".mcp.json").write_text("{}", encoding="utf-8")
            (plugin_src / "skills" / "web-test").mkdir(parents=True)
            (plugin_src / "skills" / "web-test" / "SKILL.md").write_text("tracked", encoding="utf-8")
            (plugin_src / "skills" / "web-test" / "scripts" / "node_modules" / "pkg").mkdir(parents=True)
            (plugin_src / "skills" / "web-test" / "scripts" / "node_modules" / "pkg" / "index.js").write_text(
                "untracked dependency",
                encoding="utf-8",
            )
            (plugin_src / "skills" / "web-test" / ".browser-session.json").write_text("{}", encoding="utf-8")
            (plugin_src / "skills" / "web-test" / "screenshot.png").write_bytes(b"png")
            (plugin_src / "skills" / "web-test" / "trace.mp4").write_bytes(b"mp4")

            dest = root / "dest"
            with patch.object(
                module,
                "git_tracked_plugin_files",
                return_value=[".mcp.json", "skills/web-test/SKILL.md"],
            ):
                module.copy_tracked_plugin_source(repo_root, plugin_src, dest)

            self.assertTrue((dest / ".mcp.json").is_file())
            self.assertTrue((dest / "skills" / "web-test" / "SKILL.md").is_file())
            self.assertFalse((dest / "skills" / "web-test" / "scripts" / "node_modules").exists())
            self.assertFalse((dest / "skills" / "web-test" / ".browser-session.json").exists())
            self.assertFalse((dest / "skills" / "web-test" / "screenshot.png").exists())
            self.assertFalse((dest / "skills" / "web-test" / "trace.mp4").exists())

    def test_attribution_page_and_referenced_local_licenses_are_packaged(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]
        plugin_src = repo_root / "plugins" / "unica"
        attribution = plugin_src / "ATTRIBUTIONS.md"

        self.assertTrue(attribution.is_file())
        local_license_links = {
            link
            for link in re.findall(r"\[[^]]+\]\(([^)]+)\)", attribution.read_text(encoding="utf-8"))
            if not link.startswith("https://") and "LICENSE" in link
        }
        self.assertTrue(local_license_links)

        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "unica"
            module.copy_tracked_plugin_source(repo_root, plugin_src, destination)

            self.assertTrue((destination / "ATTRIBUTIONS.md").is_file())
            for link in local_license_links:
                self.assertTrue((destination / link).is_file(), link)

    def test_plugin_source_copy_rejects_tracked_source_bin(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            plugin_src = repo_root / "plugins" / "unica"
            source_bin = plugin_src / "bin" / "win-x64" / "run-tool.ps1"
            source_bin.parent.mkdir(parents=True)
            source_bin.write_text("stale wrapper", encoding="utf-8")

            with patch.object(
                module,
                "git_tracked_plugin_files",
                return_value=["bin/win-x64/run-tool.ps1"],
            ):
                with self.assertRaisesRegex(SystemExit, "source package path is generated"):
                    module.copy_tracked_plugin_source(repo_root, plugin_src, root / "dest")

    def test_plugin_source_copy_rejects_tracked_nested_ignored_dir(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            plugin_src = repo_root / "plugins" / "unica"
            generated = plugin_src / "skills" / "web-test" / "__pycache__" / "script.pyc"
            generated.parent.mkdir(parents=True)
            generated.write_bytes(b"pyc")

            with patch.object(
                module,
                "git_tracked_plugin_files",
                return_value=["skills/web-test/__pycache__/script.pyc"],
            ):
                with self.assertRaisesRegex(SystemExit, "source package path is generated"):
                    module.copy_tracked_plugin_source(repo_root, plugin_src, root / "dest")

    @unittest.skipIf(os.name == "nt" or not hasattr(os, "symlink"), "symlink validation is POSIX-only")
    def test_plugin_source_copy_rejects_tracked_symlink(self) -> None:
        module = load_package_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            plugin_src = repo_root / "plugins" / "unica"
            outside = root / "outside-secret.txt"
            link = plugin_src / "skills" / "web-test" / "leak.txt"
            link.parent.mkdir(parents=True)
            outside.write_text("secret", encoding="utf-8")
            os.symlink(outside, link)

            with patch.object(
                module,
                "git_tracked_plugin_files",
                return_value=["skills/web-test/leak.txt"],
            ):
                with self.assertRaisesRegex(SystemExit, "symlink"):
                    module.copy_tracked_plugin_source(repo_root, plugin_src, root / "dest")

    def write_bundle(self, root: Path, target: str, module) -> Path:
        bundle = root / f"unica-tools-{target}"
        bin_dir = bundle / "bin" / target
        bin_dir.mkdir(parents=True)
        binary = bin_dir / "v8-runner"
        binary.write_text(f"binary for {target}", encoding="utf-8")
        target_triples = {
            "darwin-arm64": "aarch64-apple-darwin",
            "linux-x64": "x86_64-unknown-linux-gnu",
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

    def test_archive_base_name_is_not_used_for_thin_marketplace(self) -> None:
        module = load_package_module()

        self.assertFalse(hasattr(module, "archive_base_name"))

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

    def test_generated_marketplace_is_thin_pinned_and_target_neutral(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]
        target_triples = {
            "darwin-arm64": "aarch64-apple-darwin",
            "linux-x64": "x86_64-unknown-linux-gnu",
            "win-x64": "x86_64-pc-windows-msvc",
        }

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            metadata_root = root / "metadata"
            bootstrap_root = root / "bootstraps"
            metadata_root.mkdir()
            for target, target_triple in target_triples.items():
                exe = ".exe" if target == "win-x64" else ""
                bootstrap = (
                    bootstrap_root
                    / "bootstrap"
                    / "bin"
                    / target
                    / f"unica-bootstrap{exe}"
                )
                bootstrap.parent.mkdir(parents=True)
                bootstrap.write_bytes(f"bootstrap {target}".encode())
                (metadata_root / f"unica-runtime-{target}.json").write_text(
                    json.dumps(
                        {
                            "schemaVersion": 1,
                            "target": target,
                            "targetTriple": target_triple,
                            "pluginVersion": "0.8.1",
                            "asset": {
                                "name": f"unica-runtime-{target}.tar.gz",
                                "mediaType": "application/gzip",
                                "sha256": "1" * 64,
                            },
                            "files": [
                                {
                                    "path": f"bin/{target}/unica{exe}",
                                    "sha256": "2" * 64,
                                    "executable": True,
                                }
                            ],
                            "entrypoint": f"bin/{target}/unica{exe}",
                        }
                    ),
                    encoding="utf-8",
                )
            out_dir = root / "out"

            argv = [
                "package-unica-plugin.py",
                "--repo-root",
                str(repo_root),
                "--runtime-metadata-root",
                str(metadata_root),
                "--bootstrap-root",
                str(bootstrap_root),
                "--release-tag",
                "v0.8.1",
                "--source-commit",
                "a" * 40,
                "--out-dir",
                str(out_dir),
            ]
            with patch("sys.argv", argv):
                module.main()

            plugin = out_dir / "marketplace" / "plugins" / "unica"
            packaged_paths = {
                path.relative_to(plugin).as_posix()
                for path in plugin.rglob("*")
            }
            self.assertFalse(
                any(
                    path == "cc-1c-skills"
                    or path.startswith("cc-1c-skills/")
                    or "/cc-1c-skills/" in path
                    for path in packaged_paths
                ),
                "pristine donor scripts and cases must remain test-only",
            )
            forbidden_script_skills = {
                path
                for path in packaged_paths
                if path == "skills/img-grid"
                or path.startswith("skills/img-grid/")
                or path == "skills/web-test"
                or path.startswith("skills/web-test/")
            }
            self.assertEqual(forbidden_script_skills, set())
            self.assertFalse(
                any(
                    Path(path).name in {"package.json", "package-lock.json"}
                    for path in packaged_paths
                )
            )
            packaged_mcp = json.loads(
                (plugin / ".mcp.json").read_text(encoding="utf-8")
            )
            self.assertEqual(sorted(packaged_mcp["mcpServers"]), ["unica"])
            self.assertEqual(packaged_mcp["mcpServers"]["unica"]["command"], "git")
            self.assertEqual(packaged_mcp["mcpServers"]["unica"]["args"][2], "unica-bootstrap")
            self.assertFalse((plugin / "bin").exists())
            for target in target_triples:
                exe = ".exe" if target == "win-x64" else ""
                self.assertTrue(
                    (plugin / "bootstrap" / "bin" / target / f"unica-bootstrap{exe}").is_file()
                )
            runtime_manifest = json.loads(
                (plugin / "runtime-manifest.json").read_text(encoding="utf-8")
            )
            self.assertFalse(runtime_manifest["development"])
            self.assertEqual(runtime_manifest["source"]["commit"], "a" * 40)
            self.assertEqual(runtime_manifest["release"]["tag"], "v0.8.1")
            self.assertEqual(sorted(runtime_manifest["targets"]), sorted(target_triples))
            for target, target_data in runtime_manifest["targets"].items():
                self.assertEqual(
                    target_data["asset"]["url"],
                    "https://github.com/IngvarConsulting/unica/releases/download/"
                    f"v0.8.1/unica-runtime-{target}.tar.gz",
                )

            catalog = json.loads(
                (out_dir / "marketplace" / ".agents" / "plugins" / "marketplace.json").read_text(
                    encoding="utf-8"
                )
            )
            source = catalog["plugins"][0]["source"]
            self.assertEqual(source["source"], "git-subdir")
            self.assertEqual(source["ref"], "v0.8.1")
            self.assertEqual(source["path"], "./plugins/unica")
            self.assertNotIn("source\": \"local", json.dumps(catalog))
            self.assertEqual(list(out_dir.glob("*.tar.gz")), [])
            self.assertEqual(list(out_dir.glob("*.zip")), [])

            provenance = plugin / "provenance" / "skill-upstreams.json"
            self.assertTrue(provenance.is_file())
            self.assertIn("v8-runner-rust", provenance.read_text(encoding="utf-8"))
            upstream_review = (
                plugin
                / "provenance"
                / "reviews"
                / "2026-06-15-upstream-review.json"
            )
            self.assertTrue(upstream_review.is_file())
            upstream_review_data = json.loads(upstream_review.read_text(encoding="utf-8"))
            upstreams = {item["id"]: item for item in upstream_review_data["upstreams"]}
            ai_rules = upstreams["ai-rules-1c"]
            self.assertEqual(ai_rules["reviewStatus"], "reviewed")
            self.assertEqual(ai_rules["affectedEntries"], [])
            decisions = {item["skill"]: item for item in ai_rules["entryDecisions"]}
            self.assertEqual(decisions["api-design"]["primarySource"], "unica")
            self.assertEqual(decisions["api-design"]["decision"], "ignored-with-reason")
            product_backlog = (
                plugin
                / "provenance"
                / "reviews"
                / "2026-06-18-product-update-backlog.json"
            )
            self.assertTrue(product_backlog.is_file())
            self.assertIn("bsl-analyzer", product_backlog.read_text(encoding="utf-8"))

    def test_local_debug_mode_remains_current_host_only_and_uses_unica_dev(self) -> None:
        module = load_package_module()
        repo_root = Path(__file__).resolve().parents[2]
        lock = json.loads(
            (repo_root / "plugins/unica/third-party/tools.lock.json").read_text(encoding="utf-8")
        )
        target = "linux-x64"
        triple = lock["targets"][target]["targetTriple"]

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / "tools" / "unica-tools-linux-x64"
            bin_dir = bundle / "bin" / target
            bin_dir.mkdir(parents=True)
            tools = []
            for locked in lock["tools"]:
                binary = bin_dir / locked["binaryName"]
                binary.write_bytes(locked["name"].encode())
                tools.append(
                    {
                        "name": locked["name"],
                        "version": locked["version"],
                        "repository": locked["repository"],
                        "upstreamUrl": f"{locked['repository']}/releases/tag/{locked['sourceTag']}",
                        "sourceTag": locked["sourceTag"],
                        "sourceCommit": locked["sourceCommit"],
                        "license": locked["license"],
                        "targetTriple": triple,
                        "binaryPath": f"bin/{target}/{locked['binaryName']}",
                        "sha256": module.sha256(binary),
                    }
                )
            (bundle / "tools.json").write_text(
                json.dumps({"target": target, "targetTriple": triple, "tools": tools}),
                encoding="utf-8",
            )
            out = root / "out"
            argv = [
                "package-unica-plugin.py",
                "--repo-root",
                str(repo_root),
                "--tools-root",
                str(root / "tools"),
                "--out-dir",
                str(out),
                "--local-debug-target",
                target,
            ]
            with patch("sys.argv", argv):
                module.main()

            marketplace = json.loads(
                (out / "marketplace/.agents/plugins/marketplace.json").read_text(encoding="utf-8")
            )
            mcp = json.loads(
                (out / "marketplace/plugins/unica/.mcp.json").read_text(encoding="utf-8")
            )
            self.assertEqual(marketplace["name"], "unica-dev")
            self.assertEqual(mcp["mcpServers"]["unica"]["command"], "./bin/linux-x64/unica")
            self.assertFalse((out / "marketplace/plugins/unica/bootstrap/bin").exists())


if __name__ == "__main__":
    unittest.main()
