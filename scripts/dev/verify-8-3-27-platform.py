#!/usr/bin/env python3
"""Verify Unica export trees by round-tripping them through 1C 8.3.27.2074."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import os
import signal
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path, PurePosixPath

from lxml import etree

try:
    from scripts.dev.xml_lexical import LexicalXmlError, raw_root_attribute
except ModuleNotFoundError:
    from xml_lexical import LexicalXmlError, raw_root_attribute


XSI_NS = "http://www.w3.org/2001/XMLSchema-instance"
CORE_NS = "http://v8.1c.ru/8.1/data/core"
MD_CLASSES_NS = "http://v8.1c.ru/8.3/MDClasses"
QNAME_TEXT_ELEMENTS = {
    f"{{{CORE_NS}}}Type",
    f"{{{CORE_NS}}}TypeSet",
    f"{{{MD_CLASSES_NS}}}XDTOReturningValueType",
    f"{{{MD_CLASSES_NS}}}XDTOValueType",
}
TYPE_DESCRIPTION_SET_CHILDREN = {
    f"{{{CORE_NS}}}Type",
    f"{{{CORE_NS}}}TypeSet",
    f"{{{CORE_NS}}}TypeId",
}
UUID_TEXT_RE = re.compile(
    r"(?i)(?<![0-9a-f])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-"
    r"[0-9a-f]{4}-[0-9a-f]{12}(?![0-9a-f])"
)


class SourceError(RuntimeError):
    """The selected corpus or platform evidence cannot be trusted."""


class CorpusError(SourceError):
    """The corpus manifest/checkpoint contract is incomplete or inconsistent."""


class PlatformVersionError(SourceError):
    """The version probe completed but did not identify the required platform."""

    def __init__(self, message: str, *, record: dict, version: str, ibcmd_sha256: str):
        super().__init__(message)
        self.record = record
        self.version = version
        self.ibcmd_sha256 = ibcmd_sha256


class PlatformBinaryError(SourceError):
    """The executable bytes do not match the pinned 8.3.27.2074 binary."""

    def __init__(self, message: str, *, ibcmd_sha256: str):
        super().__init__(message)
        self.ibcmd_sha256 = ibcmd_sha256


class PlatformInstallError(SourceError):
    """The complete platform installation does not match its pinned inventory."""

    def __init__(self, message: str, *, inventory: dict):
        super().__init__(message)
        self.inventory = inventory


class CheckpointExecutionError(SourceError):
    """A checkpoint failed after retaining its completed command evidence."""

    def __init__(self, message: str, *, checkpoint: dict):
        super().__init__(message)
        self.checkpoint = checkpoint


EXPECTED_PROFILE = "1c-8.3.27-export-2.20"
EXPECTED_EXPORT_VERSION = "2.20"
EXPECTED_PLATFORM_VERSION = "8.3.27.2074"
EXPECTED_IBCMD_SHA256 = "e00f3c945fb6f60bb2802151df1b4e7ee4f3caaf7c9e24a981020af575fda6e5"
EXPECTED_PLATFORM_INSTALL_SHA256 = "5eb8897c4f7e95876572f2f36943439b0d57e47688314b622f5771e5a22df0ef"
EXPECTED_PLATFORM_INSTALL_FILE_COUNT = 4337
EXPECTED_CASE_CONTRACT_SHA256 = "52d9889946c1e44ebf542721eee8a83c4ba525526f97fb1fe2f4f74074a7a161"
DEFAULT_COMMAND_TIMEOUT_SECONDS = 300.0
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
CASE_ID_RE = re.compile(r"[a-z0-9][a-z0-9-]*\Z")
CHECKPOINT_KINDS = {"configuration", "extension", "epf", "erf"}
EXPECTED_OWNER_TYPES = {
    "configuration": ("Configuration",),
    # Designer XML represents both the base and extension owner with a
    # MDClasses Configuration child; checkpoint paths disambiguate them.
    "extension": ("Configuration", "Configuration"),
    "epf": ("ExternalDataProcessor",),
    "erf": ("ExternalReport",),
}
XML_FAMILY_BY_ROOT_QNAME = {
    "{http://v8.1c.ru/8.1/data-composition-system/schema}DataCompositionSchema": "dcs",
    "{http://v8.1c.ru/8.2/data/spreadsheet}document": "mxl",
    "{http://v8.1c.ru/8.2/managed-application/core}ClientApplicationInterface": "client-application-interface",
    "{http://v8.1c.ru/8.2/roles}Rights": "roles",
    "{http://v8.1c.ru/8.3/MDClasses}MetaDataObject": "metadata",
    "{http://v8.1c.ru/8.3/xcf/scheme}GraphicalSchema": "flowchart",
    "{http://v8.1c.ru/8.3/xcf/logform}Form": "managed-form",
    "{http://v8.1c.ru/8.3/xcf/extrnprops}CommandInterface": "command-interface",
    "{http://v8.1c.ru/8.3/xcf/extrnprops}Help": "help",
    "{http://v8.1c.ru/8.3/xcf/extrnprops}ExchangePlanContent": "exchange-plan-content",
    "{http://v8.1c.ru/8.3/xcf/extrnprops}HomePageWorkArea": "home-page-work-area",
}
MANDATORY_CASE_IDS = frozenset(
    {
        "cf-edit-root-property",
        "cf-edit-set-home-page",
        "cf-edit-set-panels",
        "cf-init-default",
        "cfe-patch-method-bsl-only",
        "cfe-patch-method-catalog-object-module",
        "cfe-patch-method-catalog-manager-module",
        "cfe-patch-method-information-register-record-set-module",
        "cfe-patch-method-catalog-form-module",
        "cfe-patch-method-constant-value-manager-module",
        "code-patch-bsl-only",
        "dcs-compile-owned-template",
        "dcs-edit-add-parameter-after-settings",
        "dcs-edit-modify-field-role-restriction",
        "dcs-edit-owned-template",
        "dcs-edit-set-structure-after-settings",
        "epf-init-managed-form",
        "erf-init-managed-form",
        "form-add-managed",
        "form-compile-managed",
        "form-edit-managed",
        "form-remove-managed",
        "help-add-object",
        "interface-edit-subsystem",
        "meta-compile-accounting-register",
        "meta-compile-accumulation-register",
        "meta-compile-business-process",
        "meta-compile-calculation-register",
        "meta-compile-catalog",
        "meta-compile-chart-of-accounts",
        "meta-compile-chart-of-calculation-types",
        "meta-compile-chart-of-characteristic-types",
        "meta-compile-common-module",
        "meta-compile-constant",
        "meta-compile-data-processor",
        "meta-compile-defined-type",
        "meta-compile-document",
        "meta-compile-document-journal",
        "meta-compile-enum",
        "meta-compile-event-subscription",
        "meta-compile-exchange-plan",
        "meta-compile-http-service",
        "meta-compile-information-register",
        "meta-compile-report",
        "meta-compile-scheduled-job",
        "meta-compile-task",
        "meta-compile-web-service",
        "meta-edit-property",
        "meta-remove-object",
        "mxl-compile-owned-template",
        "role-compile-name-field",
        "subsystem-compile-child",
        "subsystem-edit-add-child",
        "support-edit-bin-only",
        "template-add-binary-data",
        "template-add-data-composition-schema",
        "template-add-html-document",
        "template-add-spreadsheet-document",
        "template-add-text-document",
        "template-remove-object-template",
        "cfe-init-default",
        "cfe-borrow-object",
        "cfe-borrow-managed-form",
    }
)
FORBIDDEN_CREDENTIAL_OPTIONS = {
    "--password",
    "--pwd",
    "--user",
    "--username",
    "--db-user",
    "--db-password",
}


def _expanded_name(name: str) -> str:
    qname = etree.QName(name)
    return f"{{{qname.namespace or ''}}}{qname.localname}"


def _is_xml_ncname(value: str) -> bool:
    """Delegate the full Unicode XML Name production to libxml2."""
    try:
        qname = etree.QName(value)
    except ValueError:
        return False
    return qname.namespace is None and qname.localname == value


def _expanded_lexical_qname(value: str, node: etree._Element, label: str) -> str:
    lexical = value.strip()
    if ":" in lexical:
        prefix, local_name = lexical.split(":", 1)
        valid = _is_xml_ncname(prefix) and _is_xml_ncname(local_name)
    else:
        prefix = None
        local_name = lexical
        valid = _is_xml_ncname(local_name)
    if not valid:
        raise SourceError(f"invalid lexical QName in {label}: {value!r}")
    if prefix is not None:
        namespace = node.nsmap.get(prefix)
        if namespace is None:
            raise SourceError(
                f"unresolved QName prefix {prefix!r} in {label}: {value!r}"
            )
    else:
        namespace = node.nsmap.get(None, "")
    return f"{{{namespace}}}{local_name}"


def _semantic_text(value: str | None, *, indentation: bool):
    if value is None or (indentation and value.strip() == ""):
        return None
    return value


def _semantic_child(node, label: str):
    if isinstance(node, etree._Comment):
        return ("comment", node.text or "")
    if isinstance(node, etree._ProcessingInstruction):
        return ("processing-instruction", node.target, node.text or "")
    if not isinstance(node.tag, str):
        raise SourceError(f"unsupported XML node in {label}")
    return _semantic_element(node, label)


def _normalize_type_description_set_runs(node, semantic_children):
    """Treat repeated Type/TypeSet/TypeId values as ordered XSD groups of sets.

    8.3.27 orders configuration types by the workspace GeneratedType/TypeId
    index. A standalone writer cannot reproduce that private ordering without
    the complete configuration graph. The values inside each repeated group
    are semantic multisets, while the XSD group order and every qualifier stay
    significant. Sorting only contiguous runs preserves those boundaries and
    therefore does not forgive an invalid Type -> qualifier -> Type layout.
    """
    result = []
    index = 0
    children = list(node)
    while index < len(children):
        child = children[index]
        expanded_name = _expanded_name(child.tag) if isinstance(child.tag, str) else None
        if expanded_name not in TYPE_DESCRIPTION_SET_CHILDREN:
            result.append(semantic_children[index])
            index += 1
            continue
        run_end = index + 1
        while run_end < len(children):
            candidate = children[run_end]
            candidate_name = (
                _expanded_name(candidate.tag) if isinstance(candidate.tag, str) else None
            )
            if candidate_name != expanded_name:
                break
            run_end += 1
        result.extend(
            sorted(
                semantic_children[index:run_end],
                key=lambda item: json.dumps(
                    item, ensure_ascii=False, sort_keys=True, separators=(",", ":")
                ),
            )
        )
        index = run_end
    return tuple(result)


def _semantic_element(node: etree._Element, label: str):
    element_name = _expanded_name(node.tag)
    attributes = []
    for name, value in node.attrib.items():
        expanded_name = _expanded_name(name)
        if expanded_name == f"{{{XSI_NS}}}type":
            semantic_value = (
                "qname",
                _expanded_lexical_qname(value, node, f"{label} xsi:type"),
            )
        else:
            semantic_value = ("text", value)
        attributes.append((expanded_name, semantic_value))
    attributes.sort()

    node_text = _semantic_text(node.text, indentation=len(node) > 0)
    if element_name in QNAME_TEXT_ELEMENTS and node_text is not None:
        text = (
            "qname",
            _expanded_lexical_qname(
                node_text, node, f"{label} {etree.QName(node).localname}"
            ),
        )
    else:
        text = ("text", node_text)

    semantic_children = tuple(
        (_semantic_child(child, label), _semantic_text(child.tail, indentation=True))
        for child in node
    )
    children = _normalize_type_description_set_runs(node, semantic_children)
    return ("element", element_name, tuple(attributes), text, children)


def semantic_xml(payload: bytes, label: str = "XML"):
    """Return a QName-aware tree with only approved lexical differences removed."""
    parser = etree.XMLParser(
        resolve_entities=False,
        load_dtd=False,
        no_network=True,
        remove_blank_text=False,
        strip_cdata=False,
    )
    try:
        document = etree.fromstring(payload, parser=parser)
    except (etree.XMLSyntaxError, ValueError) as error:
        raise SourceError(f"invalid XML {label}: {error}") from error
    if document.getroottree().docinfo.doctype:
        raise SourceError(f"DOCTYPE/entity declarations are forbidden in {label}")
    return _semantic_element(document, label)


def _read_regular_payload(path: Path, metadata, label: str) -> bytes:
    """Read one immutable regular file without following a replaced symlink."""
    flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = None
    try:
        descriptor = os.open(path, flags)
        opened = os.fstat(descriptor)
        expected_identity = (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_mode,
            metadata.st_nlink,
            metadata.st_size,
            metadata.st_mtime_ns,
        )
        opened_identity = (
            opened.st_dev,
            opened.st_ino,
            opened.st_mode,
            opened.st_nlink,
            opened.st_size,
            opened.st_mtime_ns,
        )
        if not stat.S_ISREG(opened.st_mode):
            raise SourceError(f"non-regular file is forbidden in {label}: {path}")
        if opened.st_nlink != 1:
            raise SourceError(
                f"file has a hardlink alias (link count {opened.st_nlink}) in "
                f"{label}: {path}"
            )
        if opened_identity != expected_identity:
            raise SourceError(f"file changed before capture in {label}: {path}")
        chunks = []
        size = 0
        while True:
            block = os.read(descriptor, 1024 * 1024)
            if not block:
                break
            chunks.append(block)
            size += len(block)
        after = os.fstat(descriptor)
        after_identity = (
            after.st_dev,
            after.st_ino,
            after.st_mode,
            after.st_nlink,
            after.st_size,
            after.st_mtime_ns,
        )
        if after_identity != opened_identity or size != opened.st_size:
            raise SourceError(f"file changed while being captured in {label}: {path}")
        return b"".join(chunks)
    except OSError as error:
        raise SourceError(f"cannot capture file in {label} {path}: {error}") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _regular_payloads(
    root: Path,
) -> tuple[dict[str, bytes], dict[str, bytes], list[str]]:
    """Capture every regular file plus empty-directory topology once."""
    if not root.is_dir() or root.is_symlink():
        raise SourceError(f"source is not a safe directory: {root}")
    xml_payloads: dict[str, bytes] = {}
    non_xml_payloads: dict[str, bytes] = {}
    empty_directory_paths: list[str] = []
    identities: dict[tuple[int, int], str] = {}

    def visit(directory: Path) -> None:
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(f"cannot enumerate source {directory}: {error}") from error
        if not entries and directory != root:
            empty_directory_paths.append(directory.relative_to(root).as_posix())
        for entry in entries:
            path = Path(entry.path)
            try:
                if entry.is_symlink():
                    raise SourceError(f"symlink is forbidden in source: {path}")
                if entry.is_dir(follow_symlinks=False):
                    visit(path)
                elif entry.is_file(follow_symlinks=False):
                    metadata = entry.stat(follow_symlinks=False)
                    identity = (metadata.st_dev, metadata.st_ino)
                    relative = path.relative_to(root).as_posix()
                    previous = identities.get(identity)
                    if previous is not None:
                        raise SourceError(
                            "one file identity is exposed through multiple source paths "
                            f"(hardlink replay): {previous}, {relative}"
                        )
                    identities[identity] = relative
                    payload = _read_regular_payload(path, metadata, "source")
                    destination = (
                        xml_payloads
                        if path.suffix.lower() == ".xml"
                        else non_xml_payloads
                    )
                    destination[relative] = payload
                else:
                    raise SourceError(f"special filesystem entry in source: {path}")
            except OSError as error:
                raise SourceError(f"cannot inspect source entry {path}: {error}") from error

    visit(root)
    return xml_payloads, non_xml_payloads, sorted(empty_directory_paths)


def _empty_directory_paths(root: Path) -> list[str]:
    """Return the minimal topology not already implied by regular-file paths."""
    _xml_payloads, _non_xml_payloads, empty_directory_paths = _regular_payloads(root)
    return empty_directory_paths


def _xml_payloads(
    root: Path,
    *,
    allow_empty: bool = False,
    exclude_root_config_dump_info: bool = True,
) -> tuple[dict[str, bytes], list[str]]:
    payloads, _non_xml_payloads, _empty_directory_paths = _regular_payloads(root)
    excluded: list[str] = []
    if exclude_root_config_dump_info and "ConfigDumpInfo.xml" in payloads:
        excluded.append("ConfigDumpInfo.xml")
        del payloads["ConfigDumpInfo.xml"]
    if not payloads and not allow_empty:
        raise SourceError(f"XML source contains no XML except exclusions: {root}")
    return payloads, sorted(excluded)


def semantic_xml_set(root: Path) -> dict[str, tuple]:
    payloads, _excluded = _xml_payloads(Path(root))
    return {
        relative: semantic_xml(payloads[relative], f"{root}/{relative}")
        for relative in sorted(payloads)
    }


def _compare_xml_payloads(
    left_payloads: dict[str, bytes],
    right_payloads: dict[str, bytes],
    left_label: str,
    right_label: str,
    excluded: list[str],
) -> dict:
    left_paths = set(left_payloads)
    right_paths = set(right_payloads)
    shared = sorted(left_paths & right_paths)
    changed = [
        path
        for path in shared
        if semantic_xml(left_payloads[path], f"{left_label}/{path}")
        != semantic_xml(right_payloads[path], f"{right_label}/{path}")
    ]
    added = sorted(right_paths - left_paths)
    removed = sorted(left_paths - right_paths)
    return {
        "equal": not added and not removed and not changed,
        "added": added,
        "removed": removed,
        "changed": changed,
        "excluded": sorted(set(excluded)),
    }


def _hash_xml_payloads(payloads: dict[str, bytes]) -> dict[str, str]:
    return {
        relative: hashlib.sha256(payloads[relative]).hexdigest()
        for relative in sorted(payloads)
    }


def _compare_non_xml_payloads(
    left_payloads: dict[str, bytes],
    right_payloads: dict[str, bytes],
) -> dict:
    left_paths = set(left_payloads)
    right_paths = set(right_payloads)
    shared = sorted(left_paths & right_paths)
    changed = [
        path for path in shared if left_payloads[path] != right_payloads[path]
    ]
    added = sorted(right_paths - left_paths)
    removed = sorted(left_paths - right_paths)
    return {
        "equal": not added and not removed and not changed,
        "added": added,
        "removed": removed,
        "changed": changed,
        "excluded": [],
    }


def uuid_normalized_semantic_sha256(
    payloads: dict[str, bytes], label: str
) -> str:
    """Digest XML semantics while alpha-renaming declared object identities only."""
    uuid_tokens: dict[str, str] = {}

    def is_random_v4_identity(value: str) -> bool:
        parts = value.lower().split("-")
        return (
            len(parts) == 5
            and parts[2].startswith("4")
            and parts[3][:1] in {"8", "9", "a", "b"}
        )

    for relative in sorted(payloads):
        parser = etree.XMLParser(resolve_entities=False, load_dtd=False, no_network=True)
        try:
            root = etree.fromstring(payloads[relative], parser=parser)
        except etree.XMLSyntaxError as error:
            raise CorpusError(f"invalid corpus XML {label}/{relative}: {error}") from error
        if root.getroottree().docinfo.doctype:
            raise CorpusError(f"DOCTYPE is forbidden in corpus XML: {label}/{relative}")
        for element in root.iter():
            if not isinstance(element.tag, str):
                continue
            element_local_name = etree.QName(element).localname
            for attribute_name, attribute_value in element.attrib.items():
                attribute_local_name = etree.QName(attribute_name).localname
                if (
                    attribute_local_name in {"uuid", "id"}
                    and UUID_TEXT_RE.fullmatch(attribute_value) is not None
                    and is_random_v4_identity(attribute_value)
                    and not (
                        element_local_name == "panelDef"
                        and attribute_local_name == "id"
                    )
                ):
                    lexical = attribute_value.lower()
                    uuid_tokens.setdefault(
                        lexical, f"__UNICA_UUID_{len(uuid_tokens) + 1:06d}__"
                    )
            local_name = element_local_name
            text_value = (element.text or "").strip()
            if (
                local_name in {"TypeId", "ValueId", "ObjectId", "ThisNode"}
                and UUID_TEXT_RE.fullmatch(text_value) is not None
                and is_random_v4_identity(text_value)
            ):
                lexical = text_value.lower()
                uuid_tokens.setdefault(
                    lexical, f"__UNICA_UUID_{len(uuid_tokens) + 1:06d}__"
                )

    def normalize_text(value: str) -> str:
        def replacement(match: re.Match) -> str:
            lexical = match.group(0).lower()
            token = uuid_tokens.get(lexical)
            return token if token is not None else match.group(0)

        return UUID_TEXT_RE.sub(replacement, value)

    def normalize(value):
        if isinstance(value, str):
            return normalize_text(value)
        if isinstance(value, tuple):
            return tuple(normalize(item) for item in value)
        if isinstance(value, list):
            return [normalize(item) for item in value]
        return value

    semantic_payload = [
        [
            relative,
            normalize(semantic_xml(payloads[relative], f"{label}/{relative}")),
        ]
        for relative in sorted(payloads)
    ]
    encoded = json.dumps(
        semantic_payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def transition_semantic_sha256(
    pre_payloads: dict[str, bytes], post_payloads: dict[str, bytes], label: str
) -> str:
    """Digest one pre/post transition with a shared UUID identity token map."""
    transition_payloads = {
        **{f"0-pre/{path}": payload for path, payload in pre_payloads.items()},
        **{f"1-post/{path}": payload for path, payload in post_payloads.items()},
    }
    return uuid_normalized_semantic_sha256(transition_payloads, label)


def capture_directory_xml_snapshot(root: Path, *, allow_empty: bool = False) -> dict:
    """Capture all platform input bytes; XML is semantic, non-XML is exact."""
    root = Path(root)
    raw_payloads, raw_non_xml_payloads, empty_directory_paths = _regular_payloads(root)
    excluded = ["ConfigDumpInfo.xml"] if "ConfigDumpInfo.xml" in raw_payloads else []
    comparison_payloads = {
        relative: payload
        for relative, payload in raw_payloads.items()
        if relative not in excluded
    }
    if not comparison_payloads and not allow_empty:
        raise SourceError(f"XML source contains no XML except exclusions: {root}")
    return {
        "label": str(root),
        "rawPayloads": raw_payloads,
        "comparisonPayloads": comparison_payloads,
        "excluded": excluded,
        "rawHashes": _hash_xml_payloads(raw_payloads),
        "rawNonXmlPayloads": raw_non_xml_payloads,
        "comparisonNonXmlPayloads": raw_non_xml_payloads,
        "rawNonXmlHashes": _hash_xml_payloads(raw_non_xml_payloads),
        "emptyDirectoryPaths": empty_directory_paths,
    }


def capture_artifact_xml_snapshot(descriptor: Path, content: Path) -> dict:
    """Capture one artifact descriptor/content pair as one immutable file set."""
    descriptor = Path(descriptor)
    content = Path(content)
    payloads, non_xml_payloads, excluded, empty_directory_paths = _artifact_payloads(
        descriptor, content
    )
    return {
        "label": str(descriptor.with_suffix("")),
        "rawPayloads": payloads,
        "comparisonPayloads": payloads,
        "excluded": excluded,
        "rawHashes": _hash_xml_payloads(payloads),
        "rawNonXmlPayloads": non_xml_payloads,
        "comparisonNonXmlPayloads": non_xml_payloads,
        "rawNonXmlHashes": _hash_xml_payloads(non_xml_payloads),
        "emptyDirectoryPaths": empty_directory_paths,
    }


def compare_xml_snapshots(left: dict, right: dict) -> dict:
    xml_comparison = _compare_xml_payloads(
        left["comparisonPayloads"],
        right["comparisonPayloads"],
        left["label"],
        right["label"],
        left["excluded"] + right["excluded"],
    )
    non_xml_comparison = _compare_non_xml_payloads(
        left.get("comparisonNonXmlPayloads", {}),
        right.get("comparisonNonXmlPayloads", {}),
    )
    left_directories = set(left.get("emptyDirectoryPaths", []))
    right_directories = set(right.get("emptyDirectoryPaths", []))
    directory_comparison = {
        "equal": left_directories == right_directories,
        "added": sorted(right_directories - left_directories),
        "removed": sorted(left_directories - right_directories),
    }
    return {
        "equal": (
            xml_comparison["equal"]
            and non_xml_comparison["equal"]
            and directory_comparison["equal"]
        ),
        "added": sorted(xml_comparison["added"] + non_xml_comparison["added"]),
        "removed": sorted(xml_comparison["removed"] + non_xml_comparison["removed"]),
        "changed": sorted(xml_comparison["changed"] + non_xml_comparison["changed"]),
        "excluded": xml_comparison["excluded"],
        "xml": xml_comparison,
        "nonXml": non_xml_comparison,
        "directories": directory_comparison,
    }


def _snapshot_raw_identity(
    snapshot: dict,
) -> tuple[dict[str, str], dict[str, str], list[str]]:
    return (
        snapshot["rawHashes"],
        snapshot.get("rawNonXmlHashes", {}),
        snapshot.get("emptyDirectoryPaths", []),
    )


def _expected_checkpoint_snapshot_identity(
    item: dict, *, artifact: bool, base: bool = False
) -> tuple[dict[str, str], dict[str, str], list[str]]:
    prefix = "base" if base else "source"
    xml_hashes = item.get(f"{prefix}ExpectedXmlHashes")
    non_xml_hashes = item.get(f"{prefix}ExpectedNonXmlHashes")
    empty_directory_paths = item.get(f"{prefix}ExpectedEmptyDirectoryPaths")
    if (
        not isinstance(xml_hashes, dict)
        or not isinstance(non_xml_hashes, dict)
        or not isinstance(empty_directory_paths, list)
    ):
        raise SourceError(
            f"checkpoint {item.get('id')} is missing its manifest-bound "
            f"{prefix} file hashes or empty-directory paths"
        )
    for label, hashes in (("XML", xml_hashes), ("non-XML", non_xml_hashes)):
        for path, digest in hashes.items():
            if (
                not isinstance(path, str)
                or not path
                or PurePosixPath(path).is_absolute()
                or any(part in {"", ".", ".."} for part in PurePosixPath(path).parts)
                or not isinstance(digest, str)
                or SHA256_RE.fullmatch(digest) is None
            ):
                raise SourceError(
                    f"checkpoint {item.get('id')} has invalid expected "
                    f"{prefix} {label} identity"
                )
    checked_empty_directories = []
    for path in empty_directory_paths:
        if (
            not isinstance(path, str)
            or not path
            or PurePosixPath(path).is_absolute()
            or any(part in {"", ".", ".."} for part in PurePosixPath(path).parts)
        ):
            raise SourceError(
                f"checkpoint {item.get('id')} has invalid expected "
                f"{prefix} empty-directory identity"
            )
        checked_empty_directories.append(path)
    if checked_empty_directories != sorted(set(checked_empty_directories)):
        raise SourceError(
            f"checkpoint {item.get('id')} expected {prefix} empty-directory "
            "paths must be sorted and unique"
        )
    if base or not artifact:
        return (
            dict(sorted(xml_hashes.items())),
            dict(sorted(non_xml_hashes.items())),
            checked_empty_directories,
        )

    owner_paths = item.get("sourceOwnerRelativePaths")
    if not isinstance(owner_paths, list) or len(owner_paths) != 1:
        raise SourceError(
            f"artifact checkpoint {item.get('id')} needs one manifest-bound owner"
        )
    owner_path = owner_paths[0]
    owner = PurePosixPath(owner_path)
    if owner.suffix.lower() != ".xml" or owner_path not in xml_hashes:
        raise SourceError(
            f"artifact checkpoint {item.get('id')} owner is absent from expected XML"
        )
    content_root = owner.with_suffix("")

    def transform(hashes: dict[str, str], *, include_descriptor: bool) -> dict[str, str]:
        result = {}
        for path, digest in sorted(hashes.items()):
            logical = PurePosixPath(path)
            if include_descriptor and logical == owner:
                result["descriptor.xml"] = digest
                continue
            if logical.parts[: len(content_root.parts)] != content_root.parts:
                raise SourceError(
                    f"artifact checkpoint {item.get('id')} expected file is outside "
                    f"its descriptor/content pair: {path}"
                )
            relative_parts = logical.parts[len(content_root.parts) :]
            if not relative_parts:
                raise SourceError(
                    f"artifact checkpoint {item.get('id')} has an invalid content file: "
                    f"{path}"
                )
            result[
                (PurePosixPath("content").joinpath(*relative_parts)).as_posix()
            ] = digest
        return result

    transformed_empty_directories = []
    for path in checked_empty_directories:
        logical = PurePosixPath(path)
        if logical == content_root:
            transformed_empty_directories.append("content")
            continue
        if logical.parts[: len(content_root.parts)] != content_root.parts:
            raise SourceError(
                f"artifact checkpoint {item.get('id')} expected empty directory is "
                f"outside its descriptor/content pair: {path}"
            )
        relative_parts = logical.parts[len(content_root.parts) :]
        transformed_empty_directories.append(
            (PurePosixPath("content").joinpath(*relative_parts)).as_posix()
        )

    return (
        transform(xml_hashes, include_descriptor=True),
        transform(non_xml_hashes, include_descriptor=False),
        sorted(transformed_empty_directories),
    )


def compare_xml_directories(left: Path, right: Path) -> dict:
    left_payloads, left_excluded = _xml_payloads(Path(left))
    right_payloads, right_excluded = _xml_payloads(Path(right))
    return _compare_xml_payloads(
        left_payloads,
        right_payloads,
        str(left),
        str(right),
        left_excluded + right_excluded,
    )


def _artifact_payloads(
    descriptor: Path, content: Path
) -> tuple[dict[str, bytes], dict[str, bytes], list[str], list[str]]:
    descriptor = Path(descriptor)
    content = Path(content)
    if descriptor.is_symlink() or not descriptor.is_file():
        raise SourceError(f"artifact descriptor is not a safe regular XML file: {descriptor}")
    try:
        descriptor_metadata = descriptor.stat(follow_symlinks=False)
    except OSError as error:
        raise SourceError(f"cannot inspect artifact descriptor {descriptor}: {error}") from error
    if descriptor_metadata.st_nlink != 1:
        raise SourceError(
            f"artifact descriptor has a hardlink alias (link count "
            f"{descriptor_metadata.st_nlink}): {descriptor}"
        )
    if descriptor.suffix.lower() != ".xml":
        raise SourceError(f"artifact descriptor must have .xml suffix: {descriptor}")
    descriptor_payload = _read_regular_payload(
        descriptor, descriptor_metadata, "artifact descriptor"
    )
    (
        content_payloads,
        content_non_xml_payloads,
        content_empty_directory_paths,
    ) = _regular_payloads(content)
    payloads = {"descriptor.xml": descriptor_payload}
    payloads.update(
        (f"content/{relative}", payload)
        for relative, payload in content_payloads.items()
    )
    non_xml_payloads = {
        f"content/{relative}": payload
        for relative, payload in content_non_xml_payloads.items()
    }
    empty_directory_paths = [
        f"content/{relative}" for relative in content_empty_directory_paths
    ]
    if (
        not content_payloads
        and not content_non_xml_payloads
        and not content_empty_directory_paths
    ):
        empty_directory_paths.append("content")
    return payloads, non_xml_payloads, [], empty_directory_paths


def _artifact_xml_payloads(
    descriptor: Path, content: Path
) -> tuple[dict[str, bytes], list[str]]:
    payloads, _non_xml_payloads, excluded, _empty_directory_paths = _artifact_payloads(
        descriptor, content
    )
    return payloads, excluded


def compare_artifact_xml_pairs(
    left: tuple[Path, Path], right: tuple[Path, Path]
) -> dict:
    left_payloads, left_excluded = _artifact_xml_payloads(*left)
    right_payloads, right_excluded = _artifact_xml_payloads(*right)
    return _compare_xml_payloads(
        left_payloads,
        right_payloads,
        str(left[0].with_suffix("")),
        str(right[0].with_suffix("")),
        left_excluded + right_excluded,
    )


def artifact_raw_xml_hashes(descriptor: Path, content: Path) -> dict[str, str]:
    payloads, _excluded = _artifact_xml_payloads(descriptor, content)
    return _hash_xml_payloads(payloads)


def _is_relative_to(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def _canonical_relative_path(value, label: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value:
        raise CorpusError(f"{label} must be a canonical relative POSIX path")
    if any(ord(character) < 0x20 for character in value):
        raise CorpusError(f"{label} must not contain C0 control characters")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise CorpusError(f"{label} must be a canonical relative POSIX path: {value!r}")
    if path.as_posix() != value:
        raise CorpusError(f"{label} is not canonical: {value!r}")
    return value


def _relative_path_is_within(relative: str, root_relative: str) -> bool:
    path_parts = PurePosixPath(relative).parts
    root_parts = PurePosixPath(root_relative).parts
    return path_parts[: len(root_parts)] == root_parts


def _safe_existing_path(root: Path, relative: str, label: str, *, directory: bool) -> Path:
    relative = _canonical_relative_path(relative, label)
    candidate = root.joinpath(*PurePosixPath(relative).parts)
    current = root
    for part in PurePosixPath(relative).parts:
        current = current / part
        if current.is_symlink():
            raise CorpusError(f"symlink is forbidden in {label}: {relative}")
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as error:
        raise CorpusError(f"missing or unreadable {label}: {relative}: {error}") from error
    if not _is_relative_to(resolved, root):
        raise CorpusError(f"{label} escapes corpus root: {relative}")
    if directory and not resolved.is_dir():
        raise CorpusError(f"{label} is not a directory: {relative}")
    if not directory and not resolved.is_file():
        raise CorpusError(f"{label} is not a file: {relative}")
    return resolved


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise CorpusError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _parse_json_payload(payload: bytes, label: str) -> dict:
    if len(payload) > 64 * 1024 * 1024:
        raise CorpusError(f"{label} is unreasonably large")
    try:
        value = json.loads(payload, object_pairs_hook=_unique_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CorpusError(f"invalid JSON {label}: {error}") from error
    if not isinstance(value, dict):
        raise CorpusError(f"{label} root must be an object")
    return value


def _read_json_with_payload(path: Path, label: str) -> tuple[dict, bytes]:
    try:
        metadata = os.lstat(path)
    except OSError as error:
        raise CorpusError(f"cannot inspect {label}: {path}: {error}") from error
    try:
        payload = _read_regular_payload(path, metadata, label)
    except SourceError as error:
        raise CorpusError(str(error)) from error
    return _parse_json_payload(payload, label), payload


def _read_json(path: Path, label: str) -> dict:
    value, _payload = _read_json_with_payload(path, label)
    return value


def _require_exact_keys(value: dict, expected: set[str], label: str) -> None:
    actual = set(value)
    unknown = sorted(actual - expected)
    missing = sorted(expected - actual)
    if unknown or missing:
        raise CorpusError(
            f"{label} fields are not exact; missing={missing}, unknown={unknown}"
        )


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as error:
        raise CorpusError(f"cannot hash corpus file {path}: {error}") from error
    return digest.hexdigest()


def _platform_install_root(ibcmd: Path) -> Path:
    ibcmd = Path(ibcmd)
    if not ibcmd.is_absolute():
        raise SourceError("ibcmd must be absolute to derive the platform install root")
    if any(part in {"", ".", ".."} for part in ibcmd.parts[1:]):
        raise SourceError(f"ibcmd path is not canonical and may escape its install root: {ibcmd}")
    if ibcmd.is_symlink():
        raise SourceError(f"ibcmd symlink is forbidden: {ibcmd}")
    try:
        resolved_ibcmd = ibcmd.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"ibcmd is missing or unreadable: {ibcmd}: {error}") from error
    if not resolved_ibcmd.is_file():
        raise SourceError(f"ibcmd is not a regular file: {ibcmd}")
    root = ibcmd.parent
    if root.is_symlink() or not root.is_dir():
        raise SourceError(f"platform install root is not a safe directory: {root}")
    try:
        resolved_root = root.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"platform install root is unreadable: {root}: {error}") from error
    if resolved_ibcmd.parent != resolved_root:
        raise SourceError(
            f"ibcmd path escapes its canonical platform install root: {ibcmd}"
        )
    return resolved_root


def _platform_file_entry(root: Path, path: Path, metadata) -> dict:
    flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = None
    try:
        descriptor = os.open(path, flags)
        opened = os.fstat(descriptor)
        if not stat.S_ISREG(opened.st_mode):
            raise SourceError(f"non-regular platform install entry is forbidden: {path}")
        if opened.st_nlink != 1:
            raise SourceError(f"hardlinked platform install entry is forbidden: {path}")
        expected_identity = (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_mode,
            metadata.st_nlink,
            metadata.st_size,
            metadata.st_mtime_ns,
        )
        opened_identity = (
            opened.st_dev,
            opened.st_ino,
            opened.st_mode,
            opened.st_nlink,
            opened.st_size,
            opened.st_mtime_ns,
        )
        if opened_identity != expected_identity:
            raise SourceError(f"platform install entry changed before hashing: {path}")
        digest = hashlib.sha256()
        size = 0
        while True:
            block = os.read(descriptor, 1024 * 1024)
            if not block:
                break
            digest.update(block)
            size += len(block)
        after = os.fstat(descriptor)
        after_identity = (
            after.st_dev,
            after.st_ino,
            after.st_mode,
            after.st_nlink,
            after.st_size,
            after.st_mtime_ns,
        )
        if after_identity != opened_identity or size != opened.st_size:
            raise SourceError(f"platform install entry changed while hashing: {path}")
    except OSError as error:
        raise SourceError(f"cannot hash platform install entry {path}: {error}") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    relative = path.relative_to(root).as_posix()
    if (
        not relative
        or PurePosixPath(relative).is_absolute()
        or any(part in {"", ".", ".."} for part in PurePosixPath(relative).parts)
    ):
        raise SourceError(f"platform install entry escaped its root: {path}")
    return {
        "path": relative,
        "type": "file",
        "mode": f"{stat.S_IMODE(opened.st_mode):04o}",
        "size": size,
        "sha256": digest.hexdigest(),
    }


def capture_platform_install_inventory(ibcmd: Path) -> dict:
    """Bind path, type, mode, and bytes below the exact ``ibcmd`` parent."""
    root = _platform_install_root(Path(ibcmd))
    files = []
    directories = []
    identities: dict[tuple[int, int], str] = {}

    def visit(directory: Path, relative: str) -> None:
        try:
            before = directory.stat(follow_symlinks=False)
        except OSError as error:
            raise SourceError(
                f"cannot inspect platform install directory {directory}: {error}"
            ) from error
        if not stat.S_ISDIR(before.st_mode):
            raise SourceError(
                f"platform install directory changed type: {directory}"
            )
        before_identity = (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_nlink,
            before.st_mtime_ns,
        )
        directories.append(
            {
                "path": relative,
                "type": "directory",
                "mode": f"{stat.S_IMODE(before.st_mode):04o}",
            }
        )
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(
                f"cannot enumerate platform install directory {directory}: {error}"
            ) from error
        for entry in entries:
            path = Path(entry.path)
            try:
                if entry.is_symlink():
                    raise SourceError(
                        f"symlink is forbidden in platform install inventory: {path}"
                    )
                if entry.is_dir(follow_symlinks=False):
                    metadata = entry.stat(follow_symlinks=False)
                    if not stat.S_ISDIR(metadata.st_mode):
                        raise SourceError(
                            f"platform install directory changed type: {path}"
                        )
                    resolved = path.resolve(strict=True)
                    if resolved != path or not _is_relative_to(resolved, root):
                        raise SourceError(
                            f"platform install directory escaped its root: {path}"
                        )
                    visit(path, path.relative_to(root).as_posix())
                elif entry.is_file(follow_symlinks=False):
                    metadata = entry.stat(follow_symlinks=False)
                    if not stat.S_ISREG(metadata.st_mode):
                        raise SourceError(
                            f"non-regular platform install entry is forbidden: {path}"
                        )
                    resolved = path.resolve(strict=True)
                    if resolved != path or not _is_relative_to(resolved, root):
                        raise SourceError(
                            f"platform install file escaped its root: {path}"
                        )
                    identity = (metadata.st_dev, metadata.st_ino)
                    previous = identities.get(identity)
                    if previous is not None:
                        raise SourceError(
                            "one platform file identity is exposed through multiple paths: "
                            f"{previous}, {path.relative_to(root).as_posix()}"
                        )
                    identities[identity] = path.relative_to(root).as_posix()
                    files.append(_platform_file_entry(root, path, metadata))
                else:
                    raise SourceError(
                        f"non-regular or special platform install entry is forbidden: {path}"
                    )
            except OSError as error:
                raise SourceError(
                    f"cannot inspect platform install entry {path}: {error}"
                ) from error
        try:
            after = directory.stat(follow_symlinks=False)
        except OSError as error:
            raise SourceError(
                f"cannot re-inspect platform install directory {directory}: {error}"
            ) from error
        after_identity = (
            after.st_dev,
            after.st_ino,
            after.st_mode,
            after.st_nlink,
            after.st_mtime_ns,
        )
        if after_identity != before_identity:
            raise SourceError(
                f"platform install directory changed while inventory was captured: "
                f"{directory}"
            )

    visit(root, ".")
    files.sort(key=lambda item: item["path"])
    directories.sort(key=lambda item: item["path"])
    payload = json.dumps(
        {"directories": directories, "files": files},
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return {
        "root": root,
        "fileCount": len(files),
        "directoryCount": len(directories),
        "sha256": hashlib.sha256(payload).hexdigest(),
        "files": files,
        "directories": directories,
    }


def verify_platform_install_inventory(
    ibcmd: Path,
    *,
    expected_sha256: str | None = None,
    expected_file_count: int | None = None,
) -> dict:
    expected_sha256 = (
        EXPECTED_PLATFORM_INSTALL_SHA256
        if expected_sha256 is None
        else expected_sha256
    )
    expected_file_count = (
        EXPECTED_PLATFORM_INSTALL_FILE_COUNT
        if expected_file_count is None
        else expected_file_count
    )
    if not isinstance(expected_sha256, str) or SHA256_RE.fullmatch(expected_sha256) is None:
        raise SourceError(
            "expected platform install inventory SHA-256 must be lowercase hexadecimal"
        )
    if (
        type(expected_file_count) is not int
        or expected_file_count < 1
    ):
        raise SourceError("expected platform install file count must be a positive integer")
    inventory = capture_platform_install_inventory(Path(ibcmd))
    if (
        inventory["sha256"] != expected_sha256
        or inventory["fileCount"] != expected_file_count
    ):
        raise PlatformInstallError(
            "platform install inventory does not match the pinned 8.3.27.2074 "
            f"closure; expected sha256={expected_sha256}, files={expected_file_count}; "
            f"observed sha256={inventory['sha256']}, files={inventory['fileCount']}",
            inventory=inventory,
        )
    return inventory


def _snapshot_xml_hashes(root: Path) -> dict[str, str]:
    if not root.is_dir():
        raise CorpusError(f"workspace is not a directory: {root}")
    result: dict[str, str] = {}

    def visit(directory: Path) -> None:
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise CorpusError(f"cannot enumerate workspace {directory}: {error}") from error
        for entry in entries:
            path = Path(entry.path)
            if entry.is_symlink():
                raise CorpusError(f"symlink is forbidden in workspace: {path}")
            if entry.is_dir(follow_symlinks=False):
                visit(path)
            elif entry.is_file(follow_symlinks=False) and path.suffix.lower() == ".xml":
                try:
                    metadata = entry.stat(follow_symlinks=False)
                    payload = _read_regular_payload(path, metadata, "workspace XML snapshot")
                except OSError as error:
                    raise CorpusError(
                        f"cannot inspect workspace XML {path}: {error}"
                    ) from error
                result[path.relative_to(root).as_posix()] = hashlib.sha256(
                    payload
                ).hexdigest()

    visit(root)
    if not result:
        raise CorpusError(f"workspace has no XML: {root}")
    return result


def _string_list(value, label: str, *, paths: bool = False) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise CorpusError(f"{label} must be a string list")
    checked = [
        _canonical_relative_path(item, f"{label} entry") if paths else item for item in value
    ]
    if checked != sorted(set(checked)):
        raise CorpusError(f"{label} must be sorted and unique")
    return checked


def _hash_map(value, label: str) -> dict[str, str]:
    if not isinstance(value, dict):
        raise CorpusError(f"{label} must be an object")
    result = {}
    for raw_path, digest in value.items():
        path = _canonical_relative_path(raw_path, f"{label} path")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(f"invalid SHA-256 hash in {label}: {raw_path}")
        result[path] = digest
    if list(result) != sorted(result):
        raise CorpusError(f"{label} keys must be sorted")
    return result


def _removed_paths(value, label: str, before: dict[str, str]) -> list[str]:
    if not isinstance(value, list):
        raise CorpusError(f"{label} must be a list")
    paths = []
    for item in value:
        if not isinstance(item, dict) or set(item) != {"path", "sha256"}:
            raise CorpusError(f"{label} entries must contain exactly path and sha256")
        path = _canonical_relative_path(item.get("path"), f"{label} path")
        digest = item.get("sha256")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(f"invalid removed SHA-256 hash in {label}: {path}")
        if before.get(path) != digest:
            raise CorpusError(f"removed hash in {label} does not match pre hash: {path}")
        paths.append(path)
    if paths != sorted(set(paths)):
        raise CorpusError(f"{label} paths must be sorted and unique")
    return paths


def _classify_delta(before: dict[str, str], after: dict[str, str]) -> dict[str, list[str]]:
    before_paths = set(before)
    after_paths = set(after)
    shared = before_paths & after_paths
    return {
        "created": sorted(after_paths - before_paths),
        "modified": sorted(path for path in shared if before[path] != after[path]),
        "removed": sorted(before_paths - after_paths),
        "unchanged": sorted(path for path in shared if before[path] == after[path]),
    }


def _validate_non_xml_contract(
    case: dict,
    report: dict,
    case_id: str,
    workspace: Path,
    workspace_rel: str,
    source: Path,
    base_source: Path | None,
    input_relative_roots: list[str],
) -> dict:
    """Validate the exact non-XML pre/post byte contract for platform inputs."""
    for field in ("preNonXmlFiles", "nonXmlFiles", "removedNonXmlPaths"):
        if report.get(field) != case.get(field):
            raise CorpusError(
                f"case {case_id} manifest/report mismatch for {field}"
            )

    prefix = f"{workspace_rel}/"
    pre_prefix = f"cases/{case_id}/pre-non-xml/"

    def validate_path(raw_path, label: str) -> tuple[str, str]:
        corpus_path = _canonical_relative_path(raw_path, label)
        if not corpus_path.startswith(prefix):
            raise CorpusError(f"{label} is outside workspacePath")
        workspace_path = corpus_path[len(prefix) :]
        if not workspace_path:
            raise CorpusError(f"{label} has no workspace-relative path")
        if PurePosixPath(workspace_path).suffix.lower() == ".xml":
            raise CorpusError(f"{label} must identify a non-XML file")
        if not any(
            _relative_path_is_within(workspace_path, input_root)
            for input_root in input_relative_roots
        ):
            raise CorpusError(
                f"{label} is outside every platform checkpoint input boundary"
            )
        return corpus_path, workspace_path

    def validate_pre_path(raw_path, label: str) -> tuple[str, str]:
        corpus_path = _canonical_relative_path(raw_path, label)
        if not corpus_path.startswith(pre_prefix):
            raise CorpusError(
                f"{label} is outside the canonical pre-non-xml snapshot"
            )
        workspace_path = corpus_path[len(pre_prefix) :]
        if not workspace_path:
            raise CorpusError(f"{label} has no workspace-relative path")
        if PurePosixPath(workspace_path).suffix.lower() == ".xml":
            raise CorpusError(f"{label} must identify a non-XML file")
        if not any(
            _relative_path_is_within(workspace_path, input_root)
            for input_root in input_relative_roots
        ):
            raise CorpusError(
                f"{label} is outside every platform checkpoint input boundary"
            )
        return corpus_path, workspace_path

    pre_non_xml_root = _safe_existing_path(
        workspace.parent.parent.parent,
        f"cases/{case_id}/pre-non-xml",
        f"case {case_id} pre non-XML snapshot",
        directory=True,
    )
    (
        pre_xml_payloads,
        pre_non_xml_payloads,
        _pre_empty_directory_paths,
    ) = _regular_payloads(pre_non_xml_root)
    if pre_xml_payloads:
        raise CorpusError(
            f"case {case_id} pre non-XML snapshot contains XML files"
        )
    actual_before = _hash_xml_payloads(pre_non_xml_payloads)

    raw_pre_files = case.get("preNonXmlFiles")
    if not isinstance(raw_pre_files, list):
        raise CorpusError(f"case {case_id} preNonXmlFiles must be a list")
    before: dict[str, str] = {}
    pre_paths = []
    for entry in raw_pre_files:
        if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
            raise CorpusError(
                f"case {case_id} preNonXmlFiles entries must contain exactly "
                "path and sha256"
            )
        corpus_path, workspace_path = validate_pre_path(
            entry.get("path"), f"case {case_id} preNonXmlFiles path"
        )
        digest = entry.get("sha256")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(
                f"case {case_id} preNonXmlFiles hash is invalid: {corpus_path}"
            )
        if workspace_path in before:
            raise CorpusError(f"case {case_id} has duplicate preNonXmlFiles path")
        before[workspace_path] = digest
        pre_paths.append(corpus_path)
    if pre_paths != sorted(set(pre_paths)):
        raise CorpusError(
            f"case {case_id} preNonXmlFiles must be sorted and unique"
        )
    if before != actual_before:
        raise CorpusError(
            f"case {case_id} preNonXmlFiles inventory/hash does not exactly match "
            "the materialized pre non-XML snapshot"
        )

    actual_after: dict[str, str] = {}
    for platform_root in (source, base_source):
        if platform_root is None:
            continue
        (
            _xml_payloads_after,
            non_xml_payloads,
            _platform_empty_directory_paths,
        ) = _regular_payloads(platform_root)
        root_relative = platform_root.relative_to(workspace).as_posix()
        for relative, payload in non_xml_payloads.items():
            workspace_path = (PurePosixPath(root_relative) / relative).as_posix()
            if workspace_path in actual_after:
                raise CorpusError(
                    f"case {case_id} platform input roots overlap at {workspace_path}"
                )
            actual_after[workspace_path] = hashlib.sha256(payload).hexdigest()

    raw_post_files = case.get("nonXmlFiles")
    if not isinstance(raw_post_files, list):
        raise CorpusError(f"case {case_id} nonXmlFiles must be a list")
    after: dict[str, str] = {}
    post_paths = []
    for entry in raw_post_files:
        if not isinstance(entry, dict) or set(entry) != {
            "path",
            "sha256",
            "seed",
            "delta",
        }:
            raise CorpusError(
                f"case {case_id} nonXmlFiles entries must contain exactly "
                "path, sha256, seed, and delta"
            )
        corpus_path, workspace_path = validate_path(
            entry.get("path"), f"case {case_id} nonXmlFiles path"
        )
        digest = entry.get("sha256")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(
                f"case {case_id} nonXmlFiles hash is invalid: {corpus_path}"
            )
        if workspace_path in after:
            raise CorpusError(f"case {case_id} has duplicate nonXmlFiles path")
        after[workspace_path] = digest
        post_paths.append(corpus_path)
        expected_delta = (
            "created"
            if workspace_path not in before
            else (
                "unchanged"
                if before[workspace_path] == digest
                else "modified"
            )
        )
        if (
            type(entry.get("seed")) is not bool
            or entry["seed"] != (workspace_path in before)
            or entry.get("delta") != expected_delta
        ):
            raise CorpusError(
                f"case {case_id} nonXmlFiles seed/delta metadata mismatch: "
                f"{corpus_path}"
            )
    if post_paths != sorted(set(post_paths)):
        raise CorpusError(f"case {case_id} nonXmlFiles must be sorted and unique")
    if after != actual_after:
        raise CorpusError(
            f"case {case_id} nonXmlFiles inventory/hash does not exactly match "
            "the platform checkpoint inputs"
        )

    removed_paths = _string_list(
        case.get("removedNonXmlPaths"),
        f"case {case_id} removedNonXmlPaths",
        paths=True,
    )
    normalized_removed = []
    for corpus_path in removed_paths:
        canonical_path, workspace_path = validate_path(
            corpus_path, f"case {case_id} removedNonXmlPaths path"
        )
        if workspace_path not in before or workspace_path in after:
            raise CorpusError(
                f"case {case_id} removedNonXmlPaths does not identify a removed "
                f"pre-state file: {canonical_path}"
            )
        normalized_removed.append(workspace_path)
    delta = _classify_delta(before, after)
    if normalized_removed != delta["removed"]:
        raise CorpusError(
            f"case {case_id} removedNonXmlPaths do not match non-XML pre/post hashes"
        )
    return {
        "before": before,
        "after": after,
        "delta": delta,
        "preFiles": raw_pre_files,
        "files": raw_post_files,
        "removedPaths": removed_paths,
    }


def _validate_auxiliary_files(
    case: dict,
    report: dict,
    case_id: str,
    workspace: Path,
    workspace_rel: str,
    input_relative_roots: list[str],
) -> dict[str, str]:
    """Bind every stable workspace file outside the platform input boundaries."""
    raw_files = case.get("auxiliaryFiles")
    if report.get("auxiliaryFiles") != raw_files:
        raise CorpusError(
            f"case {case_id} manifest/report mismatch for auxiliaryFiles"
        )
    if not isinstance(raw_files, list):
        raise CorpusError(f"case {case_id} auxiliaryFiles must be a list")

    prefix = f"{workspace_rel}/"
    declared: dict[str, str] = {}
    corpus_paths = []
    for entry in raw_files:
        if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
            raise CorpusError(
                f"case {case_id} auxiliaryFiles entries must contain exactly "
                "path and sha256"
            )
        corpus_path = _canonical_relative_path(
            entry.get("path"), f"case {case_id} auxiliaryFiles path"
        )
        if not corpus_path.startswith(prefix):
            raise CorpusError(
                f"case {case_id} auxiliaryFiles path is outside workspacePath"
            )
        workspace_path = corpus_path[len(prefix) :]
        if not workspace_path:
            raise CorpusError(
                f"case {case_id} auxiliaryFiles path has no workspace-relative path"
            )
        if PurePosixPath(workspace_path).suffix.lower() == ".xml":
            raise CorpusError(
                f"case {case_id} auxiliaryFiles must identify non-XML files"
            )
        if any(
            _relative_path_is_within(workspace_path, input_root)
            for input_root in input_relative_roots
        ):
            raise CorpusError(
                f"case {case_id} auxiliaryFiles overlaps a platform checkpoint "
                f"input boundary: {workspace_path}"
            )
        digest = entry.get("sha256")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(
                f"case {case_id} auxiliaryFiles hash is invalid: {corpus_path}"
            )
        if workspace_path in declared:
            raise CorpusError(
                f"case {case_id} has duplicate auxiliaryFiles path"
            )
        declared[workspace_path] = digest
        corpus_paths.append(corpus_path)
    if corpus_paths != sorted(set(corpus_paths)):
        raise CorpusError(
            f"case {case_id} auxiliaryFiles must be sorted and unique"
        )

    (
        _workspace_xml,
        workspace_non_xml,
        _workspace_empty_directory_paths,
    ) = _regular_payloads(workspace)
    actual = {
        path: hashlib.sha256(payload).hexdigest()
        for path, payload in workspace_non_xml.items()
        if not any(
            _relative_path_is_within(path, input_root)
            for input_root in input_relative_roots
        )
    }
    if declared != actual:
        raise CorpusError(
            f"case {case_id} auxiliaryFiles inventory/hash does not exactly match "
            "the workspace files outside platform checkpoint boundaries"
        )
    return declared


def case_contract_sha256(
    cases: list[dict],
    normalized_cases: list[dict],
    empty_directory_paths: list[str] | None = None,
) -> str:
    """Bind exact public calls and their expected XML plus non-XML transitions."""
    normalized_by_id = {case["id"]: case for case in normalized_cases}
    contract = []
    for case in cases:
        case_id = case.get("id")
        normalized = normalized_by_id.get(case_id)
        public_arguments = (
            normalized.get("report", {}).get("publicArguments")
            if isinstance(normalized, dict)
            else None
        )
        files = case.get("files") if isinstance(case.get("files"), list) else []
        file_contract = sorted(
            (
                {
                    "path": item.get("path"),
                    "family": item.get("family"),
                    "seed": item.get("seed"),
                    "delta": item.get("delta"),
                    "ownerPath": item.get("ownerPath"),
                    "newStandalone": item.get("newStandalone"),
                }
                for item in files
                if isinstance(item, dict)
            ),
            key=lambda item: str(item["path"]),
        )
        owner_versions = case.get("ownerVersions")
        contract.append(
            {
                "id": case_id,
                "workspacePath": case.get("workspacePath"),
                "preSnapshotPath": case.get("preSnapshotPath"),
                "checkpoint": case.get("checkpoint"),
                "platformCheckpoint": case.get("platformCheckpoint"),
                "toolId": case.get("toolId"),
                "operation": case.get("operation"),
                "branch": case.get("branch"),
                "impactClass": case.get("impactClass"),
                "xmlImpact": case.get("xmlImpact"),
                "publicArguments": public_arguments,
                "files": file_contract,
                "preSignature": (
                    normalized.get("preSignature")
                    if isinstance(normalized, dict)
                    else None
                ),
                "preSemanticSha256": (
                    normalized.get("preSemanticSha256")
                    if isinstance(normalized, dict)
                    else None
                ),
                "transitionSemanticSha256": (
                    normalized.get("transitionSemanticSha256")
                    if isinstance(normalized, dict)
                    else None
                ),
                "postSemanticSha256": (
                    normalized.get("postSemanticSha256")
                    if isinstance(normalized, dict)
                    else None
                ),
                "preNonXmlFiles": case.get("preNonXmlFiles"),
                "nonXmlFiles": case.get("nonXmlFiles"),
                "removedNonXmlPaths": case.get("removedNonXmlPaths"),
                "auxiliaryFiles": case.get("auxiliaryFiles"),
                "removedPaths": case.get("removedPaths"),
                "preOwnerPaths": (
                    normalized.get("preOwnerPaths")
                    if isinstance(normalized, dict)
                    else None
                ),
                "ownerPaths": (
                    sorted(owner_versions)
                    if isinstance(owner_versions, dict)
                    else None
                ),
            }
        )
    payload = json.dumps(
        {
            "cases": contract,
            "emptyDirectoryPaths": (
                [] if empty_directory_paths is None else empty_directory_paths
            ),
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _validate_checkpoint(checkpoint, case_id: str) -> dict:
    if not isinstance(checkpoint, dict):
        raise CorpusError(f"case {case_id} platformCheckpoint must be an object")
    kind = checkpoint.get("kind")
    if kind not in CHECKPOINT_KINDS:
        raise CorpusError(f"case {case_id} has invalid checkpoint kind")
    required = {"kind", "coveredCaseIds", "sourcePath"}
    if kind == "extension":
        required.add("baseSourcePath")
    unknown = set(checkpoint) - required
    missing = required - set(checkpoint)
    if unknown:
        raise CorpusError(f"case {case_id} has unknown checkpoint fields: {sorted(unknown)}")
    if missing:
        raise CorpusError(f"case {case_id} checkpoint is missing {sorted(missing)}")
    covered = _string_list(checkpoint.get("coveredCaseIds"), f"case {case_id} coveredCaseIds")
    normalized = {"kind": kind, "coveredCaseIds": covered}
    normalized["sourcePath"] = _canonical_relative_path(
        checkpoint.get("sourcePath"), f"case {case_id} sourcePath"
    )
    if kind == "extension":
        normalized["baseSourcePath"] = _canonical_relative_path(
            checkpoint.get("baseSourcePath"), f"case {case_id} baseSourcePath"
        )
    return normalized


def _parse_root_details_payload(
    payload: bytes, label: str
) -> tuple[str | None, str | None, str]:
    parser = etree.XMLParser(resolve_entities=False, load_dtd=False, no_network=True)
    try:
        root = etree.fromstring(payload, parser=parser)
    except etree.XMLSyntaxError as error:
        raise CorpusError(f"invalid corpus XML {label}: {error}") from error
    if root.getroottree().docinfo.doctype:
        raise CorpusError(f"DOCTYPE is forbidden in corpus XML: {label}")
    try:
        version = raw_root_attribute(payload, "version")
    except LexicalXmlError as error:
        raise CorpusError(
            f"cannot inspect raw root version in corpus XML {label}: {error}"
        ) from error
    first_element = next((child for child in root if isinstance(child.tag, str)), None)
    owner_type = etree.QName(first_element).localname if first_element is not None else None
    return version, owner_type, _expanded_name(root.tag)


def _parse_root_contract_payload(
    payload: bytes, label: str
) -> tuple[str | None, str | None]:
    version, owner_type, _root_qname = _parse_root_details_payload(payload, label)
    return version, owner_type


def _parse_root_contract(path: Path) -> tuple[str | None, str | None]:
    try:
        payload = Path(path).read_bytes()
    except OSError as error:
        raise CorpusError(f"cannot read corpus XML {path}: {error}") from error
    return _parse_root_contract_payload(payload, str(path))


def _validate_pre_snapshot(
    case: dict,
    case_id: str,
    pre_snapshot_rel: str,
    pre_snapshot: dict,
    checkpoint_kind: str,
    input_relative_roots: list[str],
) -> dict:
    raw_payloads = pre_snapshot["rawPayloads"]
    raw_hashes = pre_snapshot["rawHashes"]
    prefix = f"{pre_snapshot_rel}/"
    files = case.get("preFiles")
    if not isinstance(files, list):
        raise CorpusError(f"case {case_id} preFiles must be a list")

    declared_paths = []
    payloads: dict[str, bytes] = {}
    owner_links: dict[str, str] = {}
    pre_signature = []
    root_contracts = {}
    for item in files:
        if not isinstance(item, dict):
            raise CorpusError(f"case {case_id} preFiles entry must be an object")
        required_fields = {"path", "sha256", "family"}
        allowed_fields = required_fields | {"ownerPath"}
        if set(item) - allowed_fields or not required_fields.issubset(item):
            raise CorpusError(
                f"case {case_id} preFiles entry has invalid shape: "
                f"{sorted(item)}"
            )
        corpus_path = _canonical_relative_path(
            item.get("path"), f"case {case_id} preFiles path"
        )
        if not corpus_path.startswith(prefix):
            raise CorpusError(
                f"case {case_id} preFiles path is outside preSnapshotPath"
            )
        logical_path = corpus_path[len(prefix) :]
        if not logical_path:
            raise CorpusError(f"case {case_id} preFiles path has no logical XML path")
        if corpus_path in declared_paths:
            raise CorpusError(f"case {case_id} has duplicate preFiles path")
        declared_paths.append(corpus_path)
        payload = raw_payloads.get(logical_path)
        if payload is None:
            raise CorpusError(
                f"case {case_id} preFiles entry is absent from immutable pre-snapshot: "
                f"{corpus_path}"
            )
        digest = item.get("sha256")
        if (
            not isinstance(digest, str)
            or SHA256_RE.fullmatch(digest) is None
            or digest != raw_hashes[logical_path]
        ):
            raise CorpusError(f"case {case_id} preFiles hash mismatch: {corpus_path}")
        version, owner_type, root_qname = _parse_root_details_payload(
            payload, corpus_path
        )
        expected_family = XML_FAMILY_BY_ROOT_QNAME.get(root_qname)
        if expected_family is None or item.get("family") != expected_family:
            raise CorpusError(
                f"case {case_id} preFiles family does not match XML root "
                f"{root_qname}: expected {expected_family!r}"
            )
        owner_path = item.get("ownerPath")
        if owner_path is not None:
            owner_path = _canonical_relative_path(
                owner_path, f"case {case_id} preFiles ownerPath"
            )
            if not owner_path.startswith(prefix):
                raise CorpusError(
                    f"case {case_id} preFiles ownerPath is outside preSnapshotPath"
                )
            owner_links[corpus_path] = owner_path
        payloads[logical_path] = payload
        root_contracts[logical_path] = (version, owner_type, root_qname)
        pre_signature.append(
            {
                "path": logical_path,
                "family": expected_family,
                "ownerPath": (
                    owner_path[len(prefix) :] if owner_path is not None else None
                ),
            }
        )

    if declared_paths != sorted(set(declared_paths)):
        raise CorpusError(f"case {case_id} preFiles must be sorted and unique")
    actual_paths = sorted(raw_payloads)
    declared_logical_paths = [path[len(prefix) :] for path in declared_paths]
    if declared_logical_paths != actual_paths:
        raise CorpusError(
            f"case {case_id} preFiles inventory is not exact; "
            f"declared={declared_logical_paths}, actual={actual_paths}"
        )

    raw_pre_owner_versions = case.get("preOwnerVersions")
    if not isinstance(raw_pre_owner_versions, dict):
        raise CorpusError(f"case {case_id} preOwnerVersions must be an object")
    pre_owner_versions = {}
    for raw_owner_path, declared_version in raw_pre_owner_versions.items():
        owner_path = _canonical_relative_path(
            raw_owner_path, f"case {case_id} preOwnerVersions path"
        )
        if not owner_path.startswith(prefix):
            raise CorpusError(
                f"case {case_id} preOwnerVersions path is outside preSnapshotPath"
            )
        if declared_version != EXPECTED_EXPORT_VERSION:
            raise CorpusError(f"case {case_id} preOwnerVersions value must be 2.20")
        pre_owner_versions[owner_path] = declared_version
    if list(raw_pre_owner_versions) != sorted(raw_pre_owner_versions):
        raise CorpusError(f"case {case_id} preOwnerVersions must be sorted")

    source_set_owner_types = {
        "Configuration",
        "ConfigurationExtension",
        "ExternalDataProcessor",
        "ExternalReport",
    }
    detected_owner_versions = {}
    owner_roots = {}
    owner_types = {}
    for logical_path, (version, owner_type, root_qname) in root_contracts.items():
        corpus_path = f"{prefix}{logical_path}"
        if version is not None and version != EXPECTED_EXPORT_VERSION:
            raise CorpusError(
                f"case {case_id} pre-snapshot version-bearing root is not 2.20: "
                f"{corpus_path}"
            )
        if (
            root_qname == f"{{{MD_CLASSES_NS}}}MetaDataObject"
            and owner_type in source_set_owner_types
        ):
            if version != EXPECTED_EXPORT_VERSION:
                raise CorpusError(
                    f"case {case_id} pre-snapshot source-set owner is not 2.20: "
                    f"{corpus_path}"
                )
            detected_owner_versions[corpus_path] = EXPECTED_EXPORT_VERSION
            owner_roots[corpus_path] = PurePosixPath(logical_path).parent
            owner_types[corpus_path] = owner_type
    if pre_owner_versions != detected_owner_versions:
        raise CorpusError(
            f"case {case_id} preOwnerVersions do not exactly identify source-set owners"
        )

    owner_paths = set(pre_owner_versions)
    for logical_path, (version, _owner_type, _root_qname) in root_contracts.items():
        corpus_path = f"{prefix}{logical_path}"
        linked_owner = owner_links.get(corpus_path)
        if version is not None:
            if linked_owner is not None:
                raise CorpusError(
                    f"case {case_id} version-bearing pre-snapshot XML must not "
                    f"declare ownerPath: {corpus_path}"
                )
            continue
        if linked_owner is None:
            raise CorpusError(
                f"case {case_id} versionless pre-snapshot XML needs ownerPath: "
                f"{corpus_path}"
            )
        if linked_owner not in owner_paths:
            raise CorpusError(
                f"case {case_id} pre-snapshot ownerPath is not a same-snapshot owner"
            )
        containing_owners = [
            (owner_path, owner_root)
            for owner_path, owner_root in owner_roots.items()
            if _relative_path_is_within(logical_path, owner_root.as_posix())
        ]
        if not containing_owners:
            raise CorpusError(
                f"case {case_id} versionless pre-snapshot XML has no physically "
                f"containing owner root: {corpus_path}"
            )
        deepest = max(len(owner_root.parts) for _, owner_root in containing_owners)
        deepest_owners = sorted(
            owner_path
            for owner_path, owner_root in containing_owners
            if len(owner_root.parts) == deepest
        )
        if len(deepest_owners) != 1 or linked_owner != deepest_owners[0]:
            raise CorpusError(
                f"case {case_id} pre-snapshot ownerPath must name the unique deepest "
                f"containing owner root: {corpus_path}"
            )

    if input_relative_roots:
        outside_inputs = sorted(
            path
            for path in raw_payloads
            if not any(
                _relative_path_is_within(path, input_root)
                for input_root in input_relative_roots
            )
        )
        if outside_inputs:
            raise CorpusError(
                f"case {case_id} pre-snapshot XML is outside every platform "
                f"checkpoint input/sourcePath or baseSourcePath boundary: "
                f"{outside_inputs}"
            )
        forbidden_dump_info = {
            (PurePosixPath(input_root) / "ConfigDumpInfo.xml").as_posix()
            for input_root in input_relative_roots
        } & set(raw_payloads)
        if forbidden_dump_info:
            raise CorpusError(
                f"case {case_id} pre-snapshot must not contain writer-owned "
                f"ConfigDumpInfo.xml: {sorted(forbidden_dump_info)}"
            )
        owner_logical_paths = {
            owner_path[len(prefix) :] for owner_path in owner_roots
        }
        expected_boundary_owner_types = EXPECTED_OWNER_TYPES[checkpoint_kind]
        if len(expected_boundary_owner_types) != len(input_relative_roots):
            raise CorpusError(
                f"case {case_id} pre-snapshot source/base boundary shape is invalid"
            )
        for boundary_index, input_root in enumerate(input_relative_roots):
            files_in_root = [
                path
                for path in raw_payloads
                if _relative_path_is_within(path, input_root)
            ]
            owners_in_root = [
                path
                for path in owner_logical_paths
                if _relative_path_is_within(path, input_root)
            ]
            if files_in_root and len(owners_in_root) != 1:
                raise CorpusError(
                    f"case {case_id} populated pre-snapshot source/base boundary "
                    f"{input_root!r} must contain exactly one owner"
                )
            if files_in_root:
                owner_corpus_path = f"{prefix}{owners_in_root[0]}"
                expected_owner_type = expected_boundary_owner_types[boundary_index]
                if owner_types[owner_corpus_path] != expected_owner_type:
                    raise CorpusError(
                        f"case {case_id} pre-snapshot boundary {input_root!r} "
                        f"owner type {owner_types[owner_corpus_path]!r} != "
                        f"{expected_owner_type!r}"
                    )

    return {
        "hashes": dict(raw_hashes),
        "payloads": payloads,
        "ownerLinks": owner_links,
        "ownerVersions": pre_owner_versions,
        "signature": pre_signature,
        "semanticSha256": uuid_normalized_semantic_sha256(
            payloads, f"case {case_id} pre XML"
        ),
    }


def _validate_case(root: Path, case: dict, known_case_ids: set[str]) -> dict:
    _require_exact_keys(
        case,
        {
            "id",
            "workspacePath",
            "preSnapshotPath",
            "platformCheckpoint",
            "checkpoint",
            "toolId",
            "operation",
            "branch",
            "impactClass",
            "xmlImpact",
            "preFiles",
            "files",
            "removedPaths",
            "preNonXmlFiles",
            "nonXmlFiles",
            "removedNonXmlPaths",
            "auxiliaryFiles",
            "preOwnerVersions",
            "ownerVersions",
        },
        "corpus case",
    )
    case_id = case.get("id")
    if not isinstance(case_id, str) or CASE_ID_RE.fullmatch(case_id) is None:
        raise CorpusError("corpus case id must be one safe kebab-case filename component")
    workspace_rel = _canonical_relative_path(
        case.get("workspacePath"), f"case {case_id} workspacePath"
    )
    expected_workspace_rel = f"cases/{case_id}/workspace"
    if workspace_rel != expected_workspace_rel:
        raise CorpusError(
            f"case {case_id} must use its dedicated canonical workspacePath "
            f"{expected_workspace_rel}"
        )
    workspace = _safe_existing_path(
        root, workspace_rel, f"case {case_id} workspacePath", directory=True
    )
    pre_snapshot_rel = _canonical_relative_path(
        case.get("preSnapshotPath"), f"case {case_id} preSnapshotPath"
    )
    expected_pre_snapshot_rel = f"cases/{case_id}/pre-xml"
    if pre_snapshot_rel != expected_pre_snapshot_rel:
        raise CorpusError(
            f"case {case_id} must use its dedicated canonical preSnapshotPath "
            f"{expected_pre_snapshot_rel}"
        )
    pre_snapshot_root = _safe_existing_path(
        root,
        pre_snapshot_rel,
        f"case {case_id} preSnapshotPath",
        directory=True,
    )
    pre_snapshot = capture_directory_xml_snapshot(
        pre_snapshot_root, allow_empty=True
    )
    checkpoint = _validate_checkpoint(case.get("platformCheckpoint"), case_id)
    if checkpoint["coveredCaseIds"] != [case_id]:
        raise CorpusError(
            f"case {case_id} checkpoint coveredCaseIds must cover only itself"
        )
    if any(covered not in known_case_ids for covered in checkpoint["coveredCaseIds"]):
        raise CorpusError(f"case {case_id} checkpoint covers an unknown case")
    impact_class = case.get("impactClass")
    if impact_class not in {"None", "CreateOrModify", "RemoveOrModify"}:
        raise CorpusError(f"case {case_id} has invalid impactClass")

    source_owners: list[Path] = []
    source = _safe_existing_path(
        root, checkpoint["sourcePath"], f"case {case_id} sourcePath", directory=True
    )
    if not _is_relative_to(source, workspace):
        raise CorpusError(f"case {case_id} sourcePath is outside workspacePath")
    semantic_xml_set(source)
    base_source = None
    if checkpoint["kind"] == "extension":
        base_source = _safe_existing_path(
            root,
            checkpoint["baseSourcePath"],
            f"case {case_id} baseSourcePath",
            directory=True,
        )
        if not _is_relative_to(base_source, workspace):
            raise CorpusError(f"case {case_id} baseSourcePath is outside workspacePath")
        semantic_xml_set(base_source)
        if _is_relative_to(base_source, source) or _is_relative_to(source, base_source):
            raise CorpusError(
                f"case {case_id} extension source and base source overlap"
            )

    input_relative_roots = [source.relative_to(workspace).as_posix()]
    if base_source is not None:
        input_relative_roots.append(base_source.relative_to(workspace).as_posix())
    pre_contract = _validate_pre_snapshot(
        case,
        case_id,
        pre_snapshot_rel,
        pre_snapshot,
        checkpoint["kind"],
        input_relative_roots,
    )

    checkpoint_rel = _canonical_relative_path(
        case.get("checkpoint"), f"case {case_id} checkpoint report path"
    )
    expected_checkpoint_rel = f"cases/{case_id}/case-report.json"
    if checkpoint_rel != expected_checkpoint_rel:
        raise CorpusError(
            f"case {case_id} must use its dedicated canonical checkpoint "
            f"{expected_checkpoint_rel}"
        )
    report_path = _safe_existing_path(
        root, checkpoint_rel, f"case {case_id} checkpoint report", directory=False
    )
    report, report_payload = _read_json_with_payload(
        report_path, f"case {case_id} checkpoint report"
    )
    _require_exact_keys(
        report,
        {
            "schemaVersion",
            "profile",
            "id",
            "workspacePath",
            "preSnapshotPath",
            "platformCheckpoint",
            "toolId",
            "operation",
            "branch",
            "impactClass",
            "publicArguments",
            "targetCall",
            "preFiles",
            "preNonXmlFiles",
            "nonXmlFiles",
            "removedNonXmlPaths",
            "auxiliaryFiles",
            "seedOutputs",
            "preXmlSha256",
            "postXmlSha256",
            "delta",
            "remainingXml",
            "removedPaths",
            "ownerLinks",
            "preOwnerVersions",
            "ownerVersions",
        },
        f"case {case_id} checkpoint report",
    )
    matching_fields = (
        "id",
        "workspacePath",
        "preSnapshotPath",
        "platformCheckpoint",
        "toolId",
        "operation",
        "branch",
        "impactClass",
    )
    for field in matching_fields:
        if report.get(field) != case.get(field):
            raise CorpusError(f"case {case_id} manifest/report mismatch for {field}")
    if report.get("preFiles") != case.get("preFiles"):
        raise CorpusError(f"case {case_id} manifest/report mismatch for preFiles")
    if report.get("preOwnerVersions") != case.get("preOwnerVersions"):
        raise CorpusError(
            f"case {case_id} manifest/report mismatch for preOwnerVersions"
        )
    report_schema_version = report.get("schemaVersion")
    if (
        type(report_schema_version) is not int
        or report_schema_version != 1
        or report.get("profile") != EXPECTED_PROFILE
    ):
        raise CorpusError(f"case {case_id} report profile/schema mismatch")
    arguments = report.get("publicArguments")
    if (
        not isinstance(arguments, dict)
        or arguments.get("cwd") != "$CASE_WORKSPACE"
        or arguments.get("dryRun") is not False
        or str(root) in json.dumps(arguments, ensure_ascii=False)
    ):
        raise CorpusError(f"case {case_id} publicArguments are not deterministic/sanitized")
    target = report.get("targetCall")
    if isinstance(target, dict):
        _require_exact_keys(
            target,
            {"sequence", "resultOk", "errors", "summary"},
            f"case {case_id} targetCall",
        )
    if (
        not isinstance(target, dict)
        or not isinstance(target.get("sequence"), int)
        or isinstance(target.get("sequence"), bool)
        or target.get("sequence") < 1
        or target.get("resultOk") is not True
        or target.get("errors") != []
        or not isinstance(target.get("summary"), str)
    ):
        raise CorpusError(f"case {case_id} targetCall is incomplete or failed")

    claimed_before = _hash_map(
        report.get("preXmlSha256"), f"case {case_id} pre hashes"
    )
    before = pre_contract["hashes"]
    if claimed_before != before:
        raise CorpusError(
            f"case {case_id} pre hash claims do not match immutable pre-snapshot bytes"
        )
    after = _hash_map(report.get("postXmlSha256"), f"case {case_id} post hashes")
    actual = _snapshot_xml_hashes(workspace)
    if after != actual:
        raise CorpusError(f"case {case_id} post hash map does not match workspace files")
    seed_outputs = _string_list(
        report.get("seedOutputs"), f"case {case_id} seedOutputs", paths=True
    )
    if seed_outputs != sorted(before):
        raise CorpusError(f"case {case_id} seedOutputs do not match pre hashes")
    remaining = _string_list(
        report.get("remainingXml"), f"case {case_id} remainingXml", paths=True
    )
    if remaining != sorted(after):
        raise CorpusError(f"case {case_id} remainingXml does not match post hashes")
    removed = _removed_paths(
        report.get("removedPaths"), f"case {case_id} removedPaths", before
    )
    delta = report.get("delta")
    if not isinstance(delta, dict) or set(delta) != {
        "created",
        "modified",
        "removed",
        "unchanged",
    }:
        raise CorpusError(f"case {case_id} delta has invalid shape")
    normalized_delta = {
        name: _string_list(delta[name], f"case {case_id} delta {name}", paths=True)
        for name in ("created", "modified", "removed", "unchanged")
    }
    if normalized_delta != _classify_delta(before, after):
        raise CorpusError(f"case {case_id} delta does not match pre/post hashes")
    expected_xml_impact = next(
        (
            name
            for name in ("removed", "created", "modified", "unchanged")
            if normalized_delta[name]
        ),
        "unchanged",
    )
    if case.get("xmlImpact") != expected_xml_impact:
        raise CorpusError(
            f"case {case_id} xmlImpact must match the validated delta: "
            f"expected {expected_xml_impact!r}"
        )
    if removed != normalized_delta["removed"]:
        raise CorpusError(f"case {case_id} removedPaths do not match delta")
    non_xml_contract = _validate_non_xml_contract(
        case,
        report,
        case_id,
        workspace,
        workspace_rel,
        source,
        base_source,
        input_relative_roots,
    )
    auxiliary_files = _validate_auxiliary_files(
        case,
        report,
        case_id,
        workspace,
        workspace_rel,
        input_relative_roots,
    )
    if impact_class == "CreateOrModify" and not (
        normalized_delta["created"] or normalized_delta["modified"]
    ):
        raise CorpusError(f"case {case_id} has no create/modify delta")
    if impact_class == "RemoveOrModify" and not (
        normalized_delta["removed"] and normalized_delta["modified"]
    ):
        raise CorpusError(f"case {case_id} removal delta is incomplete")
    if impact_class == "None" and before != after:
        raise CorpusError(f"case {case_id} None impact changed XML")
    if impact_class == "None" and not any(
        non_xml_contract["delta"][name]
        for name in ("created", "modified", "removed")
    ):
        raise CorpusError(
            f"case {case_id} None XML impact must still have a non-XML mutation"
        )
    outside_inputs = sorted(
        path
        for path in set(before) | set(after)
        if not any(
            _relative_path_is_within(path, input_root)
            for input_root in input_relative_roots
        )
    )
    if outside_inputs:
        raise CorpusError(
            f"case {case_id} XML is outside every platform checkpoint "
            f"input/sourcePath: {outside_inputs}"
        )
    forbidden_dump_info = {
        (PurePosixPath(input_root) / "ConfigDumpInfo.xml").as_posix()
        for input_root in input_relative_roots
    } & (set(before) | set(after))
    if forbidden_dump_info:
        raise CorpusError(
            f"case {case_id} corpus input must not contain writer-owned "
            f"ConfigDumpInfo.xml: {sorted(forbidden_dump_info)}"
        )
    if checkpoint["kind"] == "extension":
        extension_root = input_relative_roots[0]
        base_delta = sorted(
            path
            for delta_kind in ("created", "modified", "removed")
            for path in normalized_delta[delta_kind]
            if not _relative_path_is_within(path, extension_root)
        )
        base_non_xml_delta = sorted(
            path
            for delta_kind in ("created", "modified", "removed")
            for path in non_xml_contract["delta"][delta_kind]
            if not _relative_path_is_within(path, extension_root)
        )
        if base_delta or base_non_xml_delta:
            raise CorpusError(
                f"case {case_id} extension delta is outside sourcePath and hidden "
                f"inside baseSourcePath: {base_delta + base_non_xml_delta}"
            )

    files = case.get("files")
    if not isinstance(files, list):
        raise CorpusError(f"case {case_id} files must be a list")
    manifest_files = {}
    manifest_owner_links = {}
    manifest_xml_payloads = {}
    for item in files:
        if not isinstance(item, dict):
            raise CorpusError(f"case {case_id} file entry must be an object")
        allowed_file_fields = {
            "path",
            "sha256",
            "family",
            "seed",
            "delta",
            "ownerPath",
            "newStandalone",
        }
        unknown_file_fields = set(item) - allowed_file_fields
        if unknown_file_fields:
            raise CorpusError(
                f"case {case_id} file has unknown fields: "
                f"{sorted(unknown_file_fields)}"
            )
        corpus_path = _canonical_relative_path(item.get("path"), f"case {case_id} file path")
        prefix = f"{workspace_rel}/"
        if not corpus_path.startswith(prefix):
            raise CorpusError(f"case {case_id} file is outside workspacePath")
        workspace_path = corpus_path[len(prefix) :]
        xml_file = _safe_existing_path(
            root, corpus_path, f"case {case_id} manifest XML", directory=False
        )
        try:
            xml_payload = xml_file.read_bytes()
        except OSError as error:
            raise CorpusError(
                f"case {case_id} cannot read manifest XML {corpus_path}: {error}"
            ) from error
        _version, _owner_type, root_qname = _parse_root_details_payload(
            xml_payload, corpus_path
        )
        manifest_xml_payloads[workspace_path] = xml_payload
        expected_family = XML_FAMILY_BY_ROOT_QNAME.get(root_qname)
        if expected_family is None:
            raise CorpusError(
                f"case {case_id} XML root has no known family: {root_qname}"
            )
        if item.get("family") != expected_family:
            raise CorpusError(
                f"case {case_id} file family does not match XML root {root_qname}: "
                f"expected {expected_family!r}"
            )
        digest = item.get("sha256")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise CorpusError(f"case {case_id} file hash is invalid")
        if workspace_path in manifest_files:
            raise CorpusError(f"case {case_id} has duplicate file path")
        manifest_files[workspace_path] = digest
        expected_delta = next(
            (name for name, paths in normalized_delta.items() if workspace_path in paths), None
        )
        if (
            not isinstance(item.get("seed"), bool)
            or item.get("seed") != (workspace_path in before)
            or item.get("delta") != expected_delta
        ):
            raise CorpusError(f"case {case_id} file seed/delta metadata mismatch")
        owner_path = item.get("ownerPath")
        if owner_path is not None:
            manifest_owner_links[corpus_path] = _canonical_relative_path(
                owner_path, f"case {case_id} ownerPath"
            )
        if "newStandalone" in item:
            raise CorpusError(
                f"case {case_id} newStandalone XML has no platform checkpoint "
                "contract and is forbidden in the platform corpus"
            )
    if manifest_files != after:
        raise CorpusError(f"case {case_id} manifest files do not match post hash map")
    if _hash_xml_payloads(manifest_xml_payloads) != after:
        raise CorpusError(
            f"case {case_id} captured XML bytes do not match the validated post hashes"
        )
    post_semantic_sha256 = uuid_normalized_semantic_sha256(
        manifest_xml_payloads, f"case {case_id} post XML"
    )
    transition_semantic_digest = transition_semantic_sha256(
        pre_contract["payloads"],
        manifest_xml_payloads,
        f"case {case_id} pre/post transition",
    )

    platform_input_roots = [source]
    if base_source is not None:
        platform_input_roots.append(base_source)
    for workspace_path in after:
        xml_path = _safe_existing_path(
            workspace,
            workspace_path,
            f"case {case_id} post XML",
            directory=False,
        )
        if not any(
            _is_relative_to(xml_path, input_root)
            for input_root in platform_input_roots
        ):
            raise CorpusError(
                f"case {case_id} post XML is outside every platform checkpoint "
                f"input/sourcePath: {workspace_path}"
            )

    manifest_removed = _string_list(
        case.get("removedPaths"), f"case {case_id} manifest removedPaths", paths=True
    )
    if manifest_removed != [f"{workspace_rel}/{path}" for path in removed]:
        raise CorpusError(f"case {case_id} manifest/report removedPaths mismatch")
    owner_links = report.get("ownerLinks")
    if not isinstance(owner_links, dict) or owner_links != manifest_owner_links:
        raise CorpusError(f"case {case_id} ownerPath links disagree with report")
    owner_versions = case.get("ownerVersions")
    if (
        not isinstance(owner_versions, dict)
        or owner_versions != report.get("ownerVersions")
        or not owner_versions
    ):
        raise CorpusError(f"case {case_id} owner version declarations disagree")
    owner_types = []
    owner_type_by_path: dict[Path, str] = {}
    owner_root_by_manifest_path: dict[str, Path] = {}
    for raw_owner_path, declared_version in owner_versions.items():
        owner_path = _canonical_relative_path(raw_owner_path, f"case {case_id} owner path")
        if declared_version != EXPECTED_EXPORT_VERSION:
            raise CorpusError(f"case {case_id} owner version must be 2.20")
        owner_file = _safe_existing_path(
            root, owner_path, f"case {case_id} owner", directory=False
        )
        if not _is_relative_to(owner_file, workspace):
            raise CorpusError(f"case {case_id} owner is outside workspace")
        actual_version, owner_type = _parse_root_contract(owner_file)
        if actual_version != EXPECTED_EXPORT_VERSION:
            raise CorpusError(f"case {case_id} owner root version must be 2.20")
        if owner_type is None:
            raise CorpusError(f"case {case_id} owner type is absent")
        owner_types.append(owner_type)
        owner_type_by_path[owner_file] = owner_type
        owner_root_by_manifest_path[owner_path] = owner_file.parent
    owner_paths = set(owner_versions)
    for workspace_path in after:
        corpus_path = f"{workspace_rel}/{workspace_path}"
        version, _owner_type = _parse_root_contract(workspace / workspace_path)
        if version is not None and version != EXPECTED_EXPORT_VERSION:
            raise CorpusError(f"case {case_id} version-bearing root is not 2.20: {corpus_path}")
        if version is None:
            linked_owner = owner_links.get(corpus_path)
            if linked_owner is None:
                raise CorpusError(f"case {case_id} versionless XML needs ownerPath: {corpus_path}")
            if linked_owner not in owner_paths:
                raise CorpusError(f"case {case_id} ownerPath is not a same-checkpoint owner")
            xml_path = workspace / workspace_path
            containing_owners = [
                (owner_path, owner_root)
                for owner_path, owner_root in owner_root_by_manifest_path.items()
                if _is_relative_to(xml_path, owner_root)
            ]
            if not containing_owners:
                raise CorpusError(
                    f"case {case_id} versionless XML has no physically containing owner root: "
                    f"{corpus_path}"
                )
            deepest = max(len(owner_root.parts) for _owner_path, owner_root in containing_owners)
            deepest_owners = sorted(
                owner_path
                for owner_path, owner_root in containing_owners
                if len(owner_root.parts) == deepest
            )
            if len(deepest_owners) != 1 or linked_owner != deepest_owners[0]:
                raise CorpusError(
                    f"case {case_id} ownerPath must name the unique deepest containing "
                    f"owner root for {corpus_path}"
                )
        elif corpus_path in owner_links:
            raise CorpusError(
                f"case {case_id} version-bearing XML must not declare ownerPath: {corpus_path}"
            )

    expected_types = sorted(EXPECTED_OWNER_TYPES[checkpoint["kind"]])
    if sorted(owner_types) != expected_types:
        raise CorpusError(
            f"case {case_id} owner types {sorted(owner_types)} != {expected_types}"
        )
    source_owners = [path for path in owner_type_by_path if _is_relative_to(path, source)]
    if len(source_owners) != 1:
        raise CorpusError(f"case {case_id} sourcePath must contain exactly one owner")
    if checkpoint["kind"] == "extension":
        base_owners = [
            path for path in owner_type_by_path if _is_relative_to(path, base_source)
        ]
        if len(base_owners) != 1:
            raise CorpusError(
                f"case {case_id} baseSourcePath must contain exactly one owner"
            )

    def boundary_hashes(hashes: dict[str, str], boundary: str) -> dict[str, str]:
        prefix = f"{boundary}/"
        return {
            path[len(prefix) :]: digest
            for path, digest in sorted(hashes.items())
            if path.startswith(prefix)
        }

    source_boundary = input_relative_roots[0]
    base_boundary = input_relative_roots[1] if base_source is not None else None
    return {
        "id": case_id,
        "toolId": case.get("toolId"),
        "impactClass": impact_class,
        "workspace": workspace,
        "checkpoint": checkpoint,
        "source": source,
        "baseSource": base_source,
        "ownerVersions": dict(sorted(owner_versions.items())),
        "preSignature": pre_contract["signature"],
        "preOwnerPaths": sorted(pre_contract["ownerVersions"]),
        "preSemanticSha256": pre_contract["semanticSha256"],
        "transitionSemanticSha256": transition_semantic_digest,
        "postSemanticSha256": post_semantic_sha256,
        "nonXmlContract": non_xml_contract,
        "auxiliaryFiles": auxiliary_files,
        "sourceExpectedXmlHashes": boundary_hashes(after, source_boundary),
        "sourceExpectedNonXmlHashes": boundary_hashes(
            non_xml_contract["after"], source_boundary
        ),
        "sourceExpectedEmptyDirectoryPaths": _empty_directory_paths(source),
        "baseExpectedXmlHashes": (
            boundary_hashes(after, base_boundary)
            if base_boundary is not None
            else None
        ),
        "baseExpectedNonXmlHashes": (
            boundary_hashes(non_xml_contract["after"], base_boundary)
            if base_boundary is not None
            else None
        ),
        "baseExpectedEmptyDirectoryPaths": (
            _empty_directory_paths(base_source)
            if base_source is not None
            else None
        ),
        "sourceOwnerRelativePaths": sorted(
            path.relative_to(source).as_posix() for path in source_owners
        ),
        "report": report,
        "reportSha256": hashlib.sha256(report_payload).hexdigest(),
        "targetSequence": target["sequence"],
    }


def load_corpus(
    manifest_path: Path,
    *,
    repo_root: Path | None = None,
    home_root: Path | None = None,
    mandatory_case_ids=MANDATORY_CASE_IDS,
) -> dict:
    manifest_path = Path(manifest_path)
    if not manifest_path.is_absolute():
        raise CorpusError("corpus manifest path must be absolute")
    if manifest_path.is_symlink():
        raise CorpusError("corpus manifest symlink is forbidden")
    try:
        manifest_path = manifest_path.resolve(strict=True)
    except OSError as error:
        raise CorpusError(f"corpus manifest is missing or unreadable: {error}") from error
    if manifest_path.name != "corpus-manifest.json":
        raise CorpusError("corpus manifest filename must be corpus-manifest.json")
    corpus_root = manifest_path.parent
    repo = Path(repo_root or Path(__file__).resolve().parents[2]).resolve()
    home = Path(home_root or Path.home()).resolve()
    if (
        corpus_root == Path(corpus_root.anchor)
        or _is_relative_to(corpus_root, repo)
        or _is_relative_to(repo, corpus_root)
        or _is_relative_to(corpus_root, home)
        or _is_relative_to(home, corpus_root)
    ):
        raise CorpusError("corpus root overlaps a broad, home, or repository path")
    initial_snapshot = snapshot_regular_tree(corpus_root)
    manifest, manifest_payload = _read_json_with_payload(
        manifest_path, "corpus manifest"
    )
    manifest_sha256 = hashlib.sha256(manifest_payload).hexdigest()
    if initial_snapshot["files"].get("corpus-manifest.json") != manifest_sha256:
        raise CorpusError(
            "corpus manifest bytes changed between the immutable tree snapshot "
            "and JSON parsing"
        )
    _require_exact_keys(
        manifest,
        {"schemaVersion", "profile", "emptyDirectoryPaths", "cases"},
        "corpus manifest",
    )
    manifest_schema_version = manifest.get("schemaVersion")
    if type(manifest_schema_version) is not int or manifest_schema_version != 2:
        raise CorpusError("corpus schemaVersion must be 2")
    if manifest.get("profile") != EXPECTED_PROFILE:
        raise CorpusError(f"corpus profile must be {EXPECTED_PROFILE}")
    empty_directory_paths = _string_list(
        manifest.get("emptyDirectoryPaths"),
        "corpus emptyDirectoryPaths",
        paths=True,
    )
    if empty_directory_paths != initial_snapshot["emptyDirectoryPaths"]:
        raise CorpusError(
            "corpus empty directory inventory is not exact; "
            f"declared={empty_directory_paths}, "
            f"actual={initial_snapshot['emptyDirectoryPaths']}"
        )
    cases = manifest.get("cases")
    if not isinstance(cases, list) or not cases:
        raise CorpusError("corpus cases must be a non-empty list")
    ids = [case.get("id") if isinstance(case, dict) else None for case in cases]
    if any(
        not isinstance(case_id, str) or CASE_ID_RE.fullmatch(case_id) is None
        for case_id in ids
    ):
        raise CorpusError(
            "every corpus case id must be one safe kebab-case filename component"
        )
    if ids != sorted(set(ids)):
        raise CorpusError("corpus case ids must be sorted and unique")
    known_ids = set(ids)
    expected_ids = set(mandatory_case_ids)
    missing_mandatory = sorted(expected_ids - known_ids)
    unexpected_cases = sorted(known_ids - expected_ids)
    if missing_mandatory or unexpected_cases:
        raise CorpusError(
            "corpus case inventory must match the exact checkpoint contract; "
            f"missing={missing_mandatory}, unexpected={unexpected_cases}"
        )
    normalized = [_validate_case(corpus_root, case, known_ids) for case in cases]
    contract_sha256 = case_contract_sha256(
        cases, normalized, empty_directory_paths
    )
    if (
        expected_ids == set(MANDATORY_CASE_IDS)
        and contract_sha256 != EXPECTED_CASE_CONTRACT_SHA256
    ):
        raise CorpusError(
            "corpus case contract fields do not match the pinned public-writer inventory; "
            f"expected {EXPECTED_CASE_CONTRACT_SHA256}, got {contract_sha256}"
        )
    declared_xml_paths = [
        item["path"]
        for case in cases
        for collection in (case.get("preFiles", []), case.get("files", []))
        for item in collection
        if isinstance(item, dict) and isinstance(item.get("path"), str)
    ]
    if len(declared_xml_paths) != len(set(declared_xml_paths)):
        raise CorpusError("corpus XML paths must be globally unique across cases")
    declared_xml = set(declared_xml_paths)
    actual_xml = set(_snapshot_xml_hashes(corpus_root))
    if declared_xml != actual_xml:
        raise CorpusError(
            "corpus XML inventory is not exact; "
            f"unreferenced={sorted(actual_xml - declared_xml)}, "
            f"missing={sorted(declared_xml - actual_xml)}"
        )
    for field in ("preNonXmlFiles", "nonXmlFiles"):
        declared_non_xml_paths = [
            item["path"]
            for case in cases
            for item in case.get(field, [])
            if isinstance(item, dict) and isinstance(item.get("path"), str)
        ]
        if len(declared_non_xml_paths) != len(set(declared_non_xml_paths)):
            raise CorpusError(
                f"corpus {field} paths must be globally unique across cases"
            )
    for item in normalized:
        if item["checkpoint"]["kind"] in {"epf", "erf"}:
            _artifact_source_pair(item)
    sequences = [item["targetSequence"] for item in normalized]
    if sequences != list(range(1, len(normalized) + 1)):
        raise CorpusError("targetCall sequences must be complete, ordered, and unique")
    coverage: dict[str, list[str]] = {case_id: [] for case_id in ids}
    for item in normalized:
        for covered in item["checkpoint"]["coveredCaseIds"]:
            coverage[covered].append(item["id"])
    uncovered = sorted(case_id for case_id, owners in coverage.items() if not owners)
    duplicates = {case_id: owners for case_id, owners in coverage.items() if len(owners) > 1}
    if uncovered:
        raise CorpusError(f"uncovered corpus cases: {uncovered}")
    if duplicates:
        raise CorpusError(f"duplicate checkpoint coverage: {duplicates}")

    declared_entries = [
        ("corpus-manifest.json", manifest_sha256, "corpus manifest")
    ]
    normalized_by_id = {item["id"]: item for item in normalized}
    for case in cases:
        case_id = case["id"]
        normalized_case = normalized_by_id[case_id]
        declared_entries.append(
            (
                case["checkpoint"],
                normalized_case["reportSha256"],
                f"case {case_id} checkpoint report",
            )
        )
        for field in (
            "preFiles",
            "files",
            "preNonXmlFiles",
            "nonXmlFiles",
            "auxiliaryFiles",
        ):
            for entry in case[field]:
                declared_entries.append(
                    (
                        entry["path"],
                        entry["sha256"],
                        f"case {case_id} {field}",
                    )
                )
    declared_snapshot: dict[str, str] = {}
    declared_labels: dict[str, str] = {}
    for path, digest, label in declared_entries:
        previous = declared_labels.get(path)
        if previous is not None:
            raise CorpusError(
                f"corpus regular file is declared more than once: {path}: "
                f"{previous}, {label}"
            )
        declared_labels[path] = label
        declared_snapshot[path] = digest
    initial_files = initial_snapshot["files"]
    if declared_snapshot != initial_files:
        unreferenced = sorted(set(initial_files) - set(declared_snapshot))
        missing = sorted(set(declared_snapshot) - set(initial_files))
        mismatched = sorted(
            path
            for path in set(initial_files) & set(declared_snapshot)
            if initial_files[path] != declared_snapshot[path]
        )
        raise CorpusError(
            "corpus regular-file inventory/hash is not exact; "
            f"unreferenced={unreferenced}, missing={missing}, "
            f"mismatched={mismatched}"
        )
    final_snapshot = snapshot_regular_tree(corpus_root)
    if final_snapshot != initial_snapshot:
        delta = _tree_delta(initial_snapshot, final_snapshot)
        raise CorpusError(
            "corpus changed while its manifest and evidence were being validated; "
            f"added={delta['added']}, removed={delta['removed']}, "
            f"modified={delta['modified']}, "
            f"addedDirectories={delta['addedDirectories']}, "
            f"removedDirectories={delta['removedDirectories']}"
        )
    return {
        "root": corpus_root,
        "manifestPath": manifest_path,
        "manifestSha256": manifest_sha256,
        "snapshot": initial_snapshot,
        "emptyDirectoryPaths": empty_directory_paths,
        "profile": manifest["profile"],
        "caseContractSha256": contract_sha256,
        "cases": normalized,
        "selected": normalized,
        "notSelected": [],
    }


def _redact_text(value: str, redactions) -> str:
    replacements = []
    for path, replacement in redactions or []:
        raw = str(path)
        replacements.append((raw, replacement))
        try:
            resolved = str(Path(path).resolve())
        except OSError:
            resolved = raw
        replacements.append((resolved, replacement))
    for raw, replacement in sorted(set(replacements), key=lambda item: -len(item[0])):
        if raw:
            value = value.replace(raw, replacement)
    return value


class CommandRunner:
    """Run one local argv array with bounded diagnostics and tree-safe timeout."""

    def __init__(self, *, timeout_seconds: float, diagnostic_limit: int = 4096):
        if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
            raise ValueError("timeout_seconds must be finite and positive")
        if diagnostic_limit <= 0:
            raise ValueError("diagnostic_limit must be positive")
        self.timeout_seconds = timeout_seconds
        self.diagnostic_limit = diagnostic_limit

    def run(self, argv, *, cwd: Path, redactions=None) -> dict:
        if (
            not isinstance(argv, list)
            or not argv
            or any(not isinstance(argument, str) or not argument or "\0" in argument for argument in argv)
        ):
            raise SourceError("command must be a non-empty argument array")
        for argument in argv[1:]:
            option = argument.split("=", 1)[0]
            if option in FORBIDDEN_CREDENTIAL_OPTIONS:
                raise SourceError("credential options are forbidden in platform commands")
        cwd = Path(cwd)
        if not cwd.is_absolute() or not cwd.is_dir():
            raise SourceError(f"command cwd must be an existing absolute directory: {cwd}")
        environment = {
            "LANG": os.environ.get("LANG", "C.UTF-8"),
            "LC_ALL": os.environ.get("LC_ALL", "C.UTF-8"),
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "TMPDIR": str(cwd),
        }
        if os.environ.get("HOME"):
            environment["HOME"] = os.environ["HOME"]
        started = time.monotonic()
        try:
            process = subprocess.Popen(
                argv,
                cwd=cwd,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
        except (OSError, ValueError) as error:
            raise SourceError(f"cannot start command {argv[0]!r}: {error}") from error
        try:
            stdout, stderr = process.communicate(timeout=self.timeout_seconds)
        except subprocess.TimeoutExpired as error:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.communicate(timeout=1)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.communicate()
            raise SourceError(
                f"command timed out after {self.timeout_seconds:g}s; process tree terminated"
            ) from error
        duration_ms = int(round((time.monotonic() - started) * 1000))
        stdout_text = stdout.decode("utf-8", errors="replace")
        stderr_text = stderr.decode("utf-8", errors="replace")
        return {
            "argv": [_redact_text(argument, redactions) for argument in argv],
            "exitCode": process.returncode,
            "stdoutSha256": hashlib.sha256(stdout).hexdigest(),
            "stderrSha256": hashlib.sha256(stderr).hexdigest(),
            "stdout": _redact_text(stdout_text, redactions)[: self.diagnostic_limit],
            "stderr": _redact_text(stderr_text, redactions)[: self.diagnostic_limit],
            "durationMs": duration_ms,
        }


def require_executable_identity(ibcmd: Path, expected_sha256: str) -> str:
    ibcmd = Path(ibcmd)
    if not ibcmd.is_absolute() or ibcmd.is_symlink() or not ibcmd.is_file():
        raise SourceError(f"pinned ibcmd path is no longer a safe regular file: {ibcmd}")
    actual_sha256 = _sha256_file(ibcmd)
    if actual_sha256 != expected_sha256:
        raise SourceError(
            "pinned ibcmd changed during the platform gate; "
            f"expected {expected_sha256}, got {actual_sha256}"
        )
    return actual_sha256


def verify_platform_version(
    ibcmd: Path,
    runner: CommandRunner,
    cwd: Path,
    *,
    redactions=None,
    expected_sha256: str | None = None,
) -> dict:
    ibcmd = Path(ibcmd)
    if not ibcmd.is_absolute() or ibcmd.is_symlink() or not ibcmd.is_file():
        raise SourceError(f"ibcmd must be an absolute regular executable: {ibcmd}")
    if not os.access(ibcmd, os.X_OK):
        raise SourceError(f"ibcmd is not executable: {ibcmd}")
    ibcmd_sha256 = _sha256_file(ibcmd)
    expected_sha256 = expected_sha256 or EXPECTED_IBCMD_SHA256
    if SHA256_RE.fullmatch(expected_sha256) is None:
        raise SourceError("expected ibcmd SHA-256 must be a lowercase hexadecimal digest")
    if ibcmd_sha256 != expected_sha256:
        raise PlatformBinaryError(
            f"ibcmd SHA-256 must equal {expected_sha256}, got {ibcmd_sha256}",
            ibcmd_sha256=ibcmd_sha256,
        )
    require_executable_identity(ibcmd, ibcmd_sha256)
    record = runner.run(
        [str(ibcmd), "--version"], cwd=Path(cwd), redactions=redactions
    )
    require_executable_identity(ibcmd, ibcmd_sha256)
    version = record["stdout"].strip()
    if record["exitCode"] != 0 or version != EXPECTED_PLATFORM_VERSION:
        raise PlatformVersionError(
            f"ibcmd version must equal {EXPECTED_PLATFORM_VERSION}, got {version!r} "
            f"with exit {record['exitCode']}",
            record=record,
            version=version,
            ibcmd_sha256=ibcmd_sha256,
        )
    return {
        "version": version,
        "command": record,
        "ibcmdSha256": ibcmd_sha256,
    }


def _connection_options(round_root: Path) -> list[str]:
    ib_root = round_root / "ib"
    return [
        f"--db-path={ib_root / 'db'}",
        f"--data={ib_root / 'data'}",
        f"--temp={ib_root / 'temp'}",
        f"--users-data={ib_root / 'users'}",
        f"--session-data={ib_root / 'session'}",
        f"--log-data={ib_root / 'log'}",
    ]


def build_checkpoint_round_commands(
    ibcmd: Path,
    kind: str,
    round_root: Path,
    source: Path,
    output: Path,
    *,
    base_source: Path | None = None,
) -> list[dict]:
    """Build argv arrays in the exact order proven against local ibcmd 8.3.27."""
    ibcmd = Path(ibcmd)
    round_root = Path(round_root)
    source = Path(source)
    output = Path(output)
    if kind not in {"configuration", "extension", "epf", "erf"}:
        raise SourceError(f"unsupported platform checkpoint kind: {kind}")
    if kind == "extension" and base_source is None:
        raise SourceError("extension checkpoint requires baseSourcePath")
    options = _connection_options(round_root)
    executable = str(ibcmd)

    if kind == "configuration":
        return [
            {
                "stage": "import-apply",
                "argv": [
                    executable,
                    "infobase",
                    "create",
                    *options,
                    f"--import={source}",
                    "--apply",
                    "--force",
                ],
            },
            {
                "stage": "check",
                "argv": [executable, "config", *options, "check", "--force"],
            },
            {
                "stage": "export",
                "argv": [executable, "config", *options, "export", str(output)],
            },
        ]

    if kind == "extension":
        base_source = Path(base_source)
        extension_option = "--extension=CorpusExtension"
        return [
            {
                "stage": "base-import-apply",
                "argv": [
                    executable,
                    "infobase",
                    "create",
                    *options,
                    f"--import={base_source}",
                    "--apply",
                    "--force",
                ],
            },
            {
                "stage": "extension-create",
                "argv": [
                    executable,
                    "extension",
                    *options,
                    "create",
                    "--name=CorpusExtension",
                    "--name-prefix=CorpusExtension_",
                    "--purpose=customization",
                ],
            },
            {
                "stage": "extension-import",
                "argv": [
                    executable,
                    "config",
                    *options,
                    "import",
                    extension_option,
                    str(source),
                ],
            },
            {
                "stage": "extension-check",
                "argv": [
                    executable,
                    "config",
                    *options,
                    "check",
                    extension_option,
                    "--force",
                ],
            },
            {
                "stage": "extension-apply",
                "argv": [
                    executable,
                    "config",
                    *options,
                    "apply",
                    extension_option,
                    "--force",
                ],
            },
            {
                "stage": "extension-export",
                "argv": [
                    executable,
                    "config",
                    *options,
                    "export",
                    extension_option,
                    str(output),
                ],
            },
        ]

    artifact = round_root / f"artifact.{kind}"
    return [
        {
            "stage": "empty-infobase-create",
            "argv": [executable, "infobase", "create", *options],
        },
        {
            "stage": "artifact-import",
            "argv": [
                executable,
                "config",
                *options,
                "import",
                f"--out={artifact}",
                str(source),
            ],
        },
        {
            "stage": "artifact-export",
            "argv": [
                executable,
                "config",
                *options,
                "export",
                f"--file={artifact}",
                str(output),
            ],
        },
    ]


def _raw_xml_hashes(root: Path) -> dict[str, str]:
    return dict(sorted(_snapshot_xml_hashes(root).items()))


def _snapshot_owner_versions(snapshot: dict, relative_paths: list[str]) -> list[dict]:
    evidence = []
    for relative in sorted(relative_paths):
        payload = snapshot["rawPayloads"].get(relative)
        if payload is None:
            evidence.append({"path": relative, "version": None, "valid": False})
            continue
        version, _owner_type = _parse_root_contract_payload(
            payload, f"{snapshot['label']}/{relative}"
        )
        evidence.append(
            {
                "path": relative,
                "version": version,
                "valid": version == EXPECTED_EXPORT_VERSION,
            }
        )
    return evidence


def _artifact_source_pair(item: dict) -> tuple[Path, Path]:
    owner_paths = item["sourceOwnerRelativePaths"]
    if len(owner_paths) != 1:
        raise SourceError(f"artifact checkpoint {item['id']} must have one source owner")
    source = Path(item["source"])
    descriptor = source.joinpath(*PurePosixPath(owner_paths[0]).parts)
    if descriptor.parent != source:
        raise CorpusError(
            f"artifact checkpoint {item['id']} owner descriptor must be at source root"
        )
    content = descriptor.with_suffix("")
    if descriptor.is_symlink() or not descriptor.is_file():
        raise CorpusError(
            f"artifact checkpoint {item['id']} descriptor is not a safe regular file"
        )
    if content.is_symlink() or not content.is_dir():
        raise CorpusError(
            f"artifact checkpoint {item['id']} content is not a safe directory"
        )
    expected_names = {descriptor.name, content.name}
    try:
        with os.scandir(source) as iterator:
            siblings = sorted(entry.name for entry in iterator if entry.name not in expected_names)
    except OSError as error:
        raise CorpusError(
            f"cannot enumerate artifact checkpoint {item['id']} source: {error}"
        ) from error
    if siblings:
        raise CorpusError(
            f"artifact checkpoint {item['id']} has unpaired source siblings: {siblings}"
        )
    return descriptor, content


def _artifact_snapshot_owner_versions(snapshot: dict) -> list[dict]:
    payload = snapshot["rawPayloads"].get("descriptor.xml")
    if payload is None:
        return [{"path": "descriptor.xml", "version": None, "valid": False}]
    version, _owner_type = _parse_root_contract_payload(
        payload, f"{snapshot['label']}/descriptor.xml"
    )
    return [
        {
            "path": "descriptor.xml",
            "version": version,
            "valid": version == EXPECTED_EXPORT_VERSION,
        }
    ]


def _copy_regular_tree(source: Path, destination: Path) -> None:
    """Copy one validated input tree without following links or special files."""
    source = Path(source)
    destination = Path(destination)
    if source.is_symlink() or not source.is_dir():
        raise SourceError(f"platform input is not a safe directory: {source}")
    if destination.exists() or destination.is_symlink():
        raise SourceError(f"private platform input target already exists: {destination}")
    try:
        destination.mkdir(parents=True)
    except OSError as error:
        raise SourceError(f"cannot create private platform input: {error}") from error

    def visit(source_directory: Path, destination_directory: Path) -> None:
        try:
            with os.scandir(source_directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(
                f"cannot enumerate private platform input source: {error}"
            ) from error
        for entry in entries:
            source_path = Path(entry.path)
            destination_path = destination_directory / entry.name
            try:
                if entry.is_symlink():
                    raise SourceError(
                        f"symlink is forbidden in private platform input: {source_path}"
                    )
                if entry.is_dir(follow_symlinks=False):
                    destination_path.mkdir()
                    visit(source_path, destination_path)
                elif entry.is_file(follow_symlinks=False):
                    shutil.copyfile(source_path, destination_path, follow_symlinks=False)
                else:
                    raise SourceError(
                        f"special entry is forbidden in private platform input: {source_path}"
                    )
            except OSError as error:
                raise SourceError(
                    f"cannot copy private platform input entry {source_path}: {error}"
                ) from error

    visit(source, destination)


def _copy_artifact_pair(
    source_pair: tuple[Path, Path], destination_descriptor: Path
) -> tuple[Path, Path]:
    """Copy one validated descriptor/content pair to an untrusted round input."""
    descriptor, content = (Path(path) for path in source_pair)
    destination_descriptor = Path(destination_descriptor)
    destination_content = destination_descriptor.with_suffix("")
    if descriptor.is_symlink() or not descriptor.is_file():
        raise SourceError(f"artifact input descriptor is not a safe file: {descriptor}")
    if destination_descriptor.exists() or destination_descriptor.is_symlink():
        raise SourceError(
            f"private artifact input descriptor already exists: {destination_descriptor}"
        )
    try:
        destination_descriptor.parent.mkdir(parents=True, exist_ok=False)
        shutil.copyfile(descriptor, destination_descriptor, follow_symlinks=False)
    except OSError as error:
        raise SourceError(f"cannot copy private artifact descriptor: {error}") from error
    _copy_regular_tree(content, destination_content)
    return destination_descriptor, destination_content


def _partial_checkpoint_result(
    item: dict,
    kind: str,
    commands: list[dict],
    round_number: int | None,
    stage: str,
    evidence_sha256: dict | None = None,
) -> dict:
    return {
        "id": item["id"],
        "toolId": item.get("toolId"),
        "kind": kind,
        "coveredCaseIds": item["checkpoint"]["coveredCaseIds"],
        "verdict": "source-error",
        "failedRound": round_number,
        "failedStage": stage,
        "commands": list(commands),
        "commandCount": len(commands),
        "durationMs": sum(entry["durationMs"] for entry in commands),
        "sourceComparison": None,
        "roundtripComparison": None,
        "evidenceSha256": evidence_sha256,
        "ownerVersions": {"source": EXPECTED_EXPORT_VERSION},
    }


def _checkpoint_execution_error(
    error: SourceError,
    item: dict,
    kind: str,
    commands: list[dict],
    round_number: int | None,
    stage: str,
    evidence_sha256: dict | None = None,
) -> CheckpointExecutionError:
    return CheckpointExecutionError(
        str(error),
        checkpoint=_partial_checkpoint_result(
            item,
            kind,
            commands,
            round_number,
            stage,
            evidence_sha256,
        ),
    )


def run_checkpoint(
    item: dict,
    ibcmd: Path,
    runner: CommandRunner,
    checkpoint_root: Path,
    corpus_root: Path,
    *,
    pinned_ibcmd_sha256: str | None = None,
) -> dict:
    """Execute two isolated platform rounds for one validated checkpoint."""
    checkpoint_root = Path(checkpoint_root)
    corpus_root = Path(corpus_root)
    if checkpoint_root.exists():
        raise SourceError(f"checkpoint evidence target already exists and must be empty: {checkpoint_root}")
    try:
        checkpoint_root.mkdir(parents=True)
    except OSError as error:
        raise SourceError(f"cannot create checkpoint evidence target: {error}") from error
    kind = item["checkpoint"]["kind"]
    pinned_ibcmd_sha256 = pinned_ibcmd_sha256 or _sha256_file(Path(ibcmd))
    if SHA256_RE.fullmatch(pinned_ibcmd_sha256) is None:
        raise SourceError("pinned ibcmd SHA-256 is invalid")
    require_executable_identity(Path(ibcmd), pinned_ibcmd_sha256)
    source = Path(item["source"])
    base_source = Path(item["baseSource"]) if item.get("baseSource") is not None else None
    artifact_kind = kind in {"epf", "erf"}
    if artifact_kind:
        _artifact_source_pair(item)
    commands = []
    exports = []
    export_snapshots = []
    round_input_snapshots = []
    round_base_input_snapshots = []
    retained_file_evidence = []
    private_source = checkpoint_root / "input" / "source"
    _copy_regular_tree(source, private_source)
    private_base_source = None
    if base_source is not None:
        private_base_source = checkpoint_root / "input" / "base"
        _copy_regular_tree(base_source, private_base_source)
    try:
        if artifact_kind:
            owner_relative = item["sourceOwnerRelativePaths"][0]
            private_descriptor = private_source.joinpath(
                *PurePosixPath(owner_relative).parts
            )
            private_artifact_source = (
                private_descriptor,
                private_descriptor.with_suffix(""),
            )
            source_snapshot = capture_artifact_xml_snapshot(*private_artifact_source)
        else:
            private_artifact_source = None
            source_snapshot = capture_directory_xml_snapshot(private_source)
        base_snapshot = (
            capture_directory_xml_snapshot(private_base_source)
            if private_base_source is not None
            else None
        )
        expected_source_identity = _expected_checkpoint_snapshot_identity(
            item, artifact=artifact_kind
        )
        if _snapshot_raw_identity(source_snapshot) != expected_source_identity:
            raise SourceError(
                f"private checkpoint input differs from the manifest-bound "
                f"post-state for {item['id']}"
            )
        if base_snapshot is not None:
            expected_base_identity = _expected_checkpoint_snapshot_identity(
                item, artifact=False, base=True
            )
            if _snapshot_raw_identity(base_snapshot) != expected_base_identity:
                raise SourceError(
                    f"private base checkpoint input differs from the manifest-bound "
                    f"post-state for {item['id']}"
                )
    except SourceError as error:
        raise _checkpoint_execution_error(
            error, item, kind, commands, None, "private-input-snapshot"
        ) from error
    source_hashes = source_snapshot["rawHashes"]
    source_non_xml_hashes = source_snapshot["rawNonXmlHashes"]
    source_empty_directory_paths = source_snapshot["emptyDirectoryPaths"]
    base_hashes = base_snapshot["rawHashes"] if base_snapshot is not None else None
    base_non_xml_hashes = (
        base_snapshot["rawNonXmlHashes"] if base_snapshot is not None else None
    )
    base_empty_directory_paths = (
        base_snapshot["emptyDirectoryPaths"] if base_snapshot is not None else None
    )
    retained_file_evidence.append(
        (
            "source",
            source_snapshot,
            "artifact" if private_artifact_source is not None else "directory",
            private_artifact_source or private_source,
        )
    )
    if base_snapshot is not None:
        retained_file_evidence.append(
            ("base", base_snapshot, "directory", private_base_source)
        )

    def evidence_hashes() -> dict:
        evidence = {
            "sourceXml": source_hashes,
            "sourceNonXml": source_non_xml_hashes,
            "sourceEmptyDirectoryPaths": source_empty_directory_paths,
        }
        if base_hashes is not None:
            evidence["baseXml"] = base_hashes
            evidence["baseNonXml"] = base_non_xml_hashes
            evidence["baseEmptyDirectoryPaths"] = base_empty_directory_paths
        for index, snapshot in enumerate(round_input_snapshots, 1):
            evidence[f"round{index}InputXml"] = snapshot["rawHashes"]
            evidence[f"round{index}InputNonXml"] = snapshot["rawNonXmlHashes"]
            evidence[f"round{index}InputEmptyDirectoryPaths"] = snapshot[
                "emptyDirectoryPaths"
            ]
        for index, snapshot in enumerate(round_base_input_snapshots, 1):
            evidence[f"round{index}BaseInputXml"] = snapshot["rawHashes"]
            evidence[f"round{index}BaseInputNonXml"] = snapshot["rawNonXmlHashes"]
            evidence[f"round{index}BaseInputEmptyDirectoryPaths"] = snapshot[
                "emptyDirectoryPaths"
            ]
        for index, snapshot in enumerate(export_snapshots, 1):
            evidence[f"export{index}Xml"] = snapshot["rawHashes"]
            evidence[f"export{index}NonXml"] = snapshot["rawNonXmlHashes"]
            evidence[f"export{index}EmptyDirectoryPaths"] = snapshot[
                "emptyDirectoryPaths"
            ]
        return evidence

    def checkpoint_error(
        error: SourceError, round_number: int | None, stage: str
    ) -> CheckpointExecutionError:
        return _checkpoint_execution_error(
            error,
            item,
            kind,
            commands,
            round_number,
            stage,
            evidence_sha256=evidence_hashes(),
        )

    redactions = [
        (checkpoint_root, "$EVIDENCE"),
        (corpus_root, "$CORPUS"),
        (Path(ibcmd), "$IBCMD"),
    ]

    for round_number in (1, 2):
        round_root = checkpoint_root / f"round{round_number}"
        try:
            round_root.mkdir()
        except OSError as error:
            source_error = SourceError(f"cannot create isolated round directory: {error}")
            raise checkpoint_error(source_error, round_number, "round-setup") from error
        export_root = round_root / "export"
        try:
            if round_number == 1:
                input_source = round_root / "input" / "source"
                _copy_regular_tree(private_source, input_source)
                if artifact_kind:
                    input_descriptor = input_source.joinpath(
                        *PurePosixPath(item["sourceOwnerRelativePaths"][0]).parts
                    )
                    round_artifact_input = (
                        input_descriptor,
                        input_descriptor.with_suffix(""),
                    )
                else:
                    round_artifact_input = None
            elif artifact_kind:
                input_source = round_root / "input" / "source"
                input_descriptor, input_content = _copy_artifact_pair(
                    exports[0], input_source / "export.xml"
                )
                round_artifact_input = (input_descriptor, input_content)
            else:
                input_source = round_root / "input" / "source"
                _copy_regular_tree(exports[0], input_source)
                round_artifact_input = None
            round_base_source = None
            if private_base_source is not None:
                round_base_source = round_root / "input" / "base"
                _copy_regular_tree(private_base_source, round_base_source)
        except SourceError as error:
            raise checkpoint_error(error, round_number, "round-input-copy") from error
        try:
            round_input_snapshot = (
                capture_artifact_xml_snapshot(*round_artifact_input)
                if round_artifact_input is not None
                else capture_directory_xml_snapshot(input_source)
            )
            expected_input_snapshot = (
                source_snapshot if round_number == 1 else export_snapshots[0]
            )
            if _snapshot_raw_identity(round_input_snapshot) != _snapshot_raw_identity(
                expected_input_snapshot
            ):
                raise SourceError(
                    f"round {round_number} input differs from its immutable "
                    f"checkpoint snapshot for {item['id']}"
                )
            round_input_snapshots.append(round_input_snapshot)
            retained_file_evidence.append(
                (
                    f"round{round_number}Input",
                    round_input_snapshot,
                    "artifact" if round_artifact_input is not None else "directory",
                    round_artifact_input or input_source,
                )
            )
            if round_base_source is not None:
                round_base_snapshot = capture_directory_xml_snapshot(round_base_source)
                if _snapshot_raw_identity(
                    round_base_snapshot
                ) != _snapshot_raw_identity(base_snapshot):
                    raise SourceError(
                        f"round {round_number} base input differs from its immutable "
                        f"checkpoint snapshot for {item['id']}"
                    )
                round_base_input_snapshots.append(round_base_snapshot)
                retained_file_evidence.append(
                    (
                        f"round{round_number}BaseInput",
                        round_base_snapshot,
                        "directory",
                        round_base_source,
                    )
                )
        except SourceError as error:
            raise checkpoint_error(error, round_number, "round-input-snapshot") from error

        def require_command_input_identity(stage: str) -> None:
            if stage in {"import-apply", "extension-import", "artifact-import"}:
                current = (
                    capture_artifact_xml_snapshot(*round_artifact_input)
                    if round_artifact_input is not None
                    else capture_directory_xml_snapshot(input_source)
                )
                expected = round_input_snapshots[round_number - 1]
                label = "source"
            elif stage == "base-import-apply":
                current = capture_directory_xml_snapshot(round_base_source)
                expected = round_base_input_snapshots[round_number - 1]
                label = "base"
            else:
                return
            if _snapshot_raw_identity(current) != _snapshot_raw_identity(expected):
                raise SourceError(
                    f"round {round_number} {label} input changed at consuming "
                    f"stage {stage} for {item['id']}"
                )

        try:
            round_commands = build_checkpoint_round_commands(
                Path(ibcmd),
                kind,
                round_root,
                input_source,
                export_root,
                base_source=round_base_source,
            )
        except SourceError as error:
            raise checkpoint_error(error, round_number, "command-build") from error
        for command in round_commands:
            try:
                require_executable_identity(Path(ibcmd), pinned_ibcmd_sha256)
                require_command_input_identity(command["stage"])
                record = runner.run(
                    command["argv"], cwd=round_root, redactions=redactions
                )
            except SourceError as error:
                raise checkpoint_error(error, round_number, command["stage"]) from error
            record = {"round": round_number, "stage": command["stage"], **record}
            commands.append(record)
            try:
                require_command_input_identity(command["stage"])
                require_executable_identity(Path(ibcmd), pinned_ibcmd_sha256)
            except SourceError as error:
                raise checkpoint_error(
                    error,
                    round_number,
                    f"{command['stage']}-input-integrity",
                ) from error
            if record["exitCode"] != 0:
                return {
                    "id": item["id"],
                    "toolId": item.get("toolId"),
                    "kind": kind,
                    "coveredCaseIds": item["checkpoint"]["coveredCaseIds"],
                    "verdict": "rejected",
                    "failedRound": round_number,
                    "failedStage": command["stage"],
                    "commands": commands,
                    "commandCount": len(commands),
                    "durationMs": sum(entry["durationMs"] for entry in commands),
                    "sourceComparison": None,
                    "roundtripComparison": None,
                    "evidenceSha256": evidence_hashes(),
                    "ownerVersions": {"source": EXPECTED_EXPORT_VERSION},
                }
        if artifact_kind:
            export_descriptor = export_root.with_suffix(".xml")
            if export_descriptor.is_symlink() or not export_descriptor.is_file():
                error = SourceError(
                    f"platform command completed without artifact descriptor for {item['id']} "
                    f"round {round_number}"
                )
                raise checkpoint_error(
                    error, round_number, "artifact-export-evidence"
                )
            if export_root.is_symlink() or not export_root.is_dir():
                error = SourceError(
                    f"platform command completed without artifact content directory for "
                    f"{item['id']} round {round_number}"
                )
                raise checkpoint_error(
                    error, round_number, "artifact-export-evidence"
                )
            export_pair = (export_descriptor, export_root)
            exports.append(export_pair)
            try:
                export_snapshot = capture_artifact_xml_snapshot(*export_pair)
                export_snapshots.append(export_snapshot)
                retained_file_evidence.append(
                    (
                        f"export{round_number}",
                        export_snapshot,
                        "artifact",
                        export_pair,
                    )
                )
            except SourceError as error:
                raise checkpoint_error(
                    error, round_number, "artifact-export-snapshot"
                ) from error
        else:
            if not export_root.is_dir():
                error = SourceError(
                    f"platform command completed without export directory for {item['id']} "
                    f"round {round_number}"
                )
                raise checkpoint_error(error, round_number, "export-evidence")
            exports.append(export_root)
            try:
                export_snapshot = capture_directory_xml_snapshot(export_root)
                export_snapshots.append(export_snapshot)
                retained_file_evidence.append(
                    (
                        f"export{round_number}",
                        export_snapshot,
                        "directory",
                        export_root,
                    )
                )
            except SourceError as error:
                raise checkpoint_error(error, round_number, "export-snapshot") from error

    try:
        for label, expected_snapshot, source_kind, source_location in retained_file_evidence:
            current_snapshot = (
                capture_artifact_xml_snapshot(*source_location)
                if source_kind == "artifact"
                else capture_directory_xml_snapshot(source_location)
            )
            if _snapshot_raw_identity(current_snapshot) == _snapshot_raw_identity(
                expected_snapshot
            ):
                continue
            raise SourceError(
                f"retained file evidence {label} changed after capture for {item['id']}"
            )
    except SourceError as error:
        raise checkpoint_error(error, 2, "retained-evidence") from error

    try:
        source_comparison = compare_xml_snapshots(source_snapshot, export_snapshots[0])
        roundtrip_comparison = compare_xml_snapshots(
            export_snapshots[0], export_snapshots[1]
        )
        if artifact_kind:
            export_owner_evidence = [
                _artifact_snapshot_owner_versions(snapshot)
                for snapshot in export_snapshots
            ]
        else:
            export_owner_evidence = [
                _snapshot_owner_versions(
                    snapshot, item["sourceOwnerRelativePaths"]
                )
                for snapshot in export_snapshots
            ]
    except SourceError as error:
        raise checkpoint_error(error, 2, "semantic-compare") from error
    round1_owners_valid = all(entry["valid"] for entry in export_owner_evidence[0])
    round2_owners_valid = all(entry["valid"] for entry in export_owner_evidence[1])
    if not roundtrip_comparison["equal"] or not round2_owners_valid:
        verdict = "unstable-roundtrip"
    elif not source_comparison["equal"] or not round1_owners_valid:
        verdict = "accepted-normalized"
    else:
        verdict = "pass"
    return {
        "id": item["id"],
        "toolId": item.get("toolId"),
        "kind": kind,
        "coveredCaseIds": item["checkpoint"]["coveredCaseIds"],
        "verdict": verdict,
        "failedRound": None,
        "failedStage": None,
        "commands": commands,
        "commandCount": len(commands),
        "durationMs": sum(entry["durationMs"] for entry in commands),
        "sourceComparison": source_comparison,
        "roundtripComparison": roundtrip_comparison,
        "evidenceSha256": evidence_hashes(),
        "ownerVersions": {
            "source": EXPECTED_EXPORT_VERSION,
            "export1": export_owner_evidence[0],
            "export2": export_owner_evidence[1],
        },
    }


def snapshot_regular_tree(root: Path) -> dict:
    """Hash every regular corpus file and bind empty-directory topology."""
    root = Path(root)
    if not root.is_dir() or root.is_symlink():
        raise SourceError(f"snapshot root is not a safe directory: {root}")
    result: dict[str, str] = {}
    empty_directory_paths: list[str] = []
    identities: dict[tuple[int, int], str] = {}

    def visit(directory: Path) -> None:
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError as error:
            raise SourceError(f"cannot enumerate corpus snapshot {directory}: {error}") from error
        if not entries and directory != root:
            empty_directory_paths.append(directory.relative_to(root).as_posix())
        for entry in entries:
            path = Path(entry.path)
            try:
                if entry.is_symlink():
                    raise SourceError(f"symlink is forbidden in corpus snapshot: {path}")
                if entry.is_dir(follow_symlinks=False):
                    visit(path)
                elif entry.is_file(follow_symlinks=False):
                    relative = path.relative_to(root).as_posix()
                    metadata = entry.stat(follow_symlinks=False)
                    if metadata.st_nlink != 1:
                        raise CorpusError(
                            "corpus regular file has a hardlink/external alias "
                            f"(link count {metadata.st_nlink}): {relative}"
                        )
                    identity = (metadata.st_dev, metadata.st_ino)
                    previous = identities.get(identity)
                    if previous is not None:
                        raise CorpusError(
                            "one file identity is exposed through multiple corpus paths "
                            f"(hardlink replay): {previous}, {relative}"
                        )
                    identities[identity] = relative
                    payload = _read_regular_payload(
                        path, metadata, "corpus tree snapshot"
                    )
                    result[relative] = hashlib.sha256(payload).hexdigest()
                else:
                    raise SourceError(f"special filesystem entry is forbidden: {path}")
            except OSError as error:
                raise SourceError(f"cannot inspect corpus snapshot entry {path}: {error}") from error

    visit(root)
    return {
        "files": result,
        "emptyDirectoryPaths": sorted(empty_directory_paths),
    }


def _paths_overlap(left: Path, right: Path) -> bool:
    return _is_relative_to(left, right) or _is_relative_to(right, left)


def validate_evidence_directory(
    evidence_dir: Path, repo_root: Path, home_root: Path, corpus_root: Path
) -> Path:
    raw = Path(evidence_dir)
    if not raw.is_absolute():
        raise SourceError("evidence directory must be absolute")
    if raw.is_symlink():
        raise SourceError("evidence directory must not be a symlink")
    try:
        target = raw.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"evidence directory must exist: {error}") from error
    if not target.is_dir():
        raise SourceError("evidence target must be a directory")
    repo = Path(repo_root).resolve()
    home = Path(home_root).resolve()
    corpus = Path(corpus_root).resolve()
    filesystem_root = Path(target.anchor).resolve()
    if (
        target == filesystem_root
        or target == home
        or _paths_overlap(target, repo)
        or _paths_overlap(target, corpus)
        or _is_relative_to(home, target)
    ):
        raise SourceError("evidence directory is a broad, home, repository, or corpus path")
    try:
        with os.scandir(target) as entries:
            if next(entries, None) is not None:
                raise SourceError("evidence directory must be empty")
    except OSError as error:
        raise SourceError(f"cannot inspect evidence directory: {error}") from error
    return target


def validate_report_path(
    report_path: Path, repo_root: Path, home_root: Path, corpus_root: Path
) -> Path:
    raw = Path(report_path)
    if not raw.is_absolute():
        raise SourceError("report path must be absolute")
    if raw.is_symlink():
        raise SourceError("report path must not be a symlink")
    parent = raw.parent
    try:
        parent = parent.resolve(strict=True)
    except OSError as error:
        raise SourceError(f"report parent directory must exist: {error}") from error
    if not parent.is_dir():
        raise SourceError("report parent is not a directory")
    target = parent / raw.name
    if target.exists() and not target.is_file():
        raise SourceError("report target exists and is not a regular file")
    repo = Path(repo_root).resolve()
    home = Path(home_root).resolve()
    corpus = Path(corpus_root).resolve()
    filesystem_root = Path(parent.anchor).resolve()
    if (
        parent == filesystem_root
        or parent == home
        or _is_relative_to(target, repo)
        or _is_relative_to(target, corpus)
        or _is_relative_to(home, parent)
    ):
        raise SourceError("report path is inside a broad, home, repository, or corpus path")
    return target


def checkpoint_evidence_path(evidence_dir: Path, case_id: str) -> Path:
    if not isinstance(case_id, str) or CASE_ID_RE.fullmatch(case_id) is None:
        raise SourceError("checkpoint id must be one safe kebab-case filename component")
    try:
        evidence = Path(evidence_dir).resolve(strict=True)
        candidate = evidence / case_id
        resolved = candidate.resolve(strict=False)
    except OSError as error:
        raise SourceError(f"cannot resolve checkpoint evidence path: {error}") from error
    if candidate.parent != evidence or resolved.parent != evidence:
        raise SourceError("checkpoint evidence path must remain a direct child of evidence")
    return candidate


def _tree_delta(before: dict, after: dict) -> dict[str, list[str]]:
    before_files = before["files"]
    after_files = after["files"]
    before_paths = set(before_files)
    after_paths = set(after_files)
    before_directories = set(before["emptyDirectoryPaths"])
    after_directories = set(after["emptyDirectoryPaths"])
    return {
        "added": sorted(after_paths - before_paths),
        "removed": sorted(before_paths - after_paths),
        "modified": sorted(
            path
            for path in before_paths & after_paths
            if before_files[path] != after_files[path]
        ),
        "addedDirectories": sorted(after_directories - before_directories),
        "removedDirectories": sorted(before_directories - after_directories),
    }


def _snapshot_digest(snapshot: dict) -> str:
    payload = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _atomic_write_report(path: Path, report: dict) -> None:
    payload = (json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )
    temporary_name = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb", dir=path.parent, prefix=f".{path.name}.", delete=False
        ) as stream:
            temporary_name = Path(stream.name)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_name, path)
    except OSError as error:
        if temporary_name is not None:
            try:
                temporary_name.unlink(missing_ok=True)
            except OSError:
                pass
        raise SourceError(f"cannot write platform report: {error}") from error


def _cleanup_temporary_directory(temporary) -> None:
    try:
        temporary.cleanup()
    except OSError as error:
        raise SourceError(f"cannot clean temporary evidence directory: {error}") from error


def report_exit_code(report: dict) -> int:
    status = report.get("status")
    if status == "pass":
        return 0
    if status == "failed":
        return 1
    if status == "source-error":
        return 2
    raise SourceError(f"unknown report status: {status!r}")


def _build_gate_report(
    corpus: dict,
    manifest_sha256: str,
    platform: dict | None,
    platform_install: dict,
    checkpoints: list[dict],
    processed: list[str],
    before_snapshot: dict[str, str],
    after_snapshot: dict[str, str] | None,
    source_error: dict | None,
) -> dict:
    selected_ids = [item["id"] for item in corpus["selected"]]
    counts = {
        verdict: sum(item["verdict"] == verdict for item in checkpoints)
        for verdict in (
            "pass",
            "rejected",
            "accepted-normalized",
            "unstable-roundtrip",
            "source-error",
        )
    }
    if source_error is not None:
        status = "source-error"
    elif counts["pass"] == len(selected_ids):
        status = "pass"
    else:
        status = "failed"
    version_duration = (
        platform["command"]["durationMs"]
        if platform is not None and platform.get("command") is not None
        else 0
    )
    install_before = platform_install.get("before")
    install_after = platform_install.get("after")
    install_unchanged = (
        install_before is not None
        and install_after is not None
        and install_before["sha256"] == install_after["sha256"]
        and install_before["fileCount"] == install_after["fileCount"]
    )
    install_verified = (
        install_unchanged
        and install_before["sha256"] == platform_install["expectedSha256"]
        and install_before["fileCount"] == platform_install["expectedFileCount"]
        and install_after["sha256"] == platform_install["expectedSha256"]
        and install_after["fileCount"] == platform_install["expectedFileCount"]
    )
    return {
        "schemaVersion": 1,
        "profile": corpus["profile"],
        "platformVersion": platform.get("version") if platform else None,
        "expectedPlatformVersion": EXPECTED_PLATFORM_VERSION,
        "observedPlatformVersion": platform.get("version") if platform else None,
        "status": status,
        "comparisonPolicy": {
            "xml": "QName-aware semantic equality with only documented lexical normalization",
            "nonXml": "exact logical path and byte equality",
            "directories": "exact empty-directory logical path equality",
            "cycles": 2,
            "excludedXml": [
                "platform-owned root ConfigDumpInfo.xml in directory exports only"
            ],
        },
        "provenance": {
            "corpusManifestSha256": manifest_sha256,
            "caseContractSha256": corpus["caseContractSha256"],
            "expectedCaseContractSha256": EXPECTED_CASE_CONTRACT_SHA256,
            "expectedIbcmdSha256": EXPECTED_IBCMD_SHA256,
            "ibcmdSha256": platform.get("ibcmdSha256") if platform else None,
            "versionCheck": platform.get("command") if platform else None,
            "platformInstall": {
                "expectedSha256": platform_install["expectedSha256"],
                "expectedFileCount": platform_install["expectedFileCount"],
                "beforeSha256": (
                    install_before["sha256"] if install_before is not None else None
                ),
                "beforeFileCount": (
                    install_before["fileCount"] if install_before is not None else None
                ),
                "beforeDirectoryCount": (
                    install_before["directoryCount"]
                    if install_before is not None
                    else None
                ),
                "afterSha256": (
                    install_after["sha256"] if install_after is not None else None
                ),
                "afterFileCount": (
                    install_after["fileCount"] if install_after is not None else None
                ),
                "afterDirectoryCount": (
                    install_after["directoryCount"]
                    if install_after is not None
                    else None
                ),
                "unchanged": install_unchanged,
                "verified": install_verified,
            },
        },
        "coverage": {
            "selectedCaseIds": selected_ids,
            "processedCaseIds": processed,
            "unprocessedCaseIds": [case_id for case_id in selected_ids if case_id not in processed],
            "notSelectedCaseIds": corpus["notSelected"],
            "checkpointMapping": [
                {
                    "checkpointId": item["id"],
                    "kind": item["checkpoint"]["kind"],
                    "coveredCaseIds": item["checkpoint"]["coveredCaseIds"],
                }
                for item in corpus["selected"]
            ],
        },
        "corpusIntegrity": {
            "fileCount": len(before_snapshot["files"]),
            "emptyDirectoryCount": len(before_snapshot["emptyDirectoryPaths"]),
            "beforeSha256": _snapshot_digest(before_snapshot),
            "afterSha256": (
                _snapshot_digest(after_snapshot) if after_snapshot is not None else None
            ),
            "unchanged": (
                before_snapshot == after_snapshot if after_snapshot is not None else False
            ),
            "verified": after_snapshot is not None,
        },
        "checkpoints": checkpoints,
        "summary": {
            **counts,
            "selected": len(selected_ids),
            "processed": len(processed),
            "commandCount": sum(item["commandCount"] for item in checkpoints),
            "versionCommandCount": (
                1 if platform is not None and platform.get("command") is not None else 0
            ),
            "durationMs": version_duration
            + sum(item["durationMs"] for item in checkpoints),
        },
        "sourceError": source_error,
    }


def execute_gate(
    *,
    ibcmd: Path,
    corpus_manifest: Path,
    report_path: Path,
    evidence_dir: Path | None,
    timeout_seconds: float,
    repo_root: Path | None = None,
    home_root: Path | None = None,
    mandatory_case_ids=MANDATORY_CASE_IDS,
    runner=None,
    expected_platform_install_sha256: str | None = None,
    expected_platform_install_file_count: int | None = None,
) -> tuple[int, dict]:
    repo = Path(repo_root or Path(__file__).resolve().parents[2]).resolve()
    home = Path(home_root or Path.home()).resolve()
    corpus = load_corpus(
        Path(corpus_manifest),
        repo_root=repo,
        home_root=home,
        mandatory_case_ids=mandatory_case_ids,
    )
    try:
        platform_install_root = _platform_install_root(Path(ibcmd))
    except SourceError:
        # The normal reported source-error path below retains invalid/missing
        # ibcmd evidence. Overlap checks are meaningful only for a safe root.
        platform_install_root = None
    report_target = validate_report_path(report_path, repo, home, corpus["root"])
    if platform_install_root is not None and _is_relative_to(
        report_target, platform_install_root
    ):
        raise SourceError("report path must be outside the platform install root")
    before_snapshot = corpus["snapshot"]
    current_snapshot = snapshot_regular_tree(corpus["root"])
    if current_snapshot != before_snapshot:
        delta = _tree_delta(before_snapshot, current_snapshot)
        raise SourceError(
            "corpus changed after validation and before platform execution; "
            f"added={delta['added']}, removed={delta['removed']}, "
            f"modified={delta['modified']}, "
            f"addedDirectories={delta['addedDirectories']}, "
            f"removedDirectories={delta['removedDirectories']}"
        )
    manifest_sha256 = corpus["manifestSha256"]
    command_runner = runner or CommandRunner(timeout_seconds=timeout_seconds)
    platform_install = {
        "expectedSha256": (
            EXPECTED_PLATFORM_INSTALL_SHA256
            if expected_platform_install_sha256 is None
            else expected_platform_install_sha256
        ),
        "expectedFileCount": (
            EXPECTED_PLATFORM_INSTALL_FILE_COUNT
            if expected_platform_install_file_count is None
            else expected_platform_install_file_count
        ),
        "before": None,
        "after": None,
    }

    temporary = None
    if evidence_dir is None:
        try:
            temporary = tempfile.TemporaryDirectory(prefix="unica-8-3-27-platform-")
        except OSError as error:
            raise SourceError(
                f"cannot create temporary evidence directory: {error}"
            ) from error
        evidence = Path(temporary.name).resolve()
        try:
            evidence = validate_evidence_directory(
                evidence, repo, home, corpus["root"]
            )
        except SourceError:
            _cleanup_temporary_directory(temporary)
            raise
    else:
        evidence = validate_evidence_directory(
            Path(evidence_dir), repo, home, corpus["root"]
        )
    if platform_install_root is not None and _paths_overlap(
        evidence, platform_install_root
    ):
        if temporary is not None:
            _cleanup_temporary_directory(temporary)
        raise SourceError("evidence directory must be outside the platform install root")
    control = evidence / "control"
    checkpoints: list[dict] = []
    processed: list[str] = []
    platform = None
    source_error = None
    after_snapshot = before_snapshot
    try:
        try:
            control.mkdir()
        except OSError as error:
            raise SourceError(f"cannot create platform control directory: {error}") from error
        try:
            platform_install["before"] = verify_platform_install_inventory(
                Path(ibcmd),
                expected_sha256=platform_install["expectedSha256"],
                expected_file_count=platform_install["expectedFileCount"],
            )
        except PlatformInstallError as error:
            platform_install["before"] = error.inventory
            raise
        platform = verify_platform_version(
            Path(ibcmd),
            command_runner,
            control,
            redactions=[(evidence, "$EVIDENCE"), (Path(ibcmd), "$IBCMD")],
        )
        for item in corpus["selected"]:
            result = run_checkpoint(
                item,
                Path(ibcmd),
                command_runner,
                checkpoint_evidence_path(evidence, item["id"]),
                corpus["root"],
                pinned_ibcmd_sha256=platform["ibcmdSha256"],
            )
            checkpoints.append(result)
            processed.append(item["id"])
            current_snapshot = snapshot_regular_tree(corpus["root"])
            if current_snapshot != before_snapshot:
                after_snapshot = current_snapshot
                delta = _tree_delta(before_snapshot, current_snapshot)
                source_error = {
                    "code": "corpus-mutated",
                    "message": "platform processing changed the read-only corpus",
                    **delta,
                }
                break
        require_executable_identity(Path(ibcmd), platform["ibcmdSha256"])
    except PlatformInstallError as error:
        source_error = {
            "code": "platform-install-mismatch",
            "message": str(error),
            "added": [],
            "removed": [],
            "modified": [],
        }
    except PlatformVersionError as error:
        platform = {
            "version": error.version,
            "command": error.record,
            "ibcmdSha256": error.ibcmd_sha256,
        }
        source_error = {
            "code": "platform-version-mismatch",
            "message": str(error),
            "added": [],
            "removed": [],
            "modified": [],
        }
    except PlatformBinaryError as error:
        platform = {
            "version": None,
            "command": None,
            "ibcmdSha256": error.ibcmd_sha256,
        }
        source_error = {
            "code": "platform-binary-mismatch",
            "message": str(error),
            "added": [],
            "removed": [],
            "modified": [],
        }
    except CheckpointExecutionError as error:
        checkpoints.append(error.checkpoint)
        source_error = {
            "code": "platform-source-error",
            "message": _redact_text(
                str(error),
                [
                    (evidence, "$EVIDENCE"),
                    (corpus["root"], "$CORPUS"),
                    (Path(ibcmd), "$IBCMD"),
                ],
            ),
            "added": [],
            "removed": [],
            "modified": [],
        }
    except SourceError as error:
        source_error = {
            "code": "platform-source-error",
            "message": _redact_text(
                str(error),
                [
                    (evidence, "$EVIDENCE"),
                    (corpus["root"], "$CORPUS"),
                    (Path(ibcmd), "$IBCMD"),
                ],
            ),
            "added": [],
            "removed": [],
            "modified": [],
        }
    finally:
        try:
            after_snapshot = snapshot_regular_tree(corpus["root"])
        except SourceError as error:
            after_snapshot = None
            source_error = {
                "code": "corpus-unsafe",
                "message": _redact_text(str(error), [(corpus["root"], "$CORPUS")]),
                "added": [],
                "removed": [],
                "modified": [],
            }
        else:
            if after_snapshot != before_snapshot:
                delta = _tree_delta(before_snapshot, after_snapshot)
                source_error = {
                    "code": "corpus-mutated",
                    "message": "platform processing changed the read-only corpus",
                    **delta,
                }
        install_error = None
        try:
            platform_install["after"] = capture_platform_install_inventory(Path(ibcmd))
        except SourceError as error:
            install_error = {
                "code": "platform-install-unsafe",
                "message": _redact_text(str(error), [(Path(ibcmd), "$IBCMD")]),
                "added": [],
                "removed": [],
                "modified": [],
            }
        else:
            install_before = platform_install["before"]
            install_after = platform_install["after"]
            if install_before is not None and (
                install_before["sha256"] != install_after["sha256"]
                or install_before["fileCount"] != install_after["fileCount"]
            ):
                install_error = {
                    "code": "platform-install-mutated",
                    "message": (
                        "platform install inventory changed while the gate was running; "
                        f"before sha256={install_before['sha256']}, "
                        f"files={install_before['fileCount']}; "
                        f"after sha256={install_after['sha256']}, "
                        f"files={install_after['fileCount']}"
                    ),
                    "added": [],
                    "removed": [],
                    "modified": [],
                }
            elif (
                install_after["sha256"] != platform_install["expectedSha256"]
                or install_after["fileCount"] != platform_install["expectedFileCount"]
            ):
                install_error = {
                    "code": "platform-install-mismatch",
                    "message": (
                        "final platform install inventory does not match the pinned "
                        "8.3.27.2074 closure"
                    ),
                    "added": [],
                    "removed": [],
                    "modified": [],
                }
        if install_error is not None:
            if source_error is None:
                source_error = install_error
            elif source_error.get("code") != install_error["code"]:
                source_error["platformInstallError"] = install_error

    if source_error is None and len(processed) != len(corpus["selected"]):
        source_error = {
            "code": "incomplete-processing",
            "message": "not every selected checkpoint was processed",
            "added": [],
            "removed": [],
            "modified": [],
        }
    report = _build_gate_report(
        corpus,
        manifest_sha256,
        platform,
        platform_install,
        checkpoints,
        processed,
        before_snapshot,
        after_snapshot,
        source_error,
    )
    if temporary is not None:
        try:
            _cleanup_temporary_directory(temporary)
        except SourceError as error:
            cleanup_error = {
                "code": "evidence-cleanup-failed",
                "message": str(error),
                "added": [],
                "removed": [],
                "modified": [],
            }
            if source_error is not None:
                cleanup_error["precedingSourceError"] = source_error
            report = _build_gate_report(
                corpus,
                manifest_sha256,
                platform,
                platform_install,
                checkpoints,
                processed,
                before_snapshot,
                after_snapshot,
                cleanup_error,
            )
    _atomic_write_report(report_target, report)
    return report_exit_code(report), report


def _positive_float(value: str) -> float:
    try:
        parsed = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be a number") from error
    if not math.isfinite(parsed) or parsed <= 0:
        raise argparse.ArgumentTypeError("must be finite and greater than zero")
    return parsed


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Round-trip the fixed-profile export corpus through exactly "
            "1C 8.3.27.2074, comparing XML semantics and non-XML bytes."
        )
    )
    parser.add_argument("--ibcmd", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument(
        "--timeout",
        type=_positive_float,
        default=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        help="per-command timeout in seconds (default: 300)",
    )
    return parser


def main(argv=None) -> int:
    arguments = _argument_parser().parse_args(argv)
    try:
        exit_code, _report = execute_gate(
            ibcmd=arguments.ibcmd,
            corpus_manifest=arguments.corpus,
            report_path=arguments.report,
            evidence_dir=arguments.evidence_dir,
            timeout_seconds=arguments.timeout,
        )
    except SourceError as error:
        print(f"source error: {error}", file=sys.stderr)
        return 2
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
