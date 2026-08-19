#!/usr/bin/env python3
"""Build one target bundle of Unica tool binaries from third-party/tools.lock.json."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from collections import defaultdict
from collections.abc import Callable
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from unica_runtime_archive import RuntimeArchiveFile, load_verified_archive


ArchiveIdentity = tuple[str, str, str, str, str]


def load_lock(path: Path) -> dict:
    lock = json.loads(path.read_text(encoding="utf-8"))
    if lock.get("schemaVersion") != 1:
        raise SystemExit(f"unsupported tools lock schemaVersion in {path}: {lock.get('schemaVersion')}")
    if not lock.get("targets") or not lock.get("tools"):
        raise SystemExit(f"invalid tools lock: {path}")
    return lock


def run(args: list[str], *, cwd: Path | None = None) -> None:
    print("+", " ".join(args), flush=True)
    subprocess.run(args, cwd=cwd, check=True)


def load_cargo_workspace_binary_owners(repo_root: Path) -> dict[str, set[str]]:
    command = ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(
        command,
        cwd=repo_root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    metadata = json.loads(completed.stdout)
    owners: dict[str, set[str]] = {}
    for package in metadata.get("packages", []):
        package_name = package["name"]
        for target in package.get("targets", []):
            if "bin" in target.get("kind", []):
                owners.setdefault(target["name"], set()).add(package_name)
    return owners


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def verify_asset_checksum(path: Path, asset: dict, *, tool_name: str, target: str) -> None:
    expected = asset.get("sha256")
    if not expected:
        raise SystemExit(f"{tool_name} {target} asset {asset.get('assetName')} is missing sha256 in tools lock")
    actual = sha256(path)
    if actual != expected:
        raise SystemExit(
            f"{tool_name} {target} asset checksum mismatch for {asset.get('assetName')}: "
            f"{actual} != {expected}"
        )


def verify_asset_size(path: Path, asset: dict, *, tool_name: str, target: str) -> None:
    expected = asset.get("size")
    if not isinstance(expected, int) or isinstance(expected, bool) or expected <= 0:
        raise SystemExit(
            f"{tool_name} {target} asset {asset.get('assetName')} is missing positive size in tools lock"
        )
    actual = path.stat().st_size
    if actual != expected:
        raise SystemExit(
            f"{tool_name} {target} asset size mismatch for {asset.get('assetName')}: "
            f"{actual} != {expected}"
        )


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"download {url}", flush=True)
    with urllib.request.urlopen(url) as response, dest.open("wb") as out:
        shutil.copyfileobj(response, out)


def release_asset_url(tool: dict, asset: dict) -> str:
    repository = tool.get("assetRepository", tool["repository"])
    tag = tool.get("assetTag", tool["sourceTag"])
    return f"{repository}/releases/download/{tag}/{asset['assetName']}"


def archive_identity(tool: dict, asset: dict, *, target: str) -> ArchiveIdentity:
    repository = tool.get("assetRepository", tool["repository"])
    tag = tool.get("assetTag", tool["sourceTag"])
    name = asset.get("assetName")
    digest = asset.get("sha256")
    if not isinstance(name, str) or not name or "/" in name or "\\" in name:
        raise SystemExit(f"{tool['name']} {target} archive has unsafe assetName: {name}")
    if not isinstance(digest, str) or len(digest) != 64:
        raise SystemExit(f"{tool['name']} {target} archive is missing sha256")
    return repository, tag, target, name, digest


def set_file_mode(path: Path, *, executable: bool) -> None:
    path.chmod(0o755 if executable else 0o644)


def artifact_name(tool: dict) -> str:
    """Имя скачиваемого архива. Несколько инструментов делят один: у RLM это
    `rlm-tools-bsl` на `rlm-bsl-mcp` и `rlm-bsl-index`. Где архив свой,
    артефакт зовётся как инструмент."""
    return tool.get("releaseName", tool["name"])


def runtime_file_entry(
    path: Path,
    *,
    relative_path: str,
    executable: bool,
    artifact: str,
) -> dict:
    return {
        "path": relative_path,
        "sha256": sha256(path),
        "size": path.stat().st_size,
        "executable": executable,
        # Из какого артефакта приехал файл: без этого разрезать поставку
        # нечем — замыкание плоское и принадлежности не помнит.
        "artifact": artifact,
    }


def materialize_archive_group(
    tools: list[dict],
    *,
    target: str,
    target_triple: str,
    downloads_dir: Path,
    target_bin_dir: Path,
    reserved_paths: dict[str, str],
) -> tuple[dict[str, Path], list[dict], float]:
    first = tools[0]
    first_asset = first["assets"][target]
    identity = archive_identity(first, first_asset, target=target)
    for tool in tools[1:]:
        asset = tool["assets"][target]
        if archive_identity(tool, asset, target=target) != identity:
            raise SystemExit(
                f"conflicting archive identities for release {identity[0]} {identity[1]} {target}"
            )
        comparable = {key: value for key, value in asset.items() if key != "archiveBinary"}
        first_comparable = {
            key: value for key, value in first_asset.items() if key != "archiveBinary"
        }
        if comparable != first_comparable:
            raise SystemExit(f"conflicting archive metadata for {identity[3]}")
        if tool["sourceCommit"] != first["sourceCommit"]:
            raise SystemExit(f"conflicting archive source commits for {identity[3]}")

    entrypoints: dict[str, str] = {}
    for tool in tools:
        archive_binary = tool["assets"][target].get("archiveBinary")
        if not isinstance(archive_binary, str) or not archive_binary:
            raise SystemExit(
                f"{tool['name']} {target} archive is missing archiveBinary selector"
            )
        entrypoints[tool["name"]] = archive_binary

    downloaded = downloads_dir / f"{identity[4][:16]}-{identity[3]}"
    download_started_at = time.monotonic()
    download(release_asset_url(first, first_asset), downloaded)
    download_seconds = time.monotonic() - download_started_at
    verify_asset_checksum(downloaded, first_asset, tool_name=first["name"], target=target)
    verify_asset_size(downloaded, first_asset, tool_name=first["name"], target=target)
    files = load_verified_archive(
        downloaded,
        release_tag=identity[1],
        source_commit=first["sourceCommit"],
        target={"key": target, "triple": target_triple},
        entrypoints=entrypoints,
    )

    planned: list[tuple[RuntimeArchiveFile, str]] = []
    for item in files:
        relative = (Path("bin") / target / Path(*item.path.parts)).as_posix()
        owner = reserved_paths.get(relative)
        if owner is not None:
            raise SystemExit(
                f"runtime destination collision at {relative}: {owner} and archive {identity[3]}"
            )
        reserved_paths[relative] = f"archive {identity[3]}"
        planned.append((item, relative))

    runtime_files: list[dict] = []
    by_archive_path: dict[str, Path] = {}
    for item, relative in planned:
        destination = target_bin_dir.joinpath(*item.path.parts)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(item.payload)
        set_file_mode(destination, executable=item.executable)
        runtime_files.append(
            runtime_file_entry(
                destination,
                relative_path=relative,
                executable=item.executable,
                artifact=artifact_name(first),
            )
        )
        by_archive_path[item.path.as_posix()] = destination

    built_paths = {
        tool["name"]: by_archive_path[tool["assets"][target]["archiveBinary"]]
        for tool in tools
    }
    return built_paths, runtime_files, download_seconds


def assert_host(target: str, targets: dict) -> None:
    cfg = targets[target]
    system = platform.system()
    machine = platform.machine().lower()
    supported_machines = {str(item).lower() for item in cfg["hostMachines"]}
    if system != cfg["hostSystem"] or machine not in supported_machines:
        expected = f"{cfg['hostSystem']} {sorted(supported_machines)}"
        actual = f"{system} {machine}"
        raise SystemExit(f"target {target} must be built on {expected}; current runner is {actual}")


def build_cargo_workspace_binaries(
    cargo_tools: list[dict],
    *,
    repo_root: Path,
    target_dir: Path,
    target_bin_dir: Path,
    bundle_root: Path,
    target: str,
    exe: str,
    workspace_binary_owners: dict[str, set[str]],
) -> tuple[dict[str, Path], Path, float]:
    """Build runtime and package infrastructure in one locked Cargo invocation."""
    requested_pairs = [
        (tool["cargoPackage"], tool.get("cargoBin", tool["binaryName"]))
        for tool in cargo_tools
    ]
    requested_pairs.append(("unica-bootstrap", "unica-bootstrap"))
    packages = list(dict.fromkeys(package for package, _ in requested_pairs))
    selected_packages = set(packages)
    for package, binary_name in requested_pairs:
        owners = workspace_binary_owners.get(binary_name, set())
        if package not in owners:
            raise SystemExit(f"cargo package {package} does not own binary target {binary_name}")
        selected_owners = owners & selected_packages
        if selected_owners != {package}:
            raise SystemExit(
                f"cargo binary target {binary_name} is ambiguous across selected packages: "
                f"{', '.join(sorted(selected_owners))}"
            )
    binary_names = list(
        dict.fromkeys(binary_name for _, binary_name in requested_pairs)
    )

    command = ["cargo", "build", "--release", "--locked"]
    for package in packages:
        command.extend(["--package", package])
    for binary_name in binary_names:
        command.extend(["--bin", binary_name])
    command.extend(["--target-dir", str(target_dir)])

    started_at = time.monotonic()
    run(command, cwd=repo_root)
    cargo_build_seconds = time.monotonic() - started_at

    target_bin_dir.mkdir(parents=True, exist_ok=True)
    built_paths: dict[str, Path] = {}
    for tool in cargo_tools:
        binary_name = tool.get("cargoBin", tool["binaryName"])
        produced = target_dir / "release" / f"{binary_name}{exe}"
        if not produced.exists():
            raise SystemExit(f"cargo build output not found: {produced}")
        destination = target_bin_dir / f"{tool['binaryName']}{exe}"
        shutil.copy2(produced, destination)
        built_paths[tool["name"]] = destination

    produced = target_dir / "release" / f"unica-bootstrap{exe}"
    if not produced.exists():
        raise SystemExit(f"cargo build output not found: {produced}")

    destination = bundle_root / "bootstrap" / "bin" / target / f"unica-bootstrap{exe}"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(produced, destination)
    if not destination.name.endswith(".exe"):
        destination.chmod(destination.stat().st_mode | 0o755)
    return built_paths, destination, cargo_build_seconds


def write_build_metrics(
    path: Path,
    *,
    target: str,
    cargo_build_seconds: float,
    archive_download_seconds: float,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(
            {
                "schemaVersion": 1,
                "target": target,
                "cargoBuildSeconds": round(cargo_build_seconds, 3),
                "archiveDownloadSeconds": round(archive_download_seconds, 3),
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def tool_entry(
    *,
    target: str,
    target_triple: str,
    name: str,
    version: str,
    repository: str,
    tag: str,
    commit: str,
    license_id: str,
    binary: Path,
    relative_binary: str,
    artifact: str,
) -> dict:
    return {
        "name": name,
        "version": version,
        "artifact": artifact,
        "repository": repository,
        "upstreamUrl": f"{repository}/releases/tag/{tag}",
        "sourceTag": tag,
        "sourceCommit": commit,
        "license": license_id,
        "target": target,
        "targetTriple": target_triple,
        "binaryPath": relative_binary,
        "sha256": sha256(binary),
    }


def build_bundle_atomically(
    output_dir: Path,
    build: Callable[[Path], object],
) -> object:
    """Build outside the visible output path and publish it with one rename."""
    if output_dir.exists():
        raise SystemExit(f"bundle output already exists: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(
            prefix=f".{output_dir.name}.staging-",
            dir=output_dir.parent,
        )
    )
    try:
        result = build(staging)
        os.replace(staging, output_dir)
        return result
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def build_locked_bundle(
    args: argparse.Namespace,
    lock: dict,
    cfg: dict,
    *,
    output_dir: Path,
) -> tuple[float, float]:
    exe = cfg["exe"]

    target_bin_dir = output_dir / "bin" / args.target
    downloads_dir = args.work_dir / args.target / "downloads"
    target_bin_dir.mkdir(parents=True, exist_ok=True)
    downloads_dir.mkdir(parents=True, exist_ok=True)

    built_paths: dict[str, Path] = {}
    cargo_tools: list[dict] = []
    direct_tools: list[dict] = []
    archive_groups: dict[ArchiveIdentity, list[dict]] = defaultdict(list)
    archive_release_identities: dict[tuple[str, str, str], tuple[str, str, int]] = {}
    reserved_paths: dict[str, str] = {}
    runtime_files: list[dict] = []
    archive_download_seconds = 0.0
    for tool in lock["tools"]:
        strategy = tool["assetStrategy"]
        if strategy in ("direct-release-asset", "cargo-workspace"):
            relative = f"bin/{args.target}/{tool['binaryName']}{exe}"
            owner = reserved_paths.get(relative)
            if owner is not None:
                raise SystemExit(
                    f"runtime destination collision at {relative}: {owner} and {tool['name']}"
                )
            reserved_paths[relative] = tool["name"]

        if strategy == "direct-release-asset":
            direct_tools.append(tool)
        elif strategy == "archive-release-asset":
            asset = tool["assets"].get(args.target)
            if not asset:
                raise SystemExit(f"{tool['name']} has no asset for target {args.target}")
            identity = archive_identity(tool, asset, target=args.target)
            release_key = identity[:3]
            size = asset.get("size")
            if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
                raise SystemExit(
                    f"{tool['name']} {args.target} archive is missing positive size in tools lock"
                )
            asset_identity = (identity[3], identity[4], size)
            existing = archive_release_identities.setdefault(release_key, asset_identity)
            if existing != asset_identity:
                raise SystemExit(
                    f"conflicting archive identities for release {identity[0]} {identity[1]} {args.target}"
                )
            archive_groups[identity].append(tool)
        elif strategy == "cargo-workspace":
            cargo_tools.append(tool)
        else:
            raise SystemExit(f"unsupported assetStrategy for {tool['name']}: {strategy}")

    for tool in direct_tools:
        asset = tool["assets"].get(args.target)
        if not asset:
            raise SystemExit(f"{tool['name']} has no asset for target {args.target}")
        dest = target_bin_dir / f"{tool['binaryName']}{exe}"
        url = release_asset_url(tool, asset)
        downloaded = downloads_dir / asset["assetName"]
        download(url, downloaded)
        verify_asset_checksum(downloaded, asset, tool_name=tool["name"], target=args.target)
        shutil.copy2(downloaded, dest)
        set_file_mode(dest, executable=True)
        built_paths[tool["name"]] = dest
        runtime_files.append(
            runtime_file_entry(
                dest,
                relative_path=f"bin/{args.target}/{dest.name}",
                executable=True,
                artifact=artifact_name(tool),
            )
        )

    for identity in sorted(archive_groups):
        archive_paths, archive_files, group_download_seconds = materialize_archive_group(
            archive_groups[identity],
            target=args.target,
            target_triple=cfg["targetTriple"],
            downloads_dir=downloads_dir,
            target_bin_dir=target_bin_dir,
            reserved_paths=reserved_paths,
        )
        built_paths.update(archive_paths)
        runtime_files.extend(archive_files)
        archive_download_seconds += group_download_seconds

    cargo_paths, _, cargo_build_seconds = build_cargo_workspace_binaries(
        cargo_tools,
        repo_root=args.repo_root.resolve(),
        target_dir=args.work_dir / args.target / "cargo-target",
        target_bin_dir=target_bin_dir,
        bundle_root=output_dir,
        target=args.target,
        exe=exe,
        workspace_binary_owners=load_cargo_workspace_binary_owners(args.repo_root.resolve()),
    )
    built_paths.update(cargo_paths)
    for tool in cargo_tools:
        path = cargo_paths[tool["name"]]
        set_file_mode(path, executable=True)
        runtime_files.append(
            runtime_file_entry(
                path,
                relative_path=f"bin/{args.target}/{path.name}",
                executable=True,
                artifact=artifact_name(tool),
            )
        )
    tools = [
        tool_entry(
            target=args.target,
            target_triple=cfg["targetTriple"],
            name=tool["name"],
            version=tool["version"],
            repository=tool["repository"],
            tag=tool["sourceTag"],
            commit=tool["sourceCommit"],
            license_id=tool["license"],
            binary=built_paths[tool["name"]],
            relative_binary=f"bin/{args.target}/{built_paths[tool['name']].name}",
            artifact=artifact_name(tool),
        )
        for tool in lock["tools"]
    ]

    (output_dir / "tools.json").write_text(
        json.dumps(
            {
                "schemaVersion": 2,
                "target": args.target,
                "targetTriple": cfg["targetTriple"],
                "lockFile": str(args.lock_file),
                "tools": tools,
                "runtimeFiles": sorted(runtime_files, key=lambda item: item["path"]),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return cargo_build_seconds, archive_download_seconds


def main() -> None:
    if sys.version_info < (3, 10):
        raise SystemExit("build-unica-tools.py requires Python >= 3.10 because rlm-tools-bsl requires >= 3.10")

    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--lock-file", type=Path, default=Path("plugins/unica/third-party/tools.lock.json"))
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, default=Path(".build/unica-tools"))
    parser.add_argument("--metrics-file", type=Path)
    args = parser.parse_args()

    lock = load_lock(args.lock_file)
    targets = lock["targets"]
    if args.target not in targets:
        raise SystemExit(f"unknown target {args.target}; expected one of {', '.join(sorted(targets))}")

    assert_host(args.target, targets)
    cfg = targets[args.target]
    cargo_build_seconds, archive_download_seconds = build_bundle_atomically(
        args.out_dir,
        lambda output_dir: build_locked_bundle(
            args,
            lock,
            cfg,
            output_dir=output_dir,
        ),
    )
    if args.metrics_file is not None:
        write_build_metrics(
            args.metrics_file,
            target=args.target,
            cargo_build_seconds=cargo_build_seconds,
            archive_download_seconds=archive_download_seconds,
        )


if __name__ == "__main__":
    main()
