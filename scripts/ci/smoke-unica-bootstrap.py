#!/usr/bin/env python3
"""Exercise the final packaged bootstrap with a Node-free consumer PATH."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


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
    target: str,
    timeout_seconds: float,
    *,
    expect_download_failure: bool,
) -> None:
    executable = "unica-bootstrap.exe" if target == "win-x64" else "unica-bootstrap"
    bootstrap = plugin_root / "bootstrap" / "bin" / target / executable
    if not bootstrap.is_file():
        raise SystemExit(f"packaged bootstrap is missing: {bootstrap}")
    if target != "win-x64":
        bootstrap.chmod(bootstrap.stat().st_mode | 0o755)

    with tempfile.TemporaryDirectory(prefix="unica-bootstrap-smoke-") as directory:
        root = Path(directory)
        environment = os.environ.copy()
        environment["CODEX_HOME"] = str(root / "codex-home")
        environment["UNICA_RUNTIME_CACHE_DIR"] = str(root / "runtime-cache")
        environment["PATH"] = consumer_path(target)
        if shutil.which("node", path=environment["PATH"]):
            raise SystemExit("Node.js leaked into the bootstrap consumer PATH")
        try:
            result = subprocess.run(
                [str(bootstrap), "verify", "--plugin-root", str(plugin_root)],
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

    detail = "\n".join(part.strip() for part in (result.stderr, result.stdout) if part.strip())
    if "overflowed its stack" in detail:
        raise SystemExit(f"packaged bootstrap overflowed its stack: {detail}")
    if expect_download_failure:
        if result.returncode == 0:
            raise SystemExit("packaged bootstrap unexpectedly downloaded an unpublished runtime")
        if "failed to download" not in detail:
            raise SystemExit(
                "packaged bootstrap did not reach the expected controlled download failure: "
                f"{detail or 'no process output'}"
            )
        return
    if result.returncode != 0:
        raise SystemExit(
            f"packaged bootstrap exited with {result.returncode}: "
            f"{detail or 'no process output'}"
        )
    if not (
        "verified Unica " in result.stderr
        and " package, runtime, and MCP tools at " in result.stderr
    ):
        raise SystemExit("packaged bootstrap did not report successful MCP verification")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument(
        "--target", required=True, choices=("darwin-arm64", "linux-x64", "win-x64")
    )
    parser.add_argument("--timeout-seconds", type=float, default=90)
    parser.add_argument("--expect-download-failure", action="store_true")
    args = parser.parse_args()
    smoke(
        args.plugin_root.resolve(),
        args.target,
        args.timeout_seconds,
        expect_download_failure=args.expect_download_failure,
    )
    outcome = "controlled download failure" if args.expect_download_failure else "runtime MCP"
    print(f"verified packaged Unica bootstrap {outcome} for {args.target}")


if __name__ == "__main__":
    main()
