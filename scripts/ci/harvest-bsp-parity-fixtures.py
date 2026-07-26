#!/usr/bin/env python3
"""Harvest deterministic text fixtures from a local BSP checkout."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import xml.parsers.expat
from pathlib import Path
from typing import Any


MAX_FIXTURE_SIZE = 256 * 1024
TEXT_SUFFIXES = {".xml", ".bsl", ".json"}
FORBIDDEN_SUFFIXES = (".db", ".db-wal", ".db-shm")
BSP_2_21_TO_2_20_RECIPE = "bsp-2.21-to-2.20-v1"
BSP_2_21_TO_2_20_DERIVATION = {
    "kind": "profile-projection",
    "platformLine": "8.3.27",
    "exportFormat": "2.20",
    "recipe": BSP_2_21_TO_2_20_RECIPE,
}
METADATA_ROOT_LIMITS = {
    "AccumulationRegisters": 1,
    "Catalogs": 1,
    "CommonModules": 1,
    "CommonModules.Module.bsl": 1,
    "Documents": 1,
    "Enums": 1,
    "InformationRegisters": 1,
    "Languages": 1,
    "Reports": 1,
}
CATEGORY_LIMITS = {
    "cf": 1,
    "forms": 4,
    "dcs": 3,
    "mxl": 2,
    "roles": 2,
    "subsystems": 2,
}


def _source_root(bsp_root: Path) -> Path:
    source_root = bsp_root / "src" / "cf"
    if not source_root.is_dir():
        raise FileNotFoundError(f"BSP source root not found: {source_root}")
    return source_root


def _is_same_or_inside(path: Path, parent: Path) -> bool:
    return path == parent or path.is_relative_to(parent)


def _filesystem_root(path: Path) -> Path:
    return Path(path.anchor or "/").resolve()


def _is_empty_dir(path: Path) -> bool:
    return path.is_dir() and not any(path.iterdir())


def _has_bsp_harvest_marker(path: Path) -> bool:
    manifest_path = path / "manifest.json"
    if not manifest_path.is_file():
        return False
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return (
        isinstance(manifest, dict)
        and manifest.get("schemaVersion") in {1, 2}
        and isinstance(manifest.get("bsp"), dict)
        and isinstance(manifest.get("files"), list)
    )


def _ensure_safe_out_root(*, bsp_root: Path, source_root: Path, out_root: Path) -> None:
    if out_root.is_symlink():
        raise ValueError(f"refusing to delete symlink out_root: {out_root}")

    out_resolved = out_root.resolve()
    source_resolved = source_root.resolve()
    bsp_resolved = bsp_root.resolve()
    forbidden_exact = {
        _filesystem_root(out_resolved),
        Path.home().resolve(),
        Path.cwd().resolve(),
    }
    if out_resolved in forbidden_exact:
        raise ValueError(f"refusing to delete dangerous out_root: {out_resolved}")

    if out_root.exists() and not out_root.is_dir():
        raise ValueError(f"refusing to replace non-directory out_root: {out_root}")

    for protected_root in (source_resolved, bsp_resolved):
        if _is_same_or_inside(out_resolved, protected_root):
            raise ValueError(f"refusing to delete out_root inside BSP source: {out_resolved}")
        if _is_same_or_inside(protected_root, out_resolved):
            raise ValueError(f"refusing to delete out_root containing BSP source: {out_resolved}")

    if out_root.exists() and not _is_empty_dir(out_root) and not _has_bsp_harvest_marker(out_root):
        raise ValueError(
            "refusing to delete existing out_root without BSP harvest manifest marker: "
            f"{out_resolved}"
        )


def _is_forbidden_source(rel_path: Path) -> bool:
    if ".build" in rel_path.parts:
        return True
    return rel_path.name.lower().endswith(FORBIDDEN_SUFFIXES)


def _read_utf8_fixture(path: Path) -> tuple[bytes, str] | None:
    if path.suffix.lower() not in TEXT_SUFFIXES:
        return None
    if path.stat().st_size > MAX_FIXTURE_SIZE:
        return None
    payload = path.read_bytes()
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return payload, text


def _fixture_name(parts: tuple[str, ...]) -> str:
    return "__".join(part for part in parts if part and part != "Ext")


def _classify(rel_path: Path, text: str) -> tuple[str, Path] | None:
    parts = rel_path.parts
    if rel_path.as_posix() == "Configuration.xml":
        return "cf", Path("cf") / "Configuration.xml"

    if len(parts) >= 5 and parts[-4] == "Forms" and parts[-2] == "Ext" and parts[-1] == "Form.xml":
        fixture = _fixture_name(parts[:-4] + (parts[-3],))
        return "forms", Path("forms") / fixture / "Form.xml"

    if len(parts) >= 4 and parts[0] == "Roles" and parts[-2] == "Ext" and parts[-1] == "Rights.xml":
        return "roles", Path("roles") / _fixture_name((parts[1],)) / "Rights.xml"

    if len(parts) >= 5 and parts[-4] == "Templates" and parts[-2] == "Ext" and parts[-1] == "Template.xml":
        fixture = _fixture_name(parts[:-4] + (parts[-3],))
        category = "dcs" if "DataCompositionSchema" in text or "СхемаКомпоновки" in fixture else "mxl"
        return category, Path(category) / fixture / "Template.xml"

    if len(parts) == 2 and parts[0] in METADATA_ROOT_LIMITS and rel_path.suffix.lower() == ".xml":
        return "meta", Path("meta") / parts[0] / parts[1]

    if len(parts) >= 4 and parts[0] == "CommonModules" and parts[-2] == "Ext" and parts[-1] == "Module.bsl":
        return "meta", Path("meta") / "CommonModules" / parts[1] / "Module.bsl"

    if parts and parts[0] == "Subsystems" and rel_path.suffix.lower() in {".xml", ".json"}:
        return "subsystems", Path("subsystems") / Path(*parts[1:])

    return None


def _metadata_limit_key(rel_source: Path) -> str:
    if (
        len(rel_source.parts) >= 4
        and rel_source.parts[0] == "CommonModules"
        and rel_source.parts[-2] == "Ext"
        and rel_source.parts[-1] == "Module.bsl"
    ):
        return "CommonModules.Module.bsl"
    return rel_source.parts[0]


def _root_start_tag(payload: bytes, *, target: str) -> tuple[int, int, dict[str, str]]:
    root_start: int | None = None
    root_attributes: dict[str, str] | None = None
    parser = xml.parsers.expat.ParserCreate()

    def capture_root(_name: str, attributes: dict[str, str]) -> None:
        nonlocal root_start, root_attributes
        if root_start is None:
            root_start = parser.CurrentByteIndex
            root_attributes = attributes

    parser.StartElementHandler = capture_root
    try:
        parser.Parse(payload, True)
    except xml.parsers.expat.ExpatError as error:
        raise ValueError(
            f"{BSP_2_21_TO_2_20_RECIPE}: invalid XML in {target}: {error}"
        ) from error

    if root_start is None or root_attributes is None:
        raise ValueError(f"{BSP_2_21_TO_2_20_RECIPE}: XML has no root element: {target}")

    quote: int | None = None
    for index in range(root_start, len(payload)):
        byte = payload[index]
        if quote is not None:
            if byte == quote:
                quote = None
            continue
        if byte in {ord('"'), ord("'")}:
            quote = byte
        elif byte == ord(">"):
            return root_start, index + 1, root_attributes

    raise ValueError(f"{BSP_2_21_TO_2_20_RECIPE}: unterminated root start tag: {target}")


def _replace_exact_once(payload: bytes, *, old: bytes, new: bytes, label: str, target: str) -> bytes:
    count = payload.count(old)
    if count != 1:
        raise ValueError(
            f"{BSP_2_21_TO_2_20_RECIPE}: expected exactly one {label} token "
            f"in {target}, found {count}"
        )
    return payload.replace(old, new, 1)


def _project_bsp_2_21_to_2_20(*, payload: bytes, target: str) -> bytes:
    if Path(target).suffix.lower() != ".xml":
        return payload

    root_start, root_end, root_attributes = _root_start_tag(payload, target=target)
    root_version = root_attributes.get("version")
    if root_version is None:
        if target == "cf/Configuration.xml":
            raise ValueError(
                f"{BSP_2_21_TO_2_20_RECIPE}: Configuration root has no export version"
            )
        return payload
    if root_version != "2.21":
        raise ValueError(
            f"{BSP_2_21_TO_2_20_RECIPE}: unsupported root export version "
            f"{root_version!r} in {target}"
        )

    root_tag = payload[root_start:root_end]
    projected_root = _replace_exact_once(
        root_tag,
        old=b'version="2.21"',
        new=b'version="2.20"',
        label="root version",
        target=target,
    )
    projected = payload[:root_start] + projected_root + payload[root_end:]

    if target != "cf/Configuration.xml":
        return projected

    replacements = (
        (
            b"<ConfigurationExtensionCompatibilityMode>"
            b"Version8_5_1"
            b"</ConfigurationExtensionCompatibilityMode>",
            b"<ConfigurationExtensionCompatibilityMode>"
            b"Version8_3_24"
            b"</ConfigurationExtensionCompatibilityMode>",
            "ConfigurationExtensionCompatibilityMode",
        ),
        (
            b"<InterfaceCompatibilityMode>"
            b"Version8_5EnableTaxi"
            b"</InterfaceCompatibilityMode>",
            b"<InterfaceCompatibilityMode>"
            b"Taxi"
            b"</InterfaceCompatibilityMode>",
            "InterfaceCompatibilityMode",
        ),
        (
            b"<CompatibilityMode>"
            b"Version8_5_1"
            b"</CompatibilityMode>",
            b"<CompatibilityMode>"
            b"Version8_3_24"
            b"</CompatibilityMode>",
            "CompatibilityMode",
        ),
    )
    for old, new, label in replacements:
        projected = _replace_exact_once(
            projected,
            old=old,
            new=new,
            label=label,
            target=target,
        )
    return projected


def _project_record(record: dict[str, Any], *, recipe: str | None) -> dict[str, Any]:
    if recipe is None:
        return record
    if recipe != BSP_2_21_TO_2_20_RECIPE:
        raise ValueError(f"unsupported BSP fixture derivation recipe: {recipe}")

    harvested_payload = record["payload"]
    target_payload = _project_bsp_2_21_to_2_20(
        payload=harvested_payload,
        target=record["target"],
    )
    return {
        **record,
        "harvestedSha256": record["sha256"],
        "harvestedSize": record["size"],
        "payload": target_payload,
        "sha256": hashlib.sha256(target_payload).hexdigest(),
        "size": len(target_payload),
    }


def _selected_records(source_root: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    category_counts = {category: 0 for category in CATEGORY_LIMITS}
    metadata_counts = {root: 0 for root in METADATA_ROOT_LIMITS}
    used_targets: set[str] = set()
    selected_report_name: str | None = None
    selected_report_template_name: str | None = None
    source_resolved = source_root.resolve()

    paths = sorted(
        source_root.rglob("*"),
        key=lambda path: path.relative_to(source_root).as_posix(),
    )
    for path in paths:
        if path.is_symlink() or not path.is_file():
            continue
        if not _is_same_or_inside(path.resolve(), source_resolved):
            continue
        rel_source = path.relative_to(source_root)
        if _is_forbidden_source(rel_source):
            continue
        fixture = _read_utf8_fixture(path)
        if fixture is None:
            continue
        payload, text = fixture
        parts = rel_source.parts
        selected_report_fixture = False
        classified: tuple[str, Path] | None = None
        if (
            selected_report_name is not None
            and len(parts) == 4
            and parts[0] == "Reports"
            and parts[1] == selected_report_name
            and parts[2] == "Templates"
            and rel_source.suffix.lower() == ".xml"
        ):
            candidate_name = rel_source.stem
            if selected_report_template_name is None:
                selected_report_template_name = candidate_name
            if candidate_name == selected_report_template_name:
                selected_report_fixture = True
                classified = "meta", Path("meta") / rel_source
        elif (
            selected_report_name is not None
            and selected_report_template_name is not None
            and len(parts) == 6
            and parts[0] == "Reports"
            and parts[1] == selected_report_name
            and parts[2] == "Templates"
            and parts[3] == selected_report_template_name
            and parts[4] == "Ext"
            and parts[5] == "Template.xml"
        ):
            selected_report_fixture = True
            classified = "meta", Path("meta") / rel_source
        if classified is None:
            classified = _classify(rel_source, text)
        if classified is None:
            continue
        category, rel_target = classified

        if selected_report_fixture:
            pass
        elif category == "meta":
            root_name = _metadata_limit_key(rel_source)
            if root_name in metadata_counts and metadata_counts[root_name] >= METADATA_ROOT_LIMITS[root_name]:
                continue
            if root_name in metadata_counts:
                metadata_counts[root_name] += 1
            if root_name == "Reports":
                selected_report_name = rel_source.stem
        elif category in category_counts:
            if category_counts[category] >= CATEGORY_LIMITS[category]:
                continue
            category_counts[category] += 1

        target_key = rel_target.as_posix()
        if target_key in used_targets:
            continue
        used_targets.add(target_key)
        digest = hashlib.sha256(payload).hexdigest()
        records.append(
            {
                "category": category,
                "payload": payload,
                "sha256": digest,
                "size": len(payload),
                "source": (Path("src") / "cf" / rel_source).as_posix(),
                "target": target_key,
            }
        )

    records.sort(key=lambda record: (record["target"], record["source"]))
    return records


def harvest(
    *,
    bsp_root: Path,
    out_root: Path,
    bsp_ref: str,
    bsp_commit: str,
    recipe: str | None = None,
) -> dict[str, Any]:
    source_root = _source_root(bsp_root)
    _ensure_safe_out_root(bsp_root=bsp_root, source_root=source_root, out_root=out_root)
    records = [
        _project_record(record, recipe=recipe)
        for record in _selected_records(source_root)
    ]

    if out_root.exists():
        shutil.rmtree(out_root)
    out_root.mkdir(parents=True)

    manifest_files: list[dict[str, Any]] = []
    for record in records:
        target = out_root / record["target"]
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(record["payload"])
        manifest_entry = {
            "category": record["category"],
            "sha256": record["sha256"],
            "size": record["size"],
            "source": record["source"],
            "target": record["target"],
        }
        if recipe is not None:
            manifest_entry.update(
                {
                    "harvestedSha256": record["harvestedSha256"],
                    "harvestedSize": record["harvestedSize"],
                }
            )
        manifest_files.append(manifest_entry)

    manifest: dict[str, Any] = {
        "schemaVersion": 2 if recipe is not None else 1,
        "bsp": {
            "ref": bsp_ref,
            "commit": bsp_commit,
        },
        "files": manifest_files,
    }
    if recipe is not None:
        manifest["derivation"] = BSP_2_21_TO_2_20_DERIVATION
    (out_root / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bsp-root", required=True, type=Path)
    parser.add_argument("--out-root", required=True, type=Path)
    parser.add_argument("--bsp-ref", required=True)
    parser.add_argument("--bsp-commit", required=True)
    parser.add_argument("--recipe", choices=[BSP_2_21_TO_2_20_RECIPE])
    args = parser.parse_args()

    manifest = harvest(
        bsp_root=args.bsp_root,
        out_root=args.out_root,
        bsp_ref=args.bsp_ref,
        bsp_commit=args.bsp_commit,
        recipe=args.recipe,
    )
    print(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
