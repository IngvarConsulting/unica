#!/usr/bin/env python3
"""Build and verify the ordinary binary shared by Python product suites.

The prepared path is deliberately outside Cargo's target directory. A later
Receipt build must never replace bytes that an acceptance suite is executing.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

MANIFEST_ENV = "UNICA_PREPARED_BINARY_MANIFEST"
# Default Cargo target directory of a standalone product build. The CI job
# passes its own `target`: the product shares dependency artifacts with the
# test archives, and its bytes are copied out before any later build.
PRODUCT_TARGET = Path(".build/ci-product-target")
SCHEMA = 1
ORDINARY_FEATURES = ["default"]
# One local build per process and checkout; CI never reaches it.
_LOCAL_BUILDS: dict[Path, Path] = {}
LOCAL_BUILD = ["cargo", "build", "--quiet", "--locked", "-p", "unica-coder", "--bin", "unica"]


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def output(root: Path, command: list[str]) -> str:
    return subprocess.check_output(command, cwd=root, text=True).strip()


def source_digest(root: Path) -> str:
    """Include uncommitted tracked files and new, nonignored source files."""
    paths = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=root,
    ).split(b"\0")
    result = hashlib.sha256()
    for raw in sorted(set(paths) - {b""}):
        path = root / os.fsdecode(raw)
        result.update(raw + b"\0")
        if path.is_symlink():
            result.update(b"link\0" + os.fsencode(os.readlink(path)))
        elif path.is_file():
            result.update((b"executable\0" if path.stat().st_mode & 0o111 else b"file\0"))
            result.update(bytes.fromhex(digest(path)))
        elif not path.exists():
            result.update(b"deleted\0")
        else:
            raise ValueError(f"unsupported source input: {path}")
    return result.hexdigest()


def host_target(root: Path) -> str:
    for line in output(root, ["rustc", "-vV"]).splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise ValueError("rustc did not report a host target")


def identity(root: Path) -> dict:
    return {
        "sha": output(root, ["git", "rev-parse", "HEAD"]),
        "sourceDigest": source_digest(root),
        "os": platform.system(),
        "target": host_target(root),
        "toolchain": output(root, ["rustc", "-vV"]),
        "cfg": sorted(output(root, ["rustc", "--print", "cfg"]).splitlines()),
        "rustflags": {name: os.environ.get(name, "") for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS")},
        "targetRustflags": {name: value for name, value in sorted(os.environ.items()) if name.startswith("CARGO_TARGET_") and name.endswith("_RUSTFLAGS")},
        "lockSha256": digest(root / "Cargo.lock"),
    }


def verify(manifest_path: Path, root: Path, *, expected: dict | None = None) -> Path:
    manifest_path = manifest_path.resolve(strict=True)
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    if document.get("schemaVersion") != SCHEMA or document.get("kind") != "ordinary-product":
        raise ValueError("unknown prepared binary manifest")
    expected = identity(root) if expected is None else expected
    if document.get("identity") != expected:
        raise ValueError("prepared binary belongs to another source tree, target, toolchain or cfg")
    if document.get("profile") != "dev" or document.get("features") != ORDINARY_FEATURES:
        raise ValueError("prepared binary is not the ordinary production configuration")
    name = "unica.exe" if expected["os"] == "Windows" else "unica"
    if document.get("binary") != name:
        raise ValueError("invalid prepared binary path")
    binary = manifest_path.parent / name
    if binary.is_symlink() or not binary.is_file():
        raise ValueError("prepared binary is missing or is a link")
    if digest(binary) != document.get("sha256") or binary.stat().st_size != document.get("bytes"):
        raise ValueError("prepared binary hash or size mismatch")
    # Windows does not preserve Unix modes. The declared archive mode is still
    # checked before extraction; native execution checks the actual bytes.
    if document.get("mode") != 0o555 or (os.name != "nt" and stat.S_IMODE(binary.stat().st_mode) != 0o555):
        raise ValueError("prepared binary is not immutable and executable")
    return binary


def binary_path(root: Path) -> Path:
    manifest = os.environ.get(MANIFEST_ENV)
    if manifest is not None:
        if not manifest:
            raise ValueError(f"{MANIFEST_ENV} is empty")
        return verify(Path(manifest), root)
    return (root / os.environ.get("CARGO_TARGET_DIR", "target") / "debug" / ("unica.exe" if os.name == "nt" else "unica")).resolve()


def ensure_binary(root: Path) -> Path:
    if MANIFEST_ENV in os.environ:
        return binary_path(root)
    # CI Python suites consume the verified bytes of the `product` step. A job
    # without the manifest is a wiring defect, not a reason to compile again.
    if os.environ.get("GITHUB_ACTIONS") == "true":
        raise ValueError(f"{MANIFEST_ENV} is unset in CI; Python suites never build the product")
    key = root.resolve()
    if key not in _LOCAL_BUILDS:
        print(f"prepared_binary: {MANIFEST_ENV} is unset, building the product: {' '.join(LOCAL_BUILD)}\n"
              f"prepared_binary: to reuse one verified build across runs of an unchanged tree, run "
              f"`python scripts/ci/prepared_binary.py build --destination DIR --archive FILE` (a separate "
              f"build into .build/ci-product-target) and set {MANIFEST_ENV}=DIR/manifest.json; unset it "
              f"after editing sources, the manifest then refuses",
              file=sys.stderr)
        subprocess.run(LOCAL_BUILD, cwd=root, check=True)
        binary = binary_path(root)
        if not binary.is_file():
            raise ValueError(f"Cargo did not produce {binary}")
        _LOCAL_BUILDS[key] = binary
    return _LOCAL_BUILDS[key]


def build(root: Path, destination: Path, target_dir: Path = PRODUCT_TARGET) -> Path:
    if destination.exists():
        raise ValueError(f"prepared destination already exists: {destination}")
    before = identity(root)
    # A shared target may later be rebuilt with other features (Receipt):
    # the copy below, outside the target, is what Python executes.
    target = root / target_dir
    command = ["cargo", "build", "--locked", "-p", "unica-coder", "--bin", "unica",
               "--target", before["target"], "--target-dir", str(target), "--message-format", "json"]
    completed = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, check=True)
    artifacts = []
    closure, build_cfg = [], []
    for line in completed.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact":
            closure.append({"packageId": message["package_id"], "target": message["target"]["name"],
                            "kind": message["target"]["kind"], "features": message["features"],
                            "profile": message["profile"]})
        elif message.get("reason") == "build-script-executed":
            build_cfg.append({"packageId": message["package_id"], "cfg": message.get("cfgs", [])})
        if message.get("reason") == "compiler-artifact" and message.get("target", {}).get("name") == "unica" and message.get("executable"):
            artifacts.append(message)
    if len(artifacts) != 1 or artifacts[0].get("features") != ORDINARY_FEATURES or artifacts[0].get("profile", {}).get("test"):
        raise ValueError("Cargo did not describe exactly one ordinary product binary")
    if identity(root) != before:
        raise ValueError("source inputs changed during the ordinary build")
    destination.mkdir(parents=True)
    name = "unica.exe" if before["os"] == "Windows" else "unica"
    binary = destination / name
    shutil.copyfile(artifacts[0]["executable"], binary)
    binary.chmod(0o555)
    manifest = destination / "manifest.json"
    manifest.write_text(json.dumps({
        "schemaVersion": SCHEMA, "kind": "ordinary-product", "identity": before,
        "profile": "dev", "features": ORDINARY_FEATURES, "binary": name,
        "sha256": digest(binary), "bytes": binary.stat().st_size, "mode": 0o555,
        "cargoProfile": artifacts[0]["profile"],
        "buildClosure": closure, "buildScriptCfg": build_cfg,
    }, indent=2) + "\n", encoding="utf-8")
    verify(manifest, root, expected=before)
    return manifest


def pack(manifest: Path, archive: Path, root: Path) -> None:
    binary = verify(manifest, root)
    archive.parent.mkdir(parents=True, exist_ok=True)
    with archive.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as zipped, tarfile.open(fileobj=zipped, mode="w") as tar:
        for path, mode in ((manifest, 0o444), (binary, 0o555)):
            data = path.read_bytes()
            member = tarfile.TarInfo(path.name)
            member.size, member.mode, member.mtime = len(data), mode, 0
            tar.addfile(member, io.BytesIO(data))


def unpack(archive: Path, destination: Path, root: Path) -> Path:
    if destination.exists():
        raise ValueError(f"prepared destination already exists: {destination}")
    expected = identity(root)
    name = "unica.exe" if expected["os"] == "Windows" else "unica"
    with tarfile.open(archive, "r:gz") as tar:
        members = tar.getmembers()
        if len(members) != 2 or {m.name for m in members} != {"manifest.json", name}:
            raise ValueError("prepared archive has unexpected or missing members")
        payload = {}
        for member in members:
            mode = 0o444 if member.name == "manifest.json" else 0o555
            if not member.isfile() or member.mode != mode or member.mtime != 0 or member.uid != 0 or member.gid != 0:
                raise ValueError("prepared archive has an unsafe member or unexpected metadata")
            payload[member.name] = tar.extractfile(member).read()
    document = json.loads(payload["manifest.json"])
    if document.get("sha256") != hashlib.sha256(payload[name]).hexdigest() or document.get("bytes") != len(payload[name]):
        raise ValueError("prepared archive payload mismatch")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".prepared-", dir=destination.parent) as temporary:
        staged = Path(temporary)
        for filename, data in payload.items():
            path = staged / filename
            path.write_bytes(data)
            path.chmod(0o444 if filename == "manifest.json" else 0o555)
        verify(staged / "manifest.json", root, expected=expected)
        if destination.exists():
            raise ValueError(f"prepared destination already exists: {destination}")
        staged.rename(destination)
    return destination.resolve() / name


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "verify", "unpack"))
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--target-dir", type=Path, default=PRODUCT_TARGET, help="Cargo target directory of the build")
    args = parser.parse_args()
    if args.action == "build":
        if args.destination is None:
            parser.error("build requires --destination")
        manifest = build(args.repo_root, args.destination, args.target_dir)
        if args.archive is not None:
            pack(manifest, args.archive, args.repo_root)
        print(manifest.resolve())
    elif args.action == "unpack":
        if args.destination is None or args.archive is None:
            parser.error("unpack requires --destination and --archive")
        print(unpack(args.archive, args.destination, args.repo_root))
    else:
        if args.manifest is None:
            parser.error("verify requires --manifest")
        print(verify(args.manifest, args.repo_root))


if __name__ == "__main__":
    main()
