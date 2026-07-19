from __future__ import annotations

import importlib.util
import io
import tarfile
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


class BuildPythonEntrypointTests(unittest.TestCase):
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

    def test_archive_extraction_rejects_path_traversal_members(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "v8-runner.tar.gz"
            outside = root / "escape"
            payload = b"owned"

            with tarfile.open(archive, "w:gz") as tf:
                info = tarfile.TarInfo("../escape")
                info.size = len(payload)
                tf.addfile(info, io.BytesIO(payload))

            with self.assertRaisesRegex(SystemExit, "unsafe archive member"):
                module.extract_v8_runner(archive, "v8-runner", root / "v8-runner")

            self.assertFalse(outside.exists())

    def test_pyinstaller_uses_generated_python_stub_for_entrypoint(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            work_dir = root / "pyinstaller"
            out_dir = root / "out"
            out_dir.mkdir()
            calls = []

            def fake_run(args, *, cwd=None):
                calls.append((args, cwd))
                dist = Path(cwd) / "dist"
                dist.mkdir()
                (dist / "rlm-tools-bsl.exe").write_bytes(b"frozen")

            with (
                patch.object(
                    module,
                    "resolve_console_script_entrypoint",
                    return_value=("rlm_tools_bsl.server", "main"),
                ),
                patch.object(module, "run", side_effect=fake_run),
            ):
                dest = module.build_python_entrypoint(
                    {
                        "entrypoint": "rlm-tools-bsl",
                        "binaryName": "rlm-tools-bsl",
                    },
                    work_dir,
                    out_dir,
                    ".exe",
                    root / "venv" / "Scripts" / "python.exe",
                )

            stub = work_dir / "rlm-tools-bsl" / "rlm-tools-bsl-entrypoint.py"
            self.assertEqual(dest, out_dir / "rlm-tools-bsl.exe")
            self.assertEqual(dest.read_bytes(), b"frozen")
            self.assertTrue(stub.exists())
            self.assertIn("MODULE = 'rlm_tools_bsl.server'", stub.read_text(encoding="utf-8"))

            args, cwd = calls[0]
            self.assertEqual(cwd, (work_dir / "rlm-tools-bsl").resolve())
            self.assertEqual(Path(args[-1]).resolve(), stub.resolve())
            self.assertEqual(args[args.index("--collect-all") + 1], "rlm_tools_bsl")
            self.assertEqual(args[args.index("--hidden-import") + 1], "rlm_tools_bsl.server")
            self.assertFalse(str(args[-1]).endswith(".exe"))

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
