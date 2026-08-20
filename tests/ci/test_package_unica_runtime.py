from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import stat
import tarfile
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "package-unica-runtime.py"


def load_module():
    spec = importlib.util.spec_from_file_location("package_unica_runtime", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def make_bundle(root: Path) -> Path:
    bundle = root / "bundle"
    bin_dir = bundle / "bin" / "linux-x64"
    bin_dir.mkdir(parents=True)
    unica = bin_dir / "unica"
    analyzer = bin_dir / "bsl-analyzer"
    rlm_mcp = bin_dir / "rlm-bsl-mcp"
    rlm_index = bin_dir / "rlm-bsl-index"
    shared_library = bin_dir / "libpython3.12.so.1.0"
    unica.write_bytes(b"unica")
    analyzer.write_bytes(b"analyzer")
    rlm_mcp.write_bytes(b"rlm-mcp")
    rlm_index.write_bytes(b"rlm-index")
    shared_library.write_bytes(b"shared-library")
    for executable in (unica, analyzer, rlm_mcp, rlm_index):
        executable.chmod(0o755)
    shared_library.chmod(0o644)
    runtime_paths = [unica, analyzer, rlm_mcp, rlm_index, shared_library]
    # rlm-bsl-mcp и rlm-bsl-index приезжают одним архивом на 69 МБ, и общая
    # библиотека принадлежит ему же: артефакт у всех трёх один.
    artifact_of = {
        unica: "unica",
        analyzer: "bsl-analyzer",
        rlm_mcp: "rlm-tools-bsl",
        rlm_index: "rlm-tools-bsl",
        shared_library: "rlm-tools-bsl",
    }
    (bundle / "tools.json").write_text(
        json.dumps(
            {
                "schemaVersion": 2,
                "target": "linux-x64",
                "targetTriple": "x86_64-unknown-linux-gnu",
                "runtimeFiles": [
                    {
                        "path": path.relative_to(bundle).as_posix(),
                        "sha256": sha256(path),
                        "size": path.stat().st_size,
                        "executable": path != shared_library,
                        "artifact": artifact_of[path],
                    }
                    for path in sorted(runtime_paths)
                ],
                "tools": [
                    {
                        "name": "unica",
                        "version": "0.7.0",
                        "binaryPath": "bin/linux-x64/unica",
                        "sha256": sha256(unica),
                    },
                    {
                        "name": "bsl-analyzer",
                        "version": "0.2.55",
                        "binaryPath": "bin/linux-x64/bsl-analyzer",
                        "sha256": sha256(analyzer),
                    },
                    {
                        "name": "rlm-bsl-mcp",
                        "version": "1.33.0",
                        "artifact": "rlm-tools-bsl",
                        "binaryPath": "bin/linux-x64/rlm-bsl-mcp",
                        "sha256": sha256(rlm_mcp),
                    },
                    {
                        "name": "rlm-bsl-index",
                        "version": "1.33.0",
                        "artifact": "rlm-tools-bsl",
                        "binaryPath": "bin/linux-x64/rlm-bsl-index",
                        "sha256": sha256(rlm_index),
                    },
                ],
            }
        ),
        encoding="utf-8",
    )
    return bundle


class PackageUnicaRuntimeTests(unittest.TestCase):
    def test_runtime_archive_is_deterministic_and_target_only(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            first = root / "first"
            second = root / "second"
            first = {a.name: (a, m) for a, m in module.package_runtime(bundle, first)}
            second = {a.name: (a, m) for a, m in module.package_runtime(bundle, second)}

            # Разрез по артефактам: ядро отдельно, движки отдельно, RLM одним
            # архивом на два инструмента.
            self.assertEqual(
                sorted(first),
                [
                    "bsl-analyzer-runtime-linux-x64.tar.gz",
                    "rlm-tools-bsl-runtime-linux-x64.tar.gz",
                    "unica-runtime-linux-x64.tar.gz",
                ],
            )
            self.assertEqual(sorted(first), sorted(second))
            for name, (archive, metadata) in first.items():
                with self.subTest(artifact=name):
                    other_archive, other_metadata = second[name]
                    self.assertEqual(archive.read_bytes(), other_archive.read_bytes())
                    self.assertEqual(metadata.read_bytes(), other_metadata.read_bytes())

            with tarfile.open(first["unica-runtime-linux-x64.tar.gz"][0], "r:gz") as archive:
                core_members = [member.name for member in archive.getmembers()]
            # Ядро несёт себя и манифест инструментов — и ничего сверх.
            self.assertEqual(
                core_members,
                ["bin/linux-x64/unica", "third-party/manifest.json"],
            )

            with tarfile.open(first["rlm-tools-bsl-runtime-linux-x64.tar.gz"][0], "r:gz") as archive:
                rlm_members = [member.name for member in archive.getmembers()]
            self.assertEqual(
                rlm_members,
                [
                    "bin/linux-x64/libpython3.12.so.1.0",
                    "bin/linux-x64/rlm-bsl-index",
                    "bin/linux-x64/rlm-bsl-mcp",
                ],
                "два инструмента и их общая библиотека едут одним архивом",
            )
            with tarfile.open(first["unica-runtime-linux-x64.tar.gz"][0], "r:gz") as archive:
                self.assertTrue(all(member.mtime == 0 for member in archive.getmembers()))

    def test_metadata_hashes_archive_and_each_runtime_file(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            produced = {a.name: (a, m) for a, m in module.package_runtime(bundle, root / "out")}
            archive, metadata_path = produced["unica-runtime-linux-x64.tar.gz"]
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))

            self.assertEqual(metadata["schemaVersion"], 2)
            self.assertEqual(metadata["artifact"], "unica")
            self.assertEqual(metadata["role"], "core")
            self.assertEqual(metadata["version"], "0.7.0")
            self.assertEqual(metadata["target"], "linux-x64")
            self.assertEqual(metadata["pluginVersion"], "0.7.0")
            self.assertEqual(metadata["asset"]["name"], "unica-runtime-linux-x64.tar.gz")
            self.assertEqual(metadata["asset"]["sha256"], sha256(archive))
            self.assertEqual(metadata["entrypoint"], "bin/linux-x64/unica")
            actual = {item["path"]: item["sha256"] for item in metadata["files"]}
            self.assertEqual(
                actual["bin/linux-x64/unica"], hashlib.sha256(b"unica").hexdigest()
            )
            self.assertIn("third-party/manifest.json", actual)

            _, rlm_metadata_path = produced["rlm-tools-bsl-runtime-linux-x64.tar.gz"]
            rlm = json.loads(rlm_metadata_path.read_text(encoding="utf-8"))
            self.assertEqual(rlm["role"], "engine")
            self.assertEqual(rlm["version"], "1.33.0")
            # Точка входа есть только у ядра: движок запускает рантайм, а не
            # bootstrap.
            self.assertNotIn("entrypoint", rlm)

    def test_runtime_packager_rejects_symlinked_binary(self) -> None:
        module = load_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = make_bundle(root)
            binary = bundle / "bin" / "linux-x64" / "unica"
            binary.unlink()
            binary.symlink_to(bundle / "bin" / "linux-x64" / "bsl-analyzer")

            with self.assertRaisesRegex(SystemExit, "symlink"):
                module.package_runtime(bundle, root / "out")

    def test_runtime_packager_rejects_hard_linked_payload(self) -> None:
        module = load_module()
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        bundle = make_bundle(root)
        library = bundle / "bin" / "linux-x64" / "libpython3.12.so.1.0"
        os.link(library, root / "second-link")

        with self.assertRaisesRegex(SystemExit, "hard-linked"):
            module.package_runtime(bundle, root / "out")

    def test_runtime_packager_rejects_missing_extra_and_metadata_drift(self) -> None:
        module = load_module()

        mutations = {
            "missing": lambda bundle: (
                bundle / "bin" / "linux-x64" / "libpython3.12.so.1.0"
            ).unlink(),
            "extra": lambda bundle: (
                bundle / "bin" / "linux-x64" / "rlm-tools-bsl"
            ).write_bytes(b"legacy"),
            "digest": lambda bundle: (
                bundle / "bin" / "linux-x64" / "libpython3.12.so.1.0"
            ).write_bytes(b"drifted-payloa"),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                bundle = make_bundle(root)
                mutate(bundle)
                with self.assertRaisesRegex(
                    SystemExit,
                    "missing|unexpected|checksum|size|file set",
                ):
                    module.package_runtime(bundle, root / "out")

    @unittest.skipIf(os.name == "nt", "Windows does not preserve POSIX execute bits")
    def test_runtime_packager_rejects_mode_drift(self) -> None:
        module = load_module()
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        bundle = make_bundle(root)
        library = bundle / "bin" / "linux-x64" / "libpython3.12.so.1.0"
        library.chmod(library.stat().st_mode | stat.S_IXUSR)

        with self.assertRaisesRegex(SystemExit, "mode|executable"):
            module.package_runtime(bundle, root / "out")

    def test_runtime_packager_rejects_duplicate_and_out_of_closure_tool_paths(self) -> None:
        module = load_module()
        for label in ("duplicate", "outside", "wrong-target"):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                bundle = make_bundle(root)
                manifest_path = bundle / "tools.json"
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                if label == "duplicate":
                    manifest["runtimeFiles"].append(dict(manifest["runtimeFiles"][0]))
                elif label == "outside":
                    manifest["tools"][0]["binaryPath"] = "bin/linux-x64/not-declared"
                else:
                    manifest["runtimeFiles"][0]["path"] = "bin/win-x64/unica.exe"
                manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

                with self.assertRaisesRegex(
                    SystemExit,
                    "duplicate|closure|declared|outside",
                ):
                    module.package_runtime(bundle, root / "out")


if __name__ == "__main__":
    unittest.main()
