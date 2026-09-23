#!/usr/bin/env python3
"""Exercise the final packaged bootstrap with a Node-free consumer PATH."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


# Valid as a manifest value, but no archive can ever hash to it.
UNMATCHABLE_SHA256 = "0" * 64


def neutralise_published_checksums(manifest_path: Path) -> str:
    """Make the manifest describe runtime bytes that cannot exist, and return the original text.

    A pull-request package is built with the release tag of the version in the
    tree, and the manifest is self-consistent by design: `release.tag` must equal
    `v{pluginVersion}` and the asset URL must match that tag, so the probe cannot
    simply point it at an unpublished tag.

    Once that version is released, a pull request that does not change the
    runtime rebuilds byte-identical archives, the checksums match the published
    assets, and the download the probe expects to fail succeeds instead. Only
    hosts whose builds are not byte-reproducible, currently win-x64, kept
    failing by accident.

    Replacing the archive checksum removes that dependency on release state: the
    bootstrap either cannot fetch the asset or rejects the bytes it fetched, and
    both are the controlled failure this probe asserts.

    Каждый артефакт, а не только ядро: манифест обязан описывать байты, которых
    не существует нигде.
    """
    if not manifest_path.is_file():
        raise SystemExit(f"packaged runtime manifest is missing: {manifest_path}")
    original = manifest_path.read_text(encoding="utf-8")
    manifest = json.loads(original)
    neutralised = 0
    for artifact in manifest.get("artifacts", {}).values():
        for runtime in artifact.get("targets", {}).values():
            runtime["asset"]["sha256"] = UNMATCHABLE_SHA256
            neutralised += 1
    if not neutralised:
        # Молчаливый ноль оставил бы зонд холостым: он проверял бы, доехали ли
        # опубликованные байты, а не то, что отказ управляем.
        raise SystemExit(
            f"packaged runtime manifest declares no artifact assets: {manifest_path}"
        )
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return original


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


def run_bootstrap_command(
    command: list[str], environment: dict[str, str], timeout_seconds: float
) -> tuple[int, str, str]:
    # Regular files do not need EOF from descendants after the parent exits.
    # A captured pipe can remain open in a Windows grandchild after timeout.
    with tempfile.TemporaryFile(mode="w+t", encoding="utf-8") as stdout_log, tempfile.TemporaryFile(
        mode="w+t", encoding="utf-8"
    ) as stderr_log:
        try:
            result = subprocess.run(
                command,
                stdout=stdout_log,
                stderr=stderr_log,
                timeout=timeout_seconds,
                check=False,
                env=environment,
            )
        except subprocess.TimeoutExpired as error:
            stderr_log.seek(0)
            tail = stderr_log.read()[-2000:].strip()
            raise SystemExit(
                f"packaged bootstrap smoke timed out after {timeout_seconds:g}s"
                + (f": {tail}" if tail else "")
            ) from error
        stdout_log.seek(0)
        stderr_log.seek(0)
        stdout = result.stdout if result.stdout is not None else stdout_log.read()
        stderr = result.stderr if result.stderr is not None else stderr_log.read()
    return result.returncode, stdout, stderr


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

    manifest_path = plugin_root / "runtime-manifest.json"
    original_manifest = (
        neutralise_published_checksums(manifest_path) if expect_download_failure else None
    )

    # Every exit from here restores the manifest, including the consumer-PATH
    # guard below, which raises before the probe ever starts. The payload is a
    # build artifact other steps read, so it must not be left neutralised.
    try:
        with tempfile.TemporaryDirectory(prefix="unica-bootstrap-smoke-") as directory:
            # macOS /var is a symlink to /private/var; the daemon deliberately
            # refuses symlinks in its state path. Give it a physical root.
            root = Path(directory).resolve()
            environment = os.environ.copy()
            environment["CODEX_HOME"] = str(root / "codex-home")
            environment["UNICA_RUNTIME_CACHE_DIR"] = str(root / "runtime-cache")
            environment["UNICA_PROVIDER_STATE_DIR"] = str(root / "provider-state")
            environment["PATH"] = consumer_path(target)
            if shutil.which("node", path=environment["PATH"]):
                raise SystemExit("Node.js leaked into the bootstrap consumer PATH")
            returncode, stdout, stderr = run_bootstrap_command(
                [str(bootstrap), "verify", "--plugin-root", str(plugin_root)],
                environment,
                timeout_seconds,
            )
    finally:
        if original_manifest is not None:
            manifest_path.write_text(original_manifest, encoding="utf-8")

    detail = "\n".join(part.strip() for part in (stderr, stdout) if part.strip())
    if "overflowed its stack" in detail:
        raise SystemExit(f"packaged bootstrap overflowed its stack: {detail}")
    if expect_download_failure:
        if returncode == 0:
            raise SystemExit(
                "packaged bootstrap accepted a runtime archive whose checksum was neutralised"
            )
        controlled_failure = any(
            marker in detail
            for marker in ("failed to download", "runtime archive sha256")
        )
        if not controlled_failure:
            raise SystemExit(
                "packaged bootstrap did not reach the expected controlled download failure: "
                f"{detail or 'no process output'}"
            )
        return
    if returncode != 0:
        raise SystemExit(
            f"packaged bootstrap exited with {returncode}: "
            f"{detail or 'no process output'}"
        )
    if not (
        "verified Unica " in stderr
        and " package, runtime, and MCP tools at " in stderr
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
