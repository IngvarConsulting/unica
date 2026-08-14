from __future__ import annotations

import gzip
import hashlib
import importlib.util
import io
import json
import re
import shutil
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch


def load_build_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "build-unica-tools.py"
    spec = importlib.util.spec_from_file_location("build_unica_tools", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_runtime_archive_module():
    module_path = (
        Path(__file__).resolve().parents[2]
        / "scripts"
        / "ci"
        / "unica_runtime_archive.py"
    )
    spec = importlib.util.spec_from_file_location("unica_runtime_archive", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def add_archive_member(
    archive: tarfile.TarFile,
    name: str,
    payload: bytes,
    *,
    mode: int = 0o644,
    type_: bytes = tarfile.REGTYPE,
    linkname: str = "",
) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(payload) if type_ == tarfile.REGTYPE else 0
    info.mode = mode
    info.type = type_
    info.linkname = linkname
    archive.addfile(info, io.BytesIO(payload) if type_ == tarfile.REGTYPE else None)


def write_raw_archive(
    path: Path,
    members: list[tuple[str, bytes, int, bytes, str]],
) -> None:
    with path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for name, payload, mode, type_, linkname in members:
                    add_archive_member(
                        archive,
                        name,
                        payload,
                        mode=mode,
                        type_=type_,
                        linkname=linkname,
                    )


RUNTIME_RELEASE = "rlm-tools-bsl-v1.33.0-build.3"
RUNTIME_SOURCE_COMMIT = "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549"
RUNTIME_TARGET = {
    "key": "linux-x64",
    "triple": "x86_64-unknown-linux-gnu",
}
RUNTIME_ENTRYPOINTS = {
    "rlm-bsl-index": "rlm-bsl-index",
    "rlm-bsl-mcp": "rlm-bsl-mcp",
}
RUNTIME_TARGETS = ("darwin-arm64", "linux-x64", "win-x64")
RUNTIME_ARCHIVES = {
    "darwin-arm64": {
        "assetName": "rlm-tools-bsl-darwin-arm64.tar.gz",
        "sha256": "55caf6a245b3bb47344e2191408841f45aefb614b23480d9941f2cb3e2d8af2c",
        "size": 72_708_783,
    },
    "linux-x64": {
        "assetName": "rlm-tools-bsl-linux-x64.tar.gz",
        "sha256": "1a27e1305c159c01f4b928fa63358567236197844af4663188bd5b30aa780f40",
        "size": 106_083_876,
    },
    "win-x64": {
        "assetName": "rlm-tools-bsl-win-x64.tar.gz",
        "sha256": "9655a8d052ae3d033ea8761e7a503ffd1d9a7e4f303b17ed6c8bc9fd86e5abb2",
        "size": 75_235_914,
    },
}


def runtime_manifest(binary: bytes = b"multidist") -> dict:
    files = [
        {
            "path": "libpython3.12.so.1.0",
            "sha256": hashlib.sha256(b"shared").hexdigest(),
            "size": len(b"shared"),
            "executable": False,
        },
        {
            "path": "rlm-bsl-index",
            "sha256": hashlib.sha256(binary).hexdigest(),
            "size": len(binary),
            "executable": True,
        },
        {
            "path": "rlm-bsl-mcp",
            "sha256": hashlib.sha256(binary).hexdigest(),
            "size": len(binary),
            "executable": True,
        },
    ]
    return {
        "schemaVersion": 1,
        "releaseTag": RUNTIME_RELEASE,
        "source": {
            "ref": "v1.33.0",
            "commit": RUNTIME_SOURCE_COMMIT,
            "tree": "4b321de0454d4d0998762659891374a3a1326cd0",
            "patches": [],
        },
        "target": RUNTIME_TARGET,
        "entrypoints": RUNTIME_ENTRYPOINTS,
        "builder": {
            "kind": "python-nuitka-standalone",
            "python": "3.12.10",
            "uv": "0.11.29",
            "nuitka": "4.1.3",
            "compiler": {
                "cCompiler": "gcc",
                "ccName": "gcc",
                "compiler": "gcc",
            },
        },
        "files": files,
    }


class BuildUnicaToolsTests(unittest.TestCase):
    def assert_external_toolchain_contract(self, external_tools: list[dict]) -> None:
        expected_names = {
            "bsl-analyzer",
            "v8-runner",
            "rlm-bsl-mcp",
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

            source_tag = tool["sourceTag"]
            source_label = (
                source_tag
                if source_tag.startswith("v")
                else f"nightly-{re.sub(r'[^a-z0-9]+', '-', source_tag.lower()).strip('-')}"
            )
            source_suffix = f"-{source_label}"
            self.assertTrue(release_tag.endswith(source_suffix))
            release_name = release_tag[: -len(source_suffix)]
            declared_release_name = tool.get("releaseName", tool["name"])
            self.assertEqual(release_name, declared_release_name)
            self.assertTrue(
                any(
                    candidate.get("releaseName", candidate["name"])
                    == declared_release_name
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
            self.assertEqual(
                tool["assetRepository"], "https://github.com/IngvarConsulting/unica-toolchain"
            )
            for target, asset in tool["assets"].items():
                exe = ".exe" if target == "win-x64" else ""
                self.assertRegex(asset["sha256"], r"^[0-9a-f]{64}$")
                if tool["name"] in RUNTIME_ENTRYPOINTS:
                    self.assertEqual(tool["assetStrategy"], "archive-release-asset")
                    self.assertEqual(asset["assetName"], f"rlm-tools-bsl-{target}.tar.gz")
                    self.assertEqual(asset["archiveBinary"], tool["binaryName"] + exe)
                else:
                    self.assertEqual(tool["assetStrategy"], "direct-release-asset")
                    self.assertEqual(asset["assetName"], f"{tool['binaryName']}-{target}{exe}")
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
            source_tag = tool["sourceTag"]
            current_source_label = (
                source_tag
                if source_tag.startswith("v")
                else f"nightly-{re.sub(r'[^a-z0-9]+', '-', source_tag.lower()).strip('-')}"
            )
            current_revision = tool["assetTag"].rsplit("-build.", 1)[1]
            release_name = tool["assetTag"].removesuffix(
                f"-{current_source_label}-build.{current_revision}"
            )
            if source_tag.startswith("v"):
                version = source_tag.removeprefix("v").split(".")
                version[-1] = str(int(version[-1]) + 1)
                tool["sourceTag"] = f"v{'.'.join(version)}"
                source_label = tool["sourceTag"]
            else:
                source_label = (
                    f"nightly-{re.sub(r'[^a-z0-9]+', '-', source_tag.lower()).strip('-')}"
                )
            build_revision = int(current_revision) + 1
            tool["assetTag"] = f"{release_name}-{source_label}-build.{build_revision}"
        self.assert_external_toolchain_contract(updated_tools)

    def test_historical_build_2_release_provenance_is_immutable(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        review = json.loads(
            (
                repo_root
                / "docs"
                / "provenance"
                / "reviews"
                / "2026-08-13-rlm-v1-33-product-update.json"
            ).read_text(encoding="utf-8")
        )

        expected_assets = {
            "rlm-bsl-index": {
                "darwin-arm64": {
                    "assetName": "rlm-bsl-index-darwin-arm64",
                    "sha256": "d48bd7a0186e46b6d2a48476bc9926fb638882544e7be201293f89db9e654a63",
                    "size": 22_384_192,
                },
                "linux-x64": {
                    "assetName": "rlm-bsl-index-linux-x64",
                    "sha256": "94ffdcf44330ed5ad6121682fcefd7560adfdc9ebe09ee2a1476666e21c33996",
                    "size": 36_846_384,
                },
                "win-x64": {
                    "assetName": "rlm-bsl-index-win-x64.exe",
                    "sha256": "1e64e9436ea2fa69212b27ebe3f6d349fc53acc56250a79d1d2cf67c4570d69b",
                    "size": 23_834_846,
                },
            },
            "rlm-bsl-mcp": {
                "darwin-arm64": {
                    "assetName": "rlm-bsl-mcp-darwin-arm64",
                    "sha256": "312fe35fa211dee1137cf4aef7e52a2bb1eb161ad903a87257342257985efe00",
                    "size": 22_384_192,
                },
                "linux-x64": {
                    "assetName": "rlm-bsl-mcp-linux-x64",
                    "sha256": "f81cf7776fc6bf0bda6290f86a665593ce4daa6b04ec5d02d00f158345bfd277",
                    "size": 36_846_384,
                },
                "win-x64": {
                    "assetName": "rlm-bsl-mcp-win-x64.exe",
                    "sha256": "c198b3539769c207f4d0d0f95848ff47d8f892f499ec04f300f2d7538c658c11",
                    "size": 23_834_848,
                },
            },
        }

        for name, assets in expected_assets.items():
            with self.subTest(tool=name):
                self.assertEqual(review["toolchain"]["releaseTag"], "rlm-tools-bsl-v1.33.0-build.2")
                self.assertEqual(review["tools"][name]["assets"], assets)

    def test_checked_in_rlm_tools_select_one_build_3_archive_per_target(self) -> None:
        repo_root = Path(__file__).resolve().parents[2]
        lock = json.loads(
            (repo_root / "plugins" / "unica" / "third-party" / "tools.lock.json").read_text(
                encoding="utf-8"
            )
        )
        tools = {
            tool["name"]: tool
            for tool in lock["tools"]
            if tool["name"] in RUNTIME_ENTRYPOINTS
        }

        self.assertEqual(set(tools), set(RUNTIME_ENTRYPOINTS))
        for name, tool in tools.items():
            self.assertEqual(tool["assetStrategy"], "archive-release-asset")
            self.assertEqual(tool["assetTag"], RUNTIME_RELEASE)
            for target, asset in tool["assets"].items():
                expected = RUNTIME_ARCHIVES[target]
                self.assertEqual(asset["assetName"], expected["assetName"])
                self.assertEqual(asset["archiveBinary"], name + (".exe" if target == "win-x64" else ""))
                self.assertEqual(asset["sha256"], expected["sha256"])
                self.assertEqual(asset["size"], expected["size"])
        for target in RUNTIME_TARGETS:
            archive_identities = {
                (
                    tool["assets"][target]["assetName"],
                    tool["assets"][target]["sha256"],
                    tool["assets"][target]["size"],
                )
                for tool in tools.values()
            }
            self.assertEqual(len(archive_identities), 1)

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

    def test_bundle_builder_downloads_shared_archive_once_and_declares_runtime_closure(
        self,
    ) -> None:
        module = load_build_module()
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        archive_path = root / "source.tar.gz"
        manifest = runtime_manifest()
        members = [
            (
                "manifest.json",
                json.dumps(manifest, separators=(",", ":")).encode(),
                0o644,
                tarfile.REGTYPE,
                "",
            ),
            ("payload/libpython3.12.so.1.0", b"shared", 0o644, tarfile.REGTYPE, ""),
            ("payload/rlm-bsl-index", b"multidist", 0o755, tarfile.REGTYPE, ""),
            ("payload/rlm-bsl-mcp", b"multidist", 0o755, tarfile.REGTYPE, ""),
        ]
        write_raw_archive(archive_path, members)
        asset = {
            "assetName": "rlm-tools-bsl-linux-x64.tar.gz",
            "sha256": hashlib.sha256(archive_path.read_bytes()).hexdigest(),
            "size": archive_path.stat().st_size,
        }

        def tool(name: str) -> dict:
            return {
                "name": name,
                "version": "1.33.0",
                "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
                "sourceTag": "v1.33.0",
                "sourceCommit": RUNTIME_SOURCE_COMMIT,
                "license": "MIT",
                "binaryName": name,
                "assetStrategy": "archive-release-asset",
                "assetRepository": "https://github.com/IngvarConsulting/unica-toolchain",
                "assetTag": RUNTIME_RELEASE,
                "assets": {
                    "linux-x64": {
                        **asset,
                        "archiveBinary": name,
                    }
                },
            }

        lock = {
            "schemaVersion": 1,
            "targets": {
                "linux-x64": {
                    "hostSystem": "Linux",
                    "hostMachines": ["x86_64"],
                    "targetTriple": RUNTIME_TARGET["triple"],
                    "exe": "",
                }
            },
            "tools": [tool("rlm-bsl-index"), tool("rlm-bsl-mcp")],
        }
        lock_path = root / "tools.lock.json"
        lock_path.write_text(json.dumps(lock), encoding="utf-8")
        out_dir = root / "bundle"
        work_dir = root / "work"
        downloads: list[tuple[str, Path]] = []

        def fake_download(url: str, destination: Path) -> None:
            downloads.append((url, destination))
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(archive_path, destination)

        argv = [
            "build-unica-tools.py",
            "--target",
            "linux-x64",
            "--lock-file",
            str(lock_path),
            "--repo-root",
            str(root),
            "--out-dir",
            str(out_dir),
            "--work-dir",
            str(work_dir),
        ]
        with (
            patch.object(module, "assert_host"),
            patch.object(module, "download", side_effect=fake_download),
            patch.object(module, "load_cargo_workspace_binary_owners", return_value={}),
            patch.object(
                module,
                "build_cargo_workspace_binaries",
                return_value=({}, root / "bootstrap", 0.0),
            ),
            patch.object(sys, "argv", argv),
        ):
            module.main()

        self.assertEqual(len(downloads), 1)
        tools = json.loads((out_dir / "tools.json").read_text(encoding="utf-8"))
        self.assertEqual(tools["schemaVersion"], 2)
        self.assertEqual(
            [item["path"] for item in tools["runtimeFiles"]],
            [
                "bin/linux-x64/libpython3.12.so.1.0",
                "bin/linux-x64/rlm-bsl-index",
                "bin/linux-x64/rlm-bsl-mcp",
            ],
        )
        self.assertEqual(
            {item["name"]: item["binaryPath"] for item in tools["tools"]},
            {
                "rlm-bsl-index": "bin/linux-x64/rlm-bsl-index",
                "rlm-bsl-mcp": "bin/linux-x64/rlm-bsl-mcp",
            },
        )
        self.assertEqual(
            (out_dir / "bin" / "linux-x64" / "rlm-bsl-index").read_bytes(),
            b"multidist",
        )
        self.assertEqual(
            (out_dir / "bin" / "linux-x64" / "rlm-bsl-mcp").read_bytes(),
            b"multidist",
        )

    def test_verified_archive_rejects_unsafe_and_drifted_members(self) -> None:
        module = load_runtime_archive_module()
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        base_payload = [
            ("payload/libpython3.12.so.1.0", b"shared", 0o644, tarfile.REGTYPE, ""),
            ("payload/rlm-bsl-index", b"multidist", 0o755, tarfile.REGTYPE, ""),
            ("payload/rlm-bsl-mcp", b"multidist", 0o755, tarfile.REGTYPE, ""),
        ]

        def archive(
            name: str,
            *,
            manifest: dict | None = None,
            payload: list[tuple[str, bytes, int, bytes, str]] | None = None,
        ) -> Path:
            path = root / f"{name}.tar.gz"
            document = runtime_manifest() if manifest is None else manifest
            members = [
                (
                    "manifest.json",
                    json.dumps(document, separators=(",", ":")).encode(),
                    0o644,
                    tarfile.REGTYPE,
                    "",
                ),
                *(base_payload if payload is None else payload),
            ]
            write_raw_archive(path, members)
            return path

        def load(path: Path):
            return module.load_verified_archive(
                path,
                release_tag=RUNTIME_RELEASE,
                source_commit=RUNTIME_SOURCE_COMMIT,
                target=RUNTIME_TARGET,
                entrypoints=RUNTIME_ENTRYPOINTS,
            )

        files = load(archive("valid"))
        by_path = {item.path.as_posix(): item for item in files}
        self.assertEqual(set(by_path), {item[0].removeprefix("payload/") for item in base_payload})
        self.assertEqual(by_path["rlm-bsl-index"].payload, b"multidist")
        self.assertTrue(by_path["rlm-bsl-mcp"].executable)
        self.assertFalse(by_path["libpython3.12.so.1.0"].executable)

        unsafe_members = {
            "absolute": base_payload + [("/absolute", b"x", 0o644, tarfile.REGTYPE, "")],
            "parent": base_payload + [("payload/../escape", b"x", 0o644, tarfile.REGTYPE, "")],
            "backslash": base_payload + [("payload\\escape", b"x", 0o644, tarfile.REGTYPE, "")],
            "duplicate": base_payload + [base_payload[1]],
            "symlink": base_payload + [("payload/link", b"", 0o777, tarfile.SYMTYPE, "rlm-bsl-index")],
            "hardlink": base_payload + [("payload/link", b"", 0o777, tarfile.LNKTYPE, "rlm-bsl-index")],
            "fifo": base_payload + [("payload/fifo", b"", 0o644, tarfile.FIFOTYPE, "")],
        }
        for label, members in unsafe_members.items():
            with self.subTest(unsafe=label):
                with self.assertRaisesRegex(SystemExit, "unsafe|duplicate|ordinary"):
                    load(archive(f"unsafe-{label}", payload=members))

        mutations: list[
            tuple[
                str,
                dict,
                list[tuple[str, bytes, int, bytes, str]],
                str,
            ]
        ] = []
        wrong_release = runtime_manifest()
        wrong_release["releaseTag"] = "other"
        mutations.append(("release", wrong_release, base_payload, "releaseTag"))
        wrong_source = runtime_manifest()
        wrong_source["source"] = dict(wrong_source["source"], commit="a" * 40)
        mutations.append(("source", wrong_source, base_payload, "source.commit"))
        wrong_target = runtime_manifest()
        wrong_target["target"] = dict(RUNTIME_TARGET, key="win-x64")
        mutations.append(("target", wrong_target, base_payload, "target"))
        missing_entrypoint = runtime_manifest()
        missing_entrypoint["entrypoints"] = {"rlm-bsl-index": "rlm-bsl-index"}
        mutations.append(("entrypoint", missing_entrypoint, base_payload, "entrypoints"))
        wrong_digest = runtime_manifest()
        wrong_digest["files"][1]["sha256"] = "0" * 64
        mutations.append(("digest", wrong_digest, base_payload, "sha256"))
        wrong_size = runtime_manifest()
        wrong_size["files"][1]["size"] = 99
        mutations.append(("size", wrong_size, base_payload, "size"))
        wrong_mode = runtime_manifest()
        mutations.append(
            (
                "mode",
                wrong_mode,
                [base_payload[0], (base_payload[1][0], b"multidist", 0o644, tarfile.REGTYPE, ""), base_payload[2]],
                "mode|executable",
            )
        )
        non_executable_entrypoint = runtime_manifest()
        non_executable_entrypoint["files"][1]["executable"] = False
        mutations.append(
            (
                "non-executable-entrypoint",
                non_executable_entrypoint,
                [
                    base_payload[0],
                    (
                        base_payload[1][0],
                        b"multidist",
                        0o644,
                        tarfile.REGTYPE,
                        "",
                    ),
                    base_payload[2],
                ],
                "entrypoints.*not executable",
            )
        )
        mutations.append(("missing", runtime_manifest(), base_payload[:-1], "file set"))
        mutations.append(
            (
                "extra",
                runtime_manifest(),
                base_payload + [("payload/extra", b"x", 0o644, tarfile.REGTYPE, "")],
                "file set",
            )
        )
        unequal = runtime_manifest(binary=b"index")
        unequal["files"][2] = {
            "path": "rlm-bsl-mcp",
            "sha256": hashlib.sha256(b"mcp").hexdigest(),
            "size": 3,
            "executable": True,
        }
        mutations.append(
            (
                "unequal",
                unequal,
                [
                    base_payload[0],
                    ("payload/rlm-bsl-index", b"index", 0o755, tarfile.REGTYPE, ""),
                    ("payload/rlm-bsl-mcp", b"mcp", 0o755, tarfile.REGTYPE, ""),
                ],
                "byte-identical",
            )
        )

        for label, manifest, payload, message in mutations:
            with self.subTest(drift=label):
                with self.assertRaisesRegex(SystemExit, message):
                    load(archive(f"drift-{label}", manifest=manifest, payload=payload))

    def test_archive_group_rejects_conflicts_before_materialization(self) -> None:
        module = load_build_module()
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        archive_path = root / "source.tar.gz"
        write_raw_archive(
            archive_path,
            [
                (
                    "manifest.json",
                    json.dumps(runtime_manifest(), separators=(",", ":")).encode(),
                    0o644,
                    tarfile.REGTYPE,
                    "",
                ),
                (
                    "payload/libpython3.12.so.1.0",
                    b"shared",
                    0o644,
                    tarfile.REGTYPE,
                    "",
                ),
                (
                    "payload/rlm-bsl-index",
                    b"multidist",
                    0o755,
                    tarfile.REGTYPE,
                    "",
                ),
                (
                    "payload/rlm-bsl-mcp",
                    b"multidist",
                    0o755,
                    tarfile.REGTYPE,
                    "",
                ),
            ],
        )
        common_asset = {
            "assetName": "rlm-tools-bsl-linux-x64.tar.gz",
            "sha256": hashlib.sha256(archive_path.read_bytes()).hexdigest(),
            "size": archive_path.stat().st_size,
        }

        def tool(name: str, selector: str | None) -> dict:
            asset = dict(common_asset)
            if selector is not None:
                asset["archiveBinary"] = selector
            return {
                "name": name,
                "repository": "https://github.com/Dach-Coin/rlm-tools-bsl",
                "sourceTag": "v1.33.0",
                "sourceCommit": RUNTIME_SOURCE_COMMIT,
                "assetRepository": "https://github.com/IngvarConsulting/unica-toolchain",
                "assetTag": RUNTIME_RELEASE,
                "assets": {"linux-x64": asset},
            }

        missing = [tool("rlm-bsl-index", None), tool("rlm-bsl-mcp", "rlm-bsl-mcp")]
        with patch.object(module, "download") as download_call:
            with self.assertRaisesRegex(SystemExit, "missing archiveBinary"):
                module.materialize_archive_group(
                    missing,
                    target="linux-x64",
                    target_triple=RUNTIME_TARGET["triple"],
                    downloads_dir=root / "downloads",
                    target_bin_dir=root / "bundle" / "bin" / "linux-x64",
                    reserved_paths={},
                )
        download_call.assert_not_called()

        conflicting = [
            tool("rlm-bsl-index", "rlm-bsl-index"),
            tool("rlm-bsl-mcp", "rlm-bsl-mcp"),
        ]
        conflicting[1]["assets"]["linux-x64"]["sha256"] = "0" * 64
        with patch.object(module, "download") as download_call:
            with self.assertRaisesRegex(SystemExit, "conflicting archive identities"):
                module.materialize_archive_group(
                    conflicting,
                    target="linux-x64",
                    target_triple=RUNTIME_TARGET["triple"],
                    downloads_dir=root / "downloads",
                    target_bin_dir=root / "bundle" / "bin" / "linux-x64",
                    reserved_paths={},
                )
        download_call.assert_not_called()

        def fake_download(_url: str, destination: Path) -> None:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(archive_path, destination)

        valid = [
            tool("rlm-bsl-index", "rlm-bsl-index"),
            tool("rlm-bsl-mcp", "rlm-bsl-mcp"),
        ]
        with patch.object(module, "download", side_effect=fake_download) as download_call:
            with self.assertRaisesRegex(SystemExit, "runtime destination collision"):
                module.materialize_archive_group(
                    valid,
                    target="linux-x64",
                    target_triple=RUNTIME_TARGET["triple"],
                    downloads_dir=root / "downloads",
                    target_bin_dir=root / "bundle" / "bin" / "linux-x64",
                    reserved_paths={
                        "bin/linux-x64/libpython3.12.so.1.0": "another tool"
                    },
                )
        download_call.assert_called_once()

    def test_workspace_binaries_share_one_locked_cargo_build(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo_root = root / "repo"
            repo_root.mkdir()
            bundle_root = root / "bundle"
            target_bin_dir = bundle_root / "bin" / "win-x64"
            target_bin_dir.mkdir(parents=True)
            target_dir = root / "cargo-target"
            runtime_binary = target_dir / "release" / "unica.exe"
            runtime_binary.parent.mkdir(parents=True)
            runtime_binary.write_bytes(b"rust mcp")
            bootstrap_binary = target_dir / "release" / "unica-bootstrap.exe"
            bootstrap_binary.write_bytes(b"native bootstrap")
            calls = []

            def fake_run(args, *, cwd=None):
                calls.append((args, cwd))

            with (
                patch.object(module, "run", side_effect=fake_run),
                patch.object(module.time, "monotonic", side_effect=[10.0, 12.5]),
            ):
                built_paths, bootstrap_path, duration = module.build_cargo_workspace_binaries(
                    [
                        {
                            "name": "unica",
                            "binaryName": "unica",
                            "cargoPackage": "unica-coder",
                            "cargoBin": "unica",
                        }
                    ],
                    repo_root=repo_root,
                    target_dir=target_dir,
                    target_bin_dir=target_bin_dir,
                    bundle_root=bundle_root,
                    target="win-x64",
                    exe=".exe",
                    workspace_binary_owners={
                        "unica": {"unica-coder"},
                        "unica-bootstrap": {"unica-bootstrap"},
                    },
                )

            self.assertEqual(built_paths, {"unica": target_bin_dir / "unica.exe"})
            self.assertEqual(built_paths["unica"].read_bytes(), b"rust mcp")
            self.assertEqual(
                bootstrap_path,
                bundle_root / "bootstrap" / "bin" / "win-x64" / "unica-bootstrap.exe",
            )
            self.assertEqual(bootstrap_path.read_bytes(), b"native bootstrap")
            self.assertEqual(duration, 2.5)
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][1], repo_root)
            self.assertEqual(
                calls[0][0],
                [
                    "cargo",
                    "build",
                    "--release",
                    "--locked",
                    "--package",
                    "unica-coder",
                    "--package",
                    "unica-bootstrap",
                    "--bin",
                    "unica",
                    "--bin",
                    "unica-bootstrap",
                    "--target-dir",
                    str(target_dir),
                ],
            )

    def test_workspace_binary_pairs_are_rejected_before_build_when_lock_is_malformed(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(module, "run") as cargo_run:
                with self.assertRaisesRegex(
                    SystemExit,
                    "unica-coder does not own binary target unica-bootstrap",
                ):
                    module.build_cargo_workspace_binaries(
                        [
                            {
                                "name": "unica",
                                "binaryName": "unica",
                                "cargoPackage": "unica-coder",
                                "cargoBin": "unica-bootstrap",
                            }
                        ],
                        repo_root=root,
                        target_dir=root / "cargo-target",
                        target_bin_dir=root / "bundle" / "bin" / "linux-x64",
                        bundle_root=root / "bundle",
                        target="linux-x64",
                        exe="",
                        workspace_binary_owners={
                            "unica": {"unica-coder"},
                            "unica-bootstrap": {"unica-bootstrap"},
                        },
                    )

            cargo_run.assert_not_called()

    def test_workspace_binary_owners_come_from_locked_cargo_metadata(self) -> None:
        module = load_build_module()
        metadata = {
            "packages": [
                {
                    "name": "unica-coder",
                    "targets": [
                        {"name": "unica", "kind": ["bin"]},
                        {"name": "unica_coder", "kind": ["lib"]},
                    ],
                },
                {
                    "name": "unica-bootstrap",
                    "targets": [{"name": "unica-bootstrap", "kind": ["bin"]}],
                },
            ]
        }
        completed = Mock(stdout=json.dumps(metadata))

        with patch.object(module.subprocess, "run", return_value=completed) as cargo_metadata:
            owners = module.load_cargo_workspace_binary_owners(Path("/repo"))

        self.assertEqual(
            owners,
            {
                "unica": {"unica-coder"},
                "unica-bootstrap": {"unica-bootstrap"},
            },
        )
        cargo_metadata.assert_called_once_with(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            cwd=Path("/repo"),
            check=True,
            text=True,
            stdout=module.subprocess.PIPE,
        )

    def test_workspace_binary_name_collision_is_rejected_before_build(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(module, "run") as cargo_run:
                with self.assertRaisesRegex(
                    SystemExit,
                    "shared-bin is ambiguous across selected packages: package-a, package-b",
                ):
                    module.build_cargo_workspace_binaries(
                        [
                            {
                                "name": "tool-a",
                                "binaryName": "tool-a",
                                "cargoPackage": "package-a",
                                "cargoBin": "shared-bin",
                            },
                            {
                                "name": "tool-b",
                                "binaryName": "tool-b",
                                "cargoPackage": "package-b",
                                "cargoBin": "shared-bin",
                            },
                        ],
                        repo_root=root,
                        target_dir=root / "cargo-target",
                        target_bin_dir=root / "bundle" / "bin" / "linux-x64",
                        bundle_root=root / "bundle",
                        target="linux-x64",
                        exe="",
                        workspace_binary_owners={
                            "shared-bin": {"package-a", "package-b"},
                            "unica-bootstrap": {"unica-bootstrap"},
                        },
                    )

            cargo_run.assert_not_called()

    def test_build_metrics_have_stable_schema_and_trailing_newline(self) -> None:
        module = load_build_module()

        with tempfile.TemporaryDirectory() as tmp:
            metrics_file = Path(tmp) / "nested" / "cargo.json"

            module.write_build_metrics(
                metrics_file,
                target="linux-x64",
                cargo_build_seconds=12.34567,
            )

            self.assertEqual(
                json.loads(metrics_file.read_text(encoding="utf-8")),
                {
                    "schemaVersion": 1,
                    "target": "linux-x64",
                    "cargoBuildSeconds": 12.346,
                },
            )
            self.assertTrue(metrics_file.read_bytes().endswith(b"\n"))


if __name__ == "__main__":
    unittest.main()
