#!/usr/bin/env python3
"""Build one target bundle of pinned Unica tool binaries for release packaging."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path


BSL_VERSION = "0.1.144"
BSL_TAG = "v0.1.144"
BSL_COMMIT = "aff7de0f7b6e2db73bba5c16ca95b84971bc1002"
BSL_REPO = "https://github.com/itrous/bsl-analyzer"

V8_RUNNER_VERSION = "0.3.0"
V8_RUNNER_TAG = "v0.3.0"
V8_RUNNER_COMMIT = "e41c9dba5552851036fd6087f0b4a72ede01cb81"
V8_RUNNER_REPO = "https://github.com/alkoleft/v8-runner-rust"

RLM_VERSION = "1.9.4"
RLM_TAG = "v1.9.4"
RLM_COMMIT = "0af7461caf9547bed14bf742b66d71deb57b86e8"
RLM_REPO = "https://github.com/Dach-Coin/rlm-tools-bsl"


TARGETS = {
    "darwin-arm64": {
        "targetTriple": "aarch64-apple-darwin",
        "hostSystem": "Darwin",
        "hostMachine": {"arm64", "aarch64"},
        "exe": "",
        "bslAsset": "bsl-analyzer-app-darwin-arm64",
        "v8Asset": "v8-runner-macos-aarch64.tar.gz",
        "v8Binary": "v8-runner",
    },
    "linux-x64": {
        "targetTriple": "x86_64-unknown-linux-gnu",
        "hostSystem": "Linux",
        "hostMachine": {"x86_64", "amd64"},
        "exe": "",
        "bslAsset": "bsl-analyzer-app-linux-amd64",
        "v8Asset": "v8-runner-linux-x86_64-musl.tar.gz",
        "v8Binary": "v8-runner",
    },
    "win-x64": {
        "targetTriple": "x86_64-pc-windows-msvc",
        "hostSystem": "Windows",
        "hostMachine": {"amd64", "x86_64"},
        "exe": ".exe",
        "bslAsset": "bsl-analyzer-app-windows-amd64.exe",
        "v8Asset": "v8-runner-windows-x86_64.zip",
        "v8Binary": "v8-runner.exe",
    },
}


def run(args: list[str], *, cwd: Path | None = None) -> None:
    print("+", " ".join(args), flush=True)
    subprocess.run(args, cwd=cwd, check=True)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"download {url}", flush=True)
    with urllib.request.urlopen(url) as response, dest.open("wb") as out:
        shutil.copyfileobj(response, out)


def assert_host(target: str) -> None:
    cfg = TARGETS[target]
    system = platform.system()
    machine = platform.machine().lower()
    if system != cfg["hostSystem"] or machine not in cfg["hostMachine"]:
        expected = f"{cfg['hostSystem']} {sorted(cfg['hostMachine'])}"
        actual = f"{system} {machine}"
        raise SystemExit(f"target {target} must be built on {expected}; current runner is {actual}")


def extract_v8_runner(archive: Path, binary_name: str, dest: Path) -> None:
    extract_dir = archive.parent / f"{archive.name}.extract"
    shutil.rmtree(extract_dir, ignore_errors=True)
    extract_dir.mkdir(parents=True)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as zf:
            zf.extractall(extract_dir)
    else:
        with tarfile.open(archive) as tf:
            tf.extractall(extract_dir)

    matches = [p for p in extract_dir.rglob(binary_name) if p.is_file()]
    if not matches:
        raise SystemExit(f"{binary_name} not found in {archive}")
    shutil.copy2(matches[0], dest)


def build_rlm_tools(source_dir: Path, work_dir: Path, out_dir: Path, exe: str) -> None:
    if not source_dir.exists():
        raise SystemExit(f"rlm-tools-bsl source directory not found: {source_dir}")

    run([sys.executable, "-m", "pip", "install", "--upgrade", "pip", "pyinstaller"])
    run([sys.executable, "-m", "pip", "install", str(source_dir)])

    for command_name in ("rlm-tools-bsl", "rlm-bsl-index"):
        script = shutil.which(command_name)
        if not script:
            raise SystemExit(f"installed entrypoint not found on PATH: {command_name}")

        build_root = work_dir / command_name
        shutil.rmtree(build_root, ignore_errors=True)
        build_root.mkdir(parents=True)
        run(
            [
                sys.executable,
                "-m",
                "PyInstaller",
                "--onefile",
                "--clean",
                "--noconfirm",
                "--name",
                command_name,
                "--collect-all",
                "rlm_tools_bsl",
                script,
            ],
            cwd=build_root,
        )
        produced = build_root / "dist" / f"{command_name}{exe}"
        if not produced.exists():
            raise SystemExit(f"PyInstaller output not found: {produced}")
        shutil.copy2(produced, out_dir / f"{command_name}{exe}")


def tool_entry(
    *,
    target: str,
    name: str,
    version: str,
    repository: str,
    tag: str,
    commit: str,
    license_id: str,
    binary: Path,
    relative_binary: str,
) -> dict:
    return {
        "name": name,
        "version": version,
        "repository": repository,
        "upstreamUrl": f"{repository}/releases/tag/{tag}",
        "sourceTag": tag,
        "sourceCommit": commit,
        "license": license_id,
        "target": target,
        "targetTriple": TARGETS[target]["targetTriple"],
        "binaryPath": relative_binary,
        "sha256": sha256(binary),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=sorted(TARGETS), required=True)
    parser.add_argument("--rlm-source", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, default=Path(".build/unica-tools"))
    args = parser.parse_args()

    assert_host(args.target)
    cfg = TARGETS[args.target]
    exe = cfg["exe"]

    target_bin_dir = args.out_dir / "bin" / args.target
    downloads_dir = args.work_dir / args.target / "downloads"
    target_bin_dir.mkdir(parents=True, exist_ok=True)
    downloads_dir.mkdir(parents=True, exist_ok=True)

    bsl_url = f"{BSL_REPO}/releases/download/{BSL_TAG}/{cfg['bslAsset']}"
    bsl_dest = target_bin_dir / f"bsl-analyzer{exe}"
    download(bsl_url, downloads_dir / cfg["bslAsset"])
    shutil.copy2(downloads_dir / cfg["bslAsset"], bsl_dest)

    v8_url = f"{V8_RUNNER_REPO}/releases/download/{V8_RUNNER_TAG}/{cfg['v8Asset']}"
    v8_dest = target_bin_dir / f"v8-runner{exe}"
    download(v8_url, downloads_dir / cfg["v8Asset"])
    extract_v8_runner(downloads_dir / cfg["v8Asset"], cfg["v8Binary"], v8_dest)

    build_rlm_tools(args.rlm_source, args.work_dir / args.target / "rlm-build", target_bin_dir, exe)

    for path in target_bin_dir.iterdir():
        if path.is_file() and os.name != "nt":
            path.chmod(path.stat().st_mode | 0o755)

    tools = [
        tool_entry(
            target=args.target,
            name="bsl-analyzer",
            version=BSL_VERSION,
            repository=BSL_REPO,
            tag=BSL_TAG,
            commit=BSL_COMMIT,
            license_id="LGPL-3.0-or-later",
            binary=bsl_dest,
            relative_binary=f"bin/{args.target}/bsl-analyzer{exe}",
        ),
        tool_entry(
            target=args.target,
            name="v8-runner",
            version=V8_RUNNER_VERSION,
            repository=V8_RUNNER_REPO,
            tag=V8_RUNNER_TAG,
            commit=V8_RUNNER_COMMIT,
            license_id="NOASSERTION",
            binary=v8_dest,
            relative_binary=f"bin/{args.target}/v8-runner{exe}",
        ),
        tool_entry(
            target=args.target,
            name="rlm-tools-bsl",
            version=RLM_VERSION,
            repository=RLM_REPO,
            tag=RLM_TAG,
            commit=RLM_COMMIT,
            license_id="MIT",
            binary=target_bin_dir / f"rlm-tools-bsl{exe}",
            relative_binary=f"bin/{args.target}/rlm-tools-bsl{exe}",
        ),
        tool_entry(
            target=args.target,
            name="rlm-bsl-index",
            version=RLM_VERSION,
            repository=RLM_REPO,
            tag=RLM_TAG,
            commit=RLM_COMMIT,
            license_id="MIT",
            binary=target_bin_dir / f"rlm-bsl-index{exe}",
            relative_binary=f"bin/{args.target}/rlm-bsl-index{exe}",
        ),
    ]

    (args.out_dir / "tools.json").write_text(
        json.dumps(
            {
                "target": args.target,
                "targetTriple": cfg["targetTriple"],
                "tools": tools,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
