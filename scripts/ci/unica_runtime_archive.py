from __future__ import annotations

import hashlib
import json
import re
import tarfile
import unicodedata
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any


SHA40 = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
WINDOWS_FORBIDDEN_CHARS = frozenset('<>:"|?*')
WINDOWS_RESERVED_NAMES = frozenset(
    {"CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"}
    | {f"COM{number}" for number in range(1, 10)}
    | {f"LPT{number}" for number in range(1, 10)}
)


@dataclass(frozen=True)
class RuntimeArchiveFile:
    path: PurePosixPath
    sha256: str
    size: int
    executable: bool
    payload: bytes


@dataclass(frozen=True)
class DeclaredFile:
    path: PurePosixPath
    sha256: str
    size: int
    executable: bool


def _object(value: Any, *, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise SystemExit(f"{field} must be an object")
    return value


def _exact_fields(value: dict[str, Any], expected: set[str], *, field: str) -> None:
    missing = sorted(expected - value.keys())
    unknown = sorted(value.keys() - expected)
    if missing or unknown:
        raise SystemExit(
            f"{field} fields mismatch: missing={missing}, unknown={unknown}"
        )


def _safe_path(value: Any, *, field: str) -> PurePosixPath:
    if not isinstance(value, str) or not value:
        raise SystemExit(f"{field} must be a non-empty relative path")
    if value.startswith("/") or "\\" in value or "\x00" in value:
        raise SystemExit(f"unsafe {field}: {value!r}")
    parts = value.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise SystemExit(f"unsafe {field}: {value!r}")
    for part in parts:
        normalized = unicodedata.normalize("NFKC", part)
        device_name = normalized.split(".", maxsplit=1)[0].upper()
        if (
            part.endswith((".", " "))
            or any(character in WINDOWS_FORBIDDEN_CHARS for character in part)
            or any(ord(character) < 32 for character in part)
            or device_name in WINDOWS_RESERVED_NAMES
        ):
            raise SystemExit(f"unsafe portable path in {field}: {value!r}")
    path = PurePosixPath(value)
    if path.is_absolute() or path.as_posix() != value:
        raise SystemExit(f"unsafe {field}: {value!r}")
    return path


def _portable_path_key(path: PurePosixPath) -> str:
    return "/".join(
        unicodedata.normalize("NFKC", part).casefold() for part in path.parts
    )


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise SystemExit(f"duplicate JSON key in manifest: {key}")
        result[key] = value
    return result


def _manifest(payload: bytes) -> dict[str, Any]:
    try:
        value = json.loads(
            payload.decode("utf-8"),
            object_pairs_hook=_unique_object,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise SystemExit(f"invalid UTF-8 JSON manifest: {exc}") from exc
    document = _object(value, field="manifest")
    _exact_fields(
        document,
        {
            "schemaVersion",
            "releaseTag",
            "source",
            "target",
            "entrypoints",
            "builder",
            "files",
        },
        field="manifest",
    )
    if document["schemaVersion"] != 1:
        raise SystemExit(
            f"unsupported runtime archive schemaVersion: {document['schemaVersion']}"
        )
    return document


def _source(value: Any) -> dict[str, Any]:
    source = _object(value, field="source")
    _exact_fields(source, {"ref", "commit", "tree", "patches"}, field="source")
    if not isinstance(source["ref"], str) or not source["ref"]:
        raise SystemExit("source.ref must be a non-empty string")
    for field in ("commit", "tree"):
        if not isinstance(source[field], str) or not SHA40.fullmatch(source[field]):
            raise SystemExit(
                f"source.{field} must be 40 lowercase hexadecimal characters"
            )
    patches = source["patches"]
    if not isinstance(patches, list):
        raise SystemExit("source.patches must be an array")
    for index, item in enumerate(patches):
        patch = _object(item, field=f"source.patches[{index}]")
        _exact_fields(
            patch,
            {"path", "sha256"},
            field=f"source.patches[{index}]",
        )
        _safe_path(patch["path"], field=f"source.patches[{index}].path")
        if not isinstance(patch["sha256"], str) or not SHA256.fullmatch(
            patch["sha256"]
        ):
            raise SystemExit(
                f"source.patches[{index}].sha256 must be 64 lowercase hexadecimal characters"
            )
    return source


def _target(value: Any) -> dict[str, str]:
    target = _object(value, field="target")
    _exact_fields(target, {"key", "triple"}, field="target")
    for field in ("key", "triple"):
        if not isinstance(target[field], str) or not target[field]:
            raise SystemExit(f"target.{field} must be a non-empty string")
    return {"key": target["key"], "triple": target["triple"]}


def _entrypoints(value: Any) -> dict[str, PurePosixPath]:
    entrypoints = _object(value, field="entrypoints")
    if len(entrypoints) < 2:
        raise SystemExit("entrypoints must contain at least two programs")
    result: dict[str, PurePosixPath] = {}
    for name, path in entrypoints.items():
        if not isinstance(name, str) or not name:
            raise SystemExit("entrypoints names must be non-empty strings")
        result[name] = _safe_path(path, field=f"entrypoints.{name}")
    if len(set(result.values())) != len(result):
        raise SystemExit("entrypoints paths must be unique")
    return result


def _files(value: Any) -> tuple[DeclaredFile, ...]:
    if not isinstance(value, list) or not value:
        raise SystemExit("files must be a non-empty array")
    result: list[DeclaredFile] = []
    paths: set[PurePosixPath] = set()
    portable_paths: dict[str, PurePosixPath] = {}
    for index, raw_item in enumerate(value):
        item = _object(raw_item, field=f"files[{index}]")
        _exact_fields(
            item,
            {"path", "sha256", "size", "executable"},
            field=f"files[{index}]",
        )
        path = _safe_path(item["path"], field=f"files[{index}].path")
        if path in paths:
            raise SystemExit(f"duplicate file path in manifest: {path.as_posix()}")
        paths.add(path)
        portable_key = _portable_path_key(path)
        conflicting = portable_paths.get(portable_key)
        if conflicting is not None:
            raise SystemExit(
                "portable path collision in manifest: "
                f"{conflicting.as_posix()} and {path.as_posix()}"
            )
        portable_paths[portable_key] = path
        digest = item["sha256"]
        if not isinstance(digest, str) or not SHA256.fullmatch(digest):
            raise SystemExit(
                f"files[{index}].sha256 must be 64 lowercase hexadecimal characters"
            )
        size = item["size"]
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            raise SystemExit(f"files[{index}].size must be a non-negative integer")
        executable = item["executable"]
        if not isinstance(executable, bool):
            raise SystemExit(f"files[{index}].executable must be a boolean")
        result.append(DeclaredFile(path, digest, size, executable))
    if [item.path.as_posix() for item in result] != sorted(
        item.path.as_posix() for item in result
    ):
        raise SystemExit("files must be sorted by path")
    return tuple(result)


def _read_members(path: Path) -> dict[str, tuple[bytes, int]]:
    members: dict[str, tuple[bytes, int]] = {}
    try:
        with tarfile.open(path, "r:gz") as archive:
            for member in archive.getmembers():
                name = _safe_path(member.name, field="archive member").as_posix()
                if name in members:
                    raise SystemExit(f"duplicate archive member: {name}")
                if not member.isreg():
                    raise SystemExit(
                        f"archive member must be an ordinary file: {name}"
                    )
                if name != "manifest.json" and not name.startswith("payload/"):
                    raise SystemExit(f"unsafe archive member outside payload: {name}")
                if member.mode not in (0o644, 0o755):
                    raise SystemExit(
                        f"unsafe archive member mode for {name}: {oct(member.mode)}"
                    )
                stream = archive.extractfile(member)
                if stream is None:
                    raise SystemExit(f"archive member cannot be read: {name}")
                members[name] = (stream.read(), member.mode)
    except (OSError, tarfile.TarError) as exc:
        raise SystemExit(
            f"cannot read runtime archive: {type(exc).__name__}"
        ) from exc
    return members


def load_verified_archive(
    path: Path,
    *,
    release_tag: str,
    source_commit: str,
    target: dict[str, str],
    entrypoints: dict[str, str],
) -> tuple[RuntimeArchiveFile, ...]:
    members = _read_members(path)
    manifest_member = members.get("manifest.json")
    if manifest_member is None:
        raise SystemExit("runtime archive is missing manifest.json")
    if manifest_member[1] != 0o644:
        raise SystemExit("manifest.json mode must be 0644")
    document = _manifest(manifest_member[0])
    if document["releaseTag"] != release_tag:
        raise SystemExit(
            f"releaseTag mismatch: {document['releaseTag']} != {release_tag}"
        )
    source = _source(document["source"])
    if source["commit"] != source_commit:
        raise SystemExit(
            f"source.commit mismatch: {source['commit']} != {source_commit}"
        )
    actual_target = _target(document["target"])
    if actual_target != target:
        raise SystemExit(f"target mismatch: {actual_target} != {target}")
    actual_entrypoints = _entrypoints(document["entrypoints"])
    expected_entrypoints = {
        name: _safe_path(value, field=f"expected entrypoints.{name}")
        for name, value in entrypoints.items()
    }
    if actual_entrypoints != expected_entrypoints:
        raise SystemExit(
            f"entrypoints mismatch: {actual_entrypoints} != {expected_entrypoints}"
        )
    builder = _object(document["builder"], field="builder")
    if not isinstance(builder.get("kind"), str) or not builder["kind"]:
        raise SystemExit("builder.kind must be a non-empty string")
    declared = _files(document["files"])
    expected_members = {f"payload/{item.path.as_posix()}" for item in declared}
    actual_members = set(members) - {"manifest.json"}
    if actual_members != expected_members:
        raise SystemExit(
            "runtime archive file set mismatch: "
            f"missing={sorted(expected_members - actual_members)}, "
            f"unexpected={sorted(actual_members - expected_members)}"
        )

    result: list[RuntimeArchiveFile] = []
    for item in declared:
        member_name = f"payload/{item.path.as_posix()}"
        payload, mode = members[member_name]
        if len(payload) != item.size:
            raise SystemExit(f"size mismatch for {member_name}")
        digest = hashlib.sha256(payload).hexdigest()
        if digest != item.sha256:
            raise SystemExit(f"sha256 mismatch for {member_name}")
        expected_mode = 0o755 if item.executable else 0o644
        if mode != expected_mode:
            raise SystemExit(f"mode mismatch for {member_name}")
        result.append(
            RuntimeArchiveFile(
                item.path,
                item.sha256,
                item.size,
                item.executable,
                payload,
            )
        )

    by_path = {item.path: item for item in result}
    selected: list[RuntimeArchiveFile] = []
    for name, entrypoint in actual_entrypoints.items():
        item = by_path.get(entrypoint)
        if item is None:
            raise SystemExit(
                f"entrypoints.{name} is missing from payload: {entrypoint.as_posix()}"
            )
        if not item.executable:
            raise SystemExit(
                f"entrypoints.{name} is not executable: {entrypoint.as_posix()}"
            )
        selected.append(item)
    if len({item.sha256 for item in selected}) != 1:
        raise SystemExit("multidist entrypoints must be byte-identical")
    return tuple(result)
