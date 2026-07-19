#!/usr/bin/env python3
"""Smoke a packaged native bootstrap against a locally served runtime archive."""

from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import os
import shutil
import subprocess
import tempfile
import threading
from functools import partial
from pathlib import Path


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, _format: str, *_args: object) -> None:
        return


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def prepare_plugin(
    plugin_root: Path,
    runtime_archive: Path,
    target: str,
    destination: Path,
    asset_url: str,
) -> tuple[Path, Path]:
    prepared = destination / "plugin"
    shutil.copytree(plugin_root, prepared)
    manifest_path = prepared / "runtime-manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    target_manifest = manifest.get("targets", {}).get(target)
    if not isinstance(target_manifest, dict):
        raise SystemExit(f"runtime manifest has no target {target}")
    asset = target_manifest.get("asset")
    if not isinstance(asset, dict):
        raise SystemExit(f"runtime manifest target {target} has no asset")
    expected_sha = asset.get("sha256")
    actual_sha = sha256_file(runtime_archive)
    if expected_sha != actual_sha:
        raise SystemExit(
            f"runtime archive sha256 {actual_sha} != manifest value {expected_sha}"
        )
    asset_name = asset.get("name")
    if not isinstance(asset_name, str) or not asset_name:
        raise SystemExit(f"runtime manifest target {target} has no asset name")
    asset["url"] = asset_url
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    executable = "unica-bootstrap.exe" if target == "win-x64" else "unica-bootstrap"
    bootstrap = prepared / "bootstrap" / "bin" / target / executable
    if not bootstrap.is_file():
        raise SystemExit(f"packaged bootstrap is missing: {bootstrap}")
    if target != "win-x64":
        bootstrap.chmod(bootstrap.stat().st_mode | 0o755)
    return prepared, bootstrap


def consumer_path(target: str) -> str:
    if target != "win-x64":
        return "/usr/bin:/bin"
    system_root = os.environ.get("SystemRoot", r"C:\Windows")
    program_files = os.environ.get("ProgramFiles", r"C:\Program Files")
    return os.pathsep.join(
        [
            str(Path(system_root) / "System32"),
            system_root,
            str(Path(program_files) / "Git" / "cmd"),
        ]
    )


def smoke(
    plugin_root: Path,
    runtime_archive: Path,
    target: str,
    timeout_seconds: float,
) -> None:
    with tempfile.TemporaryDirectory(prefix="unica-bootstrap-smoke-") as directory:
        root = Path(directory)
        assets = root / "assets"
        assets.mkdir()
        archive_copy = assets / runtime_archive.name
        shutil.copy2(runtime_archive, archive_copy)

        handler = partial(QuietHandler, directory=str(assets))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/{archive_copy.name}"
            prepared, bootstrap = prepare_plugin(
                plugin_root, archive_copy, target, root, url
            )
            environment = os.environ.copy()
            environment["CODEX_HOME"] = str(root / "codex-home")
            environment["UNICA_RUNTIME_CACHE_DIR"] = str(root / "runtime-cache")
            environment["PATH"] = consumer_path(target)
            if shutil.which("node", path=environment["PATH"]):
                raise SystemExit("Node.js leaked into the bootstrap consumer PATH")
            try:
                result = subprocess.run(
                    [str(bootstrap), "verify", "--plugin-root", str(prepared)],
                    capture_output=True,
                    text=True,
                    timeout=timeout_seconds,
                    check=False,
                    env=environment,
                )
            except subprocess.TimeoutExpired as error:
                raise SystemExit(
                    f"packaged bootstrap smoke timed out after {timeout_seconds:g}s"
                ) from error
            if result.returncode != 0:
                detail = result.stderr.strip() or result.stdout.strip() or "no process output"
                raise SystemExit(
                    f"packaged bootstrap exited with {result.returncode}: {detail}"
                )
            if "verified Unica runtime" not in result.stderr:
                raise SystemExit("packaged bootstrap did not report successful MCP verification")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--runtime-archive", required=True, type=Path)
    parser.add_argument(
        "--target", required=True, choices=("darwin-arm64", "linux-x64", "win-x64")
    )
    parser.add_argument("--timeout-seconds", type=float, default=90)
    args = parser.parse_args()
    smoke(
        args.plugin_root.resolve(),
        args.runtime_archive.resolve(),
        args.target,
        args.timeout_seconds,
    )
    print(f"verified packaged Unica bootstrap and runtime for {args.target}")


if __name__ == "__main__":
    main()
