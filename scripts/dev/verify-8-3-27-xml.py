#!/usr/bin/env python3
"""Verify Unica XML against the approved 1C 8.3.27 evidence profile."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import zipfile
import zlib
from pathlib import Path, PurePosixPath, PureWindowsPath

from lxml import etree

try:
    from scripts.dev.xml_lexical import LexicalXmlError, raw_root_attribute
except ModuleNotFoundError:
    from xml_lexical import LexicalXmlError, raw_root_attribute


XS_NS = "http://www.w3.org/2001/XMLSchema"
XSI_NS = "http://www.w3.org/2001/XMLSchema-instance"
CORE_NS = "http://v8.1c.ru/8.1/data/core"
MD_CLASSES_NS = "http://v8.1c.ru/8.3/MDClasses"
QNAME_TEXT_ELEMENTS = {
    f"{{{CORE_NS}}}Type",
    f"{{{CORE_NS}}}TypeSet",
    f"{{{MD_CLASSES_NS}}}XDTOReturningValueType",
    f"{{{MD_CLASSES_NS}}}XDTOValueType",
}
REPORT_SCHEMA_NAME = "verify-8-3-27-xml-report.schema.json"
MANIFEST_SCHEMA_URI_PREFIX = "unica-manifest-xsd://schema/"
BLOCKED_SCHEMA_URI_PREFIX = "unica-blocked-xsd://dependency/"
FIXED_PROFILE = "1c-8.3.27-export-2.20"
_CANONICAL_CORPUS_VERIFIER = None


class SourceError(RuntimeError):
    """Approved schema evidence is absent, altered, ambiguous, or unusable."""


class CorpusError(RuntimeError):
    """The corpus manifest cannot be trusted or completely processed."""


class RuntimeEvidence(dict):
    def close(self):
        temporary = self.pop("_temporaryDirectory", None)
        if temporary is not None:
            temporary.cleanup()

    def __del__(self):
        self.close()


def _canonical_corpus_verifier():
    global _CANONICAL_CORPUS_VERIFIER
    if _CANONICAL_CORPUS_VERIFIER is not None:
        return _CANONICAL_CORPUS_VERIFIER
    script = Path(__file__).with_name("verify-8-3-27-platform.py")
    spec = importlib.util.spec_from_file_location(
        "unica_verify_8_3_27_platform_contract",
        script,
    )
    if spec is None or spec.loader is None:
        raise SourceError(f"cannot load canonical corpus verifier: {script}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    _CANONICAL_CORPUS_VERIFIER = module
    return module


def _private_copy_with_sha256(source: Path, destination: Path, label: str) -> str:
    """Copy once, while hashing, so all later checks consume the same bytes."""
    digest = hashlib.sha256()
    try:
        with source.open("rb") as input_stream, destination.open("xb") as output_stream:
            for block in iter(lambda: input_stream.read(1024 * 1024), b""):
                digest.update(block)
                output_stream.write(block)
    except OSError as error:
        raise SourceError(f"cannot read {label}: {source}: {error}") from error
    return digest.hexdigest()


class _ManifestOnlyResolver(etree.Resolver):
    """Resolve schema dependencies only through verified temporary copies."""

    def __init__(self, locations: dict[str, Path], blocked: set[str]):
        super().__init__()
        self.locations = locations
        self.blocked = blocked
        self.path_locations = {path.resolve(): uri for uri, path in locations.items()}

    def uri_for(self, path: Path) -> str:
        try:
            return self.path_locations[path.resolve()]
        except KeyError as error:
            raise SourceError(f"schema is not registered in verified manifest: {path}") from error

    def resolve(self, url, _public_id, context):
        if url in self.locations:
            return self.resolve_filename(str(self.locations[url]), context)
        if url in self.blocked:
            return self.resolve_string(b"<blocked-external-schema/>", context)
        if url.startswith((MANIFEST_SCHEMA_URI_PREFIX, BLOCKED_SCHEMA_URI_PREFIX)):
            raise OSError(f"unregistered schema URI: {url}")
        raise OSError(f"schema access outside verified manifest is forbidden: {url}")


def _schema_parser(resolver: _ManifestOnlyResolver | None = None) -> etree.XMLParser:
    parser = etree.XMLParser(resolve_entities=False, load_dtd=False, no_network=True)
    if resolver is not None:
        parser.resolvers.add(resolver)
    return parser


def _safe_zip_members(archive: zipfile.ZipFile) -> list[zipfile.ZipInfo]:
    infos = archive.infolist()
    names = [info.filename for info in infos]
    if len(names) != len(set(names)):
        raise SourceError("duplicate ZIP member")
    for info in infos:
        path = PurePosixPath(info.filename)
        windows_path = PureWindowsPath(info.filename)
        if (
            path.is_absolute()
            or windows_path.is_absolute()
            or windows_path.drive
            or ".." in path.parts
            or ".." in windows_path.parts
            or "\\" in info.filename
            or not path.parts
        ):
            raise SourceError(f"unsafe ZIP path: {info.filename}")
        mode = info.external_attr >> 16
        if stat.S_ISLNK(mode):
            raise SourceError(f"ZIP symlink is forbidden: {info.filename}")
    return infos


def _read_zip_member(archive: zipfile.ZipFile, name: str, label: str) -> bytes:
    try:
        return archive.read(name)
    except (
        KeyError,
        OSError,
        RuntimeError,
        EOFError,
        NotImplementedError,
        zipfile.BadZipFile,
        zipfile.LargeZipFile,
        zlib.error,
    ) as error:
        raise SourceError(f"cannot read {label} ZIP member {name}: {error}") from error


def _manifest_member(infos: list[zipfile.ZipInfo]) -> str:
    matches = [info.filename for info in infos if PurePosixPath(info.filename).name == "manifest.json"]
    if len(matches) != 1:
        raise SourceError(f"runtime archive must contain exactly one manifest.json, found {len(matches)}")
    return matches[0]


def _schema_target_namespace(payload: bytes, label: str) -> str:
    try:
        root = etree.fromstring(payload, parser=etree.XMLParser(resolve_entities=False, no_network=True))
    except etree.XMLSyntaxError as error:
        raise SourceError(f"invalid XSD {label}: {error}") from error
    if root.tag != f"{{{XS_NS}}}schema":
        raise SourceError(f"XSD root is not xs:schema: {label}")
    namespace = root.get("targetNamespace")
    if not namespace:
        raise SourceError(f"XSD has no targetNamespace: {label}")
    return namespace


def _expected_runtime(profile: dict) -> dict:
    runtime = profile.get("runtime")
    if not isinstance(runtime, dict):
        raise SourceError("profile has no runtime contract")
    return runtime


def _is_nonempty_string(value) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_sha256(value) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def _is_json_integer(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _validate_runtime_contract(runtime: dict) -> None:
    if not _is_sha256(runtime.get("sha256")):
        raise SourceError("runtime profile sha256 must be a lowercase SHA-256")
    format_version = runtime.get("formatVersion")
    if not _is_json_integer(format_version) or format_version < 0:
        raise SourceError("runtime profile formatVersion must be a non-negative integer")
    if not _is_nonempty_string(runtime.get("platformVersion")):
        raise SourceError("runtime profile platformVersion must be a non-empty string")
    summary = runtime.get("summary")
    count_names = {"packages", "namespaces", "success", "schemas", "errors"}
    if not isinstance(summary, dict) or set(summary) != count_names:
        raise SourceError("runtime profile summary must contain exactly the five count fields")
    if any(
        not isinstance(summary[name], int)
        or isinstance(summary[name], bool)
        or summary[name] < 0
        for name in count_names
    ):
        raise SourceError("runtime profile summary counts must be non-negative integers")
    known_failures = runtime.get("knownCompileFailures")
    if not isinstance(known_failures, dict) or any(
        not _is_nonempty_string(name) or not _is_nonempty_string(reason)
        for name, reason in known_failures.items()
    ):
        raise SourceError("runtime profile knownCompileFailures must be a string map")
    external_imports = runtime.get("knownExternalImports", [])
    if not isinstance(external_imports, list):
        raise SourceError("runtime profile knownExternalImports must be a list")
    seen_external = set()
    for rule in external_imports:
        if not isinstance(rule, dict):
            raise SourceError("runtime profile knownExternalImports rule must be an object")
        key = (rule.get("file"), rule.get("namespace"), rule.get("schemaLocation"))
        if (
            not _is_canonical_relative_path(key[0], suffix=".xsd")
            or not _is_nonempty_string(key[1])
            or not _is_nonempty_string(key[2])
            or key in seen_external
        ):
            raise SourceError("runtime profile knownExternalImports rule is invalid or duplicate")
        seen_external.add(key)


def _validate_edt_contract(edt: dict) -> None:
    if not _is_sha256(edt.get("sha256")):
        raise SourceError("EDT profile sha256 must be a lowercase SHA-256")
    if not _is_nonempty_string(edt.get("symbolicName")) or not _is_nonempty_string(edt.get("version")):
        raise SourceError("EDT profile symbolicName/version must be non-empty strings")
    entries = edt.get("entries")
    if not isinstance(entries, dict) or any(
        not _is_nonempty_string(label) or not _is_canonical_relative_path(name)
        for label, name in entries.items()
    ):
        raise SourceError("EDT profile entries must be a canonical path map")
    declarations = edt.get("declarations")
    if not isinstance(declarations, dict):
        raise SourceError("EDT profile declarations must be an object")
    for label, rule in declarations.items():
        if not _is_nonempty_string(label) or not isinstance(rule, dict):
            raise SourceError("EDT profile declaration rule is invalid")
        entry = rule.get("entry")
        tokens = rule.get("tokens")
        if (
            not _is_nonempty_string(entry)
            or entry not in entries
            or not isinstance(tokens, list)
            or not tokens
            or any(not _is_nonempty_string(token) for token in tokens)
            or len(tokens) != len(set(tokens))
        ):
            raise SourceError(f"EDT profile declaration is invalid: {label}")


def _validate_profile(profile: dict) -> None:
    if not isinstance(profile.get("profile"), str) or not profile["profile"].strip():
        raise SourceError("verification profile id must be a non-empty string")
    if not isinstance(profile.get("exportVersion"), str) or not profile["exportVersion"].strip():
        raise SourceError("verification profile exportVersion must be a non-empty string")
    if not isinstance(profile.get("runtime"), dict) or not isinstance(profile.get("edt"), dict):
        raise SourceError("verification profile runtime/EDT contracts must be objects")
    families = profile.get("families")
    if not isinstance(families, dict) or not families:
        raise SourceError("verification profile families must be a non-empty object")
    family_ids = set()
    owner_families = 0
    owner_references = 0
    edt_references = set()
    for expected_qname, family in families.items():
        if (
            not isinstance(expected_qname, str)
            or re.fullmatch(r"\{[^{}]+\}[^{}]+", expected_qname) is None
            or not isinstance(family, dict)
        ):
            raise SourceError("verification profile family contract is invalid")
        family_id = family.get("id")
        if not isinstance(family_id, str) or not family_id or family_id in family_ids:
            raise SourceError(f"verification profile family id is invalid or duplicate: {family_id}")
        family_ids.add(family_id)
        coverage = family.get("coverage")
        if not isinstance(coverage, str) or coverage not in {"strict", "advisory", "not-covered", "known-schema-incompatibility"}:
            raise SourceError(f"verification profile coverage is invalid: {family_id}")
        version_mode = family.get("version", "none")
        if not isinstance(version_mode, str) or version_mode not in {"none", "root", "owner"}:
            raise SourceError(f"verification profile version mode is invalid: {family_id}")
        owner_references += version_mode == "owner"
        if family.get("coverage") == "strict" and (
            not _is_nonempty_string(family.get("schemaNamespace"))
            or not _is_nonempty_string(family.get("wrapperType"))
        ):
            raise SourceError(f"strict verification profile family is incomplete: {family_id}")
        if "sourceSetOwner" in family and not isinstance(family["sourceSetOwner"], bool):
            raise SourceError(f"sourceSetOwner must be boolean: {family_id}")
        if "edtDeclaration" in family:
            if not _is_nonempty_string(family["edtDeclaration"]):
                raise SourceError(f"EDT declaration reference is invalid: {family_id}")
            edt_references.add(family["edtDeclaration"])
        if family.get("sourceSetOwner") is True:
            owner_families += 1
            if version_mode != "root" or not expected_qname.endswith("}MetaDataObject"):
                raise SourceError("sourceSetOwner must be a version-bearing MetaDataObject family")
            owner_types = family.get("sourceSetOwnerTypes")
            if (
                not isinstance(owner_types, list)
                or not owner_types
                or any(not isinstance(value, str) or not value for value in owner_types)
                or len(owner_types) != len(set(owner_types))
            ):
                raise SourceError("sourceSetOwnerTypes must be a non-empty list of unique names")
    if owner_references and owner_families != 1:
        raise SourceError("verification profile must define exactly one sourceSetOwner family")
    _validate_runtime_contract(profile["runtime"])
    _validate_edt_contract(profile["edt"])
    missing_declarations = edt_references - set(profile["edt"]["declarations"])
    if missing_declarations:
        raise SourceError(f"verification profile references missing EDT declaration: {sorted(missing_declarations)[0]}")


def verified_runtime(path: Path | str, profile: dict) -> dict:
    path = Path(path)
    expected = _expected_runtime(profile)
    _validate_runtime_contract(expected)
    temporary = tempfile.TemporaryDirectory(prefix="unica-8-3-27-xsd-")
    temp_root = Path(temporary.name)
    try:
        private_archive = temp_root / "runtime-evidence.zip"
        actual_hash = _private_copy_with_sha256(path, private_archive, "runtime XSD archive")
        if actual_hash != expected.get("sha256"):
            raise SourceError(f"runtime archive SHA-256 mismatch: {actual_hash}")
        try:
            archive_context = zipfile.ZipFile(private_archive)
        except (OSError, zipfile.BadZipFile, zipfile.LargeZipFile) as error:
            raise SourceError(f"invalid runtime ZIP archive: {error}") from error
        with archive_context as archive:
            infos = _safe_zip_members(archive)
            manifest_name = _manifest_member(infos)
            try:
                manifest = json.loads(_read_zip_member(archive, manifest_name, "runtime manifest"))
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise SourceError(f"invalid runtime manifest: {error}") from error
            if not isinstance(manifest, dict):
                raise SourceError("runtime manifest root must be an object")
            manifest_format_version = manifest.get("formatVersion")
            if (
                not _is_json_integer(manifest_format_version)
                or manifest_format_version < 0
                or manifest_format_version != expected.get("formatVersion")
            ):
                raise SourceError("runtime manifest formatVersion mismatch")
            if manifest.get("platformVersion") != expected.get("platformVersion"):
                raise SourceError("runtime manifest platformVersion mismatch")
            summary = manifest.get("summary")
            if not isinstance(summary, dict) or summary != expected.get("summary"):
                raise SourceError("runtime manifest summary mismatch")
            count_names = ("packages", "namespaces", "success", "schemas", "errors")
            if any(not isinstance(summary.get(name), int) or isinstance(summary.get(name), bool) or summary[name] < 0 for name in count_names):
                raise SourceError("runtime manifest summary count is invalid")
            declarations = manifest.get("schemas")
            if not isinstance(declarations, list) or not declarations:
                raise SourceError("runtime manifest schemas are missing or empty")
            if len(declarations) != summary["schemas"]:
                raise SourceError("runtime manifest schema count mismatch")
            prefix = str(PurePosixPath(manifest_name).parent)
            prefix = "" if prefix == "." else prefix + "/"
            declared_names = set()
            declared_by_member = {}
            namespace_paths = {}
            schema_rows = []
            for declaration in declarations:
                if not isinstance(declaration, dict):
                    raise SourceError("runtime schema declaration must be an object")
                relative = declaration.get("file")
                if not _is_canonical_relative_path(relative, suffix=".xsd"):
                    raise SourceError("runtime schema declaration has no file")
                pure = PurePosixPath(relative)
                member = prefix + relative
                if member in declared_names:
                    raise SourceError(f"duplicate runtime schema declaration: {relative}")
                declared_names.add(member)
                payload = _read_zip_member(archive, member, "runtime XSD")
                if len(payload) != declaration.get("size"):
                    raise SourceError(f"manifest size mismatch: {relative}")
                if hashlib.sha256(payload).hexdigest() != declaration.get("sha256"):
                    raise SourceError(f"manifest hash mismatch: {relative}")
                namespace = _schema_target_namespace(payload, relative)
                if namespace != declaration.get("targetNamespace"):
                    raise SourceError(f"manifest targetNamespace mismatch: {relative}")
                if namespace in namespace_paths:
                    raise SourceError(f"duplicate targetNamespace: {namespace}")
                target = temp_root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(payload)
                namespace_paths[namespace] = target
                row = {"file": relative, "targetNamespace": namespace}
                schema_rows.append(row)
                declared_by_member[member] = row
            if len(namespace_paths) != summary["namespaces"]:
                raise SourceError("runtime manifest namespace count mismatch")
            exports = manifest.get("exports")
            if not isinstance(exports, list) or len(exports) != summary["packages"]:
                raise SourceError("runtime manifest exports/package count mismatch")
            exported_files = set()
            exported_namespaces = set()
            successful_exports = 0
            failed_exports = 0
            for export in exports:
                if not isinstance(export, dict):
                    raise SourceError("runtime export row must be an object")
                namespace = export.get("sourceNamespace")
                if not isinstance(namespace, str) or namespace in exported_namespaces:
                    raise SourceError(f"duplicate or invalid exported namespace: {namespace}")
                exported_namespaces.add(namespace)
                if export.get("error") not in (None, ""):
                    failed_exports += 1
                    raise SourceError(f"runtime export reports an error for {namespace}")
                successful_exports += 1
                export_files = export.get("files")
                if not isinstance(export_files, list) or not export_files:
                    raise SourceError(f"runtime export files are missing for {namespace}")
                for relative in export_files:
                    if not _is_canonical_relative_path(relative, suffix=".xsd"):
                        raise SourceError(f"unsafe runtime export file: {relative}")
                    member = prefix + relative
                    if member in exported_files:
                        raise SourceError(f"runtime schema is exported more than once: {relative}")
                    declaration = declared_by_member.get(member)
                    if declaration is None:
                        raise SourceError(f"runtime export references undeclared schema: {relative}")
                    if declaration["targetNamespace"] != namespace:
                        raise SourceError(f"runtime export namespace does not match {relative}")
                    exported_files.add(member)
            if successful_exports != summary["success"] or failed_exports != summary["errors"]:
                raise SourceError("runtime export success/error count mismatch")
            if exported_files != declared_names or exported_namespaces != set(namespace_paths):
                raise SourceError("runtime exports do not exactly match declared schemas/namespaces")
            archived_xsd = {info.filename for info in infos if info.filename.lower().endswith(".xsd")}
            if archived_xsd != declared_names:
                difference = sorted(archived_xsd ^ declared_names)
                raise SourceError(f"runtime archive XSD declaration mismatch: {difference[0]}")

        resolver = _resolve_schema_references(
            namespace_paths,
            schema_rows,
            expected.get("knownExternalImports", []),
        )
        matrix = _compile_matrix(
            schema_rows,
            temp_root,
            expected.get("knownCompileFailures", {}),
            resolver,
        )
        if len(matrix) != summary["schemas"]:
            raise SourceError("runtime schema compilation matrix count mismatch")
        strict_namespaces = {
            family.get("schemaNamespace")
            for family in profile.get("families", {}).values()
            if family.get("coverage") == "strict"
        }
        failed = {row["targetNamespace"]: row for row in matrix if row["status"] != "compiled"}
        for namespace in strict_namespaces:
            if namespace not in namespace_paths:
                raise SourceError(f"strict schema namespace is missing: {namespace}")
            if namespace in failed:
                raise SourceError(f"strict schema dependency failed to compile: {namespace}: {failed[namespace]['detail']}")
        wrappers = {}
        for family in profile.get("families", {}).values():
            if family.get("coverage") != "strict":
                continue
            namespace = family["schemaNamespace"]
            wrappers[(namespace, family["wrapperType"])] = _compile_wrapper(
                namespace_paths[namespace], namespace, family["wrapperType"], temp_root, resolver
            )
        return RuntimeEvidence({
            "source": {
                "kind": "runtime-xsd",
                "sha256": actual_hash,
                "formatVersion": manifest["formatVersion"],
                "platformVersion": manifest["platformVersion"],
                "manifestSummary": dict(summary),
                "identityStatement": "Configured SHA-256 proves identity with approved local evidence, not download provenance.",
            },
            "compilationMatrix": matrix,
            "namespacePaths": namespace_paths,
            "wrappers": wrappers,
            "_temporaryDirectory": temporary,
        })
    except Exception:
        temporary.cleanup()
        raise


def _is_canonical_relative_path(raw, suffix: str | None = None) -> bool:
    if (
        not isinstance(raw, str)
        or not raw
        or "\\" in raw
        or any(ord(character) < 0x20 for character in raw)
    ):
        return False
    pure = PurePosixPath(raw)
    windows = PureWindowsPath(raw)
    if pure.is_absolute() or windows.is_absolute() or windows.drive or ".." in pure.parts:
        return False
    if pure.as_posix() != raw or not pure.parts:
        return False
    return suffix is None or pure.suffix.lower() == suffix.lower()


def _schema_tree(path: Path, resolver: _ManifestOnlyResolver | None = None):
    try:
        root = etree.fromstring(
            path.read_bytes(),
            parser=_schema_parser(resolver),
            base_url=path.as_uri(),
        )
    except (OSError, etree.XMLSyntaxError) as error:
        raise SourceError(f"cannot parse temporary schema {path.name}: {error}") from error
    return etree.ElementTree(root)


def _resolve_schema_references(
    namespace_paths: dict[str, Path],
    rows: list[dict],
    known_external_imports,
) -> _ManifestOnlyResolver:
    row_by_path = {
        namespace_paths[row["targetNamespace"]].resolve(): row
        for row in rows
    }
    locations = {}
    namespace_locations = {}
    path_locations = {}
    for namespace, path in namespace_paths.items():
        uri = MANIFEST_SCHEMA_URI_PREFIX + hashlib.sha256(namespace.encode()).hexdigest()
        locations[uri] = path
        namespace_locations[namespace] = uri
        path_locations[path.resolve()] = uri

    if not isinstance(known_external_imports, list):
        raise SourceError("knownExternalImports must be a list")
    configured_external = set()
    for rule in known_external_imports:
        if not isinstance(rule, dict):
            raise SourceError("known external import rule must be an object")
        key = (rule.get("file"), rule.get("namespace"), rule.get("schemaLocation"))
        if not all(isinstance(value, str) and value for value in key) or key in configured_external:
            raise SourceError("invalid or duplicate known external import rule")
        configured_external.add(key)
    observed_external = set()
    blocked = set()
    allowed_paths = set(path_locations)

    for path in namespace_paths.values():
        tree = _schema_tree(path)
        row = row_by_path[path.resolve()]
        for node in tree.findall(f".//{{{XS_NS}}}import"):
            namespace = node.get("namespace")
            if namespace in namespace_locations:
                location = node.get("schemaLocation")
                if location is not None:
                    if not _is_canonical_relative_path(location):
                        raise SourceError(
                            f"external schemaLocation is forbidden in {row['file']}: {location}"
                        )
                    target = (path.parent / location).resolve()
                    if target != namespace_paths[namespace].resolve():
                        raise SourceError(
                            f"import schemaLocation is not the manifest schema for {namespace}: {row['file']} {location}"
                        )
                node.set("schemaLocation", namespace_locations[namespace])
                continue
            key = (row["file"], namespace, node.get("schemaLocation"))
            if key not in configured_external:
                raise SourceError(
                    f"unresolved namespace import is not provided by verified manifest: {row['file']} {namespace!r}"
                )
            blocked_uri = BLOCKED_SCHEMA_URI_PREFIX + hashlib.sha256(repr(key).encode()).hexdigest()
            node.set("schemaLocation", blocked_uri)
            blocked.add(blocked_uri)
            observed_external.add(key)

        for local_name in ("include", "redefine", "override"):
            for node in tree.findall(f".//{{{XS_NS}}}{local_name}"):
                location = node.get("schemaLocation")
                if not _is_canonical_relative_path(location):
                    raise SourceError(
                        f"external schemaLocation is forbidden in {row['file']}: {location}"
                    )
                target = (path.parent / location).resolve()
                if target not in allowed_paths:
                    raise SourceError(
                        f"schema reference is not provided by verified manifest: {row['file']} {location}"
                    )
                node.set("schemaLocation", path_locations[target])
        tree.write(str(path), encoding="UTF-8", xml_declaration=True)

    missing_rules = configured_external - observed_external
    if missing_rules:
        raise SourceError(f"configured known external import was not observed: {sorted(missing_rules)[0]}")
    return _ManifestOnlyResolver(locations, blocked)


def _compile_matrix(
    rows: list[dict],
    root: Path,
    known: dict,
    resolver: _ManifestOnlyResolver,
) -> list[dict]:
    matrix = []
    unexpected = []
    for row in sorted(rows, key=lambda item: item["file"]):
        path = root / row["file"]
        result = dict(row)
        try:
            etree.XMLSchema(_schema_tree(path, resolver))
            result.update(status="compiled", detail="")
        except (SourceError, etree.XMLSchemaParseError, etree.XMLSyntaxError, OSError) as error:
            reason = known.get(PurePosixPath(row["file"]).name) or known.get(row["file"])
            if reason:
                result.update(status="known-source-incompatibility", detail=reason)
            else:
                error_log = getattr(error, "error_log", None)
                detail = str((error_log.last_error if error_log is not None else None) or error).replace(str(root), "<runtime-xsd>")
                result.update(status="compile-error", detail=detail)
                unexpected.append(result)
        matrix.append(result)
    if unexpected:
        first = unexpected[0]
        raise SourceError(f"unexpected XSD compile failure {first['file']}: {first['detail']}")
    return matrix


def _compile_wrapper(
    schema_path: Path,
    namespace: str,
    type_name: str,
    root: Path,
    resolver: _ManifestOnlyResolver,
):
    wrapper = root / f"wrapper-{hashlib.sha256((namespace + type_name).encode()).hexdigest()[:12]}.xsd"
    include = resolver.uri_for(schema_path)
    content = f'''<xs:schema xmlns:xs="{XS_NS}" xmlns:tns="{namespace}" targetNamespace="{namespace}" elementFormDefault="qualified">
<xs:include schemaLocation="{include}"/>
<xs:element name="{type_name}" type="tns:{type_name}"/>
</xs:schema>'''
    wrapper.write_text(content, encoding="utf-8")
    try:
        return etree.XMLSchema(_schema_tree(wrapper, resolver))
    except (SourceError, etree.XMLSchemaParseError, OSError) as error:
        raise SourceError(f"strict schema wrapper failed for {namespace} {type_name}: {error}") from error


def strict_xsd_error(runtime: dict, namespace: str, type_name: str, xml_text: str) -> str | None:
    try:
        document = etree.fromstring(xml_text.encode(), parser=etree.XMLParser(no_network=True, resolve_entities=False))
    except etree.XMLSyntaxError as error:
        return str(error)
    schema = runtime["wrappers"][(namespace, type_name)]
    if schema.validate(document):
        return None
    return str(schema.error_log.last_error or "XSD validation failed")


def _safe_corpus_path(root: Path, raw: str) -> Path:
    if not _is_canonical_relative_path(raw, suffix=".xml"):
        raise CorpusError(f"unsafe corpus path or non-canonical path: {raw}")
    pure = PurePosixPath(raw)
    candidate = root
    for part in pure.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise CorpusError(f"corpus symlink is forbidden: {raw}")
    try:
        candidate.resolve(strict=True).relative_to(root.resolve(strict=True))
    except (OSError, ValueError) as error:
        raise CorpusError(f"corpus path escapes root: {raw}") from error
    return candidate


def _safe_corpus_directory(root: Path, raw: str) -> Path:
    if not _is_canonical_relative_path(raw):
        raise CorpusError(f"unsafe corpus directory or non-canonical path: {raw}")
    candidate = root
    for part in PurePosixPath(raw).parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise CorpusError(f"corpus symlink is forbidden: {raw}")
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root.resolve(strict=True))
    except (OSError, ValueError) as error:
        raise CorpusError(f"corpus directory escapes root: {raw}") from error
    if not resolved.is_dir():
        raise CorpusError(f"listed corpus directory is missing: {raw}")
    return resolved


def _read_corpus_snapshot(path: Path, raw: str) -> tuple[bytes, str]:
    """Read once into immutable bytes while hashing the exact same stream."""
    digest = hashlib.sha256()
    blocks = []
    descriptor = None
    try:
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
            raise CorpusError(
                f"corpus XML snapshot is not one private regular file: {raw}"
            )
        flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(
            os, "O_NOFOLLOW", 0
        )
        descriptor = os.open(path, flags)
        opened = os.fstat(descriptor)
        identity = (
            opened.st_dev,
            opened.st_ino,
            opened.st_mode,
            opened.st_nlink,
            opened.st_size,
            opened.st_mtime_ns,
        )
        before_identity = (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_nlink,
            before.st_size,
            before.st_mtime_ns,
        )
        if identity != before_identity or not stat.S_ISREG(opened.st_mode):
            raise CorpusError(f"corpus XML snapshot identity changed before open: {raw}")
        size = 0
        while True:
            block = os.read(descriptor, 1024 * 1024)
            if not block:
                break
            blocks.append(block)
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
        final_path = path.lstat()
        final_identity = (
            final_path.st_dev,
            final_path.st_ino,
            final_path.st_mode,
            final_path.st_nlink,
            final_path.st_size,
            final_path.st_mtime_ns,
        )
        if (
            after_identity != identity
            or final_identity != identity
            or size != opened.st_size
        ):
            raise CorpusError(f"corpus XML snapshot changed while reading: {raw}")
    except CorpusError:
        raise
    except OSError as error:
        raise CorpusError(
            f"cannot capture immutable corpus XML snapshot {raw}: {error}"
        ) from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    return b"".join(blocks), digest.hexdigest()


def _corpus_empty_directory_paths(root: Path) -> list[str]:
    """Capture the minimal empty-directory topology of the corpus tree."""
    result = []

    def visit(directory: Path) -> None:
        try:
            entries = sorted(directory.iterdir(), key=lambda entry: entry.name)
        except OSError as error:
            raise CorpusError(f"cannot enumerate corpus directory {directory}: {error}") from error
        if not entries and directory != root:
            result.append(directory.relative_to(root).as_posix())
        for entry in entries:
            if entry.is_symlink():
                raise CorpusError(
                    f"corpus symlink is forbidden: {entry.relative_to(root)}"
                )
            try:
                if entry.is_dir():
                    visit(entry)
                elif not entry.is_file():
                    raise CorpusError(
                        f"special corpus entry is forbidden: {entry.relative_to(root)}"
                    )
            except OSError as error:
                raise CorpusError(
                    f"cannot inspect corpus entry {entry.relative_to(root)}: {error}"
                ) from error

    visit(root)
    return sorted(result)


def _unique_json_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _load_corpus(manifest_path: Path, expected_profile: str) -> tuple[dict, Path, list[dict]]:
    manifest_path = Path(manifest_path).absolute()
    canonical_verifier = None
    canonical_corpus = None
    if expected_profile == FIXED_PROFILE:
        canonical_verifier = _canonical_corpus_verifier()
        try:
            canonical_corpus = canonical_verifier.load_corpus(manifest_path)
        except canonical_verifier.SourceError as error:
            raise CorpusError(f"canonical corpus validation failed: {error}") from error
    try:
        manifest = json.loads(
            manifest_path.read_text(encoding="utf-8"),
            object_pairs_hook=_unique_json_object,
        )
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise CorpusError(f"invalid corpus manifest: {error}") from error
    if not isinstance(manifest, dict):
        raise CorpusError("corpus manifest root must be an object")
    expected_keys = {"schemaVersion", "profile", "emptyDirectoryPaths", "cases"}
    if set(manifest) != expected_keys:
        raise CorpusError(
            "corpus manifest has invalid shape; "
            f"expected keys={sorted(expected_keys)}, actual keys={sorted(manifest)}"
        )
    schema_version = manifest.get("schemaVersion")
    if not _is_json_integer(schema_version) or schema_version != 2:
        raise CorpusError("corpus schemaVersion must be 2")
    if manifest.get("profile") != expected_profile:
        raise CorpusError("corpus profile mismatch")
    root = manifest_path.parent
    declared_empty_directories = manifest.get("emptyDirectoryPaths")
    if not isinstance(declared_empty_directories, list) or any(
        not _is_canonical_relative_path(path)
        for path in declared_empty_directories
    ):
        raise CorpusError(
            "corpus emptyDirectoryPaths must contain canonical relative POSIX paths"
        )
    if declared_empty_directories != sorted(set(declared_empty_directories)):
        raise CorpusError("corpus emptyDirectoryPaths must be sorted and unique")
    actual_empty_directories = _corpus_empty_directory_paths(root)
    if declared_empty_directories != actual_empty_directories:
        raise CorpusError(
            "corpus empty directory inventory is not exact; "
            f"declared={declared_empty_directories}, actual={actual_empty_directories}"
        )
    files = []
    seen_paths = set()
    seen_identities = set()
    seen_cases = set()
    cases = manifest.get("cases")
    if not isinstance(cases, list) or not cases:
        raise CorpusError("corpus cases must be a non-empty list")
    for case in cases:
        if not isinstance(case, dict):
            raise CorpusError("corpus case must be an object")
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id.strip():
            raise CorpusError("corpus case id must be a non-empty string")
        if case_id in seen_cases:
            raise CorpusError(f"duplicate corpus case id: {case_id}")
        seen_cases.add(case_id)
        tool_id = case.get("toolId")
        if not isinstance(tool_id, str) or not tool_id.strip():
            raise CorpusError(f"corpus case toolId must be a non-empty string: {case_id}")
        xml_impact = case.get("xmlImpact")
        if not isinstance(xml_impact, str) or not xml_impact.strip():
            raise CorpusError(f"corpus case xmlImpact must be a non-empty string: {case_id}")
        workspace_path = case.get("workspacePath")
        pre_snapshot_path = case.get("preSnapshotPath")
        workspace = _safe_corpus_directory(root, workspace_path)
        pre_snapshot = _safe_corpus_directory(root, pre_snapshot_path)
        try:
            workspace.relative_to(root.resolve(strict=True))
            pre_snapshot.relative_to(root.resolve(strict=True))
        except ValueError as error:
            raise CorpusError(f"corpus case directories escape root: {case_id}") from error
        if workspace == pre_snapshot:
            raise CorpusError(f"workspacePath and preSnapshotPath must differ: {case_id}")

        pre_files = case.get("preFiles")
        if not isinstance(pre_files, list):
            raise CorpusError(f"corpus case preFiles must be a list: {case_id}")
        if not isinstance(case.get("files"), list) or not case["files"]:
            raise CorpusError("corpus case has no files list")
        pre_paths = []
        for snapshot_kind, entries, directory_path in (
            ("pre", pre_files, pre_snapshot_path),
            ("post", case["files"], workspace_path),
        ):
            for entry in entries:
                if not isinstance(entry, dict):
                    raise CorpusError(f"corpus file entry must be an object: {case_id}")
                if snapshot_kind == "pre":
                    required = {"path", "sha256", "family"}
                    allowed = required | {"ownerPath"}
                    if set(entry) - allowed or not required.issubset(entry):
                        raise CorpusError(
                            f"preFiles entry has invalid shape: {case_id}: {sorted(entry)}"
                        )
                raw = entry.get("path")
                if not isinstance(raw, str) or raw in seen_paths:
                    raise CorpusError(f"duplicate or missing corpus file: {raw}")
                path = _safe_corpus_path(root, raw)
                prefix = f"{directory_path}/"
                if not raw.startswith(prefix):
                    raise CorpusError(
                        f"{snapshot_kind} corpus file is outside its snapshot directory: {raw}"
                    )
                logical_path = raw[len(prefix) :]
                if not logical_path:
                    raise CorpusError(f"corpus file has no logical path: {raw}")
                seen_paths.add(raw)
                if not path.is_file():
                    raise CorpusError(f"listed corpus file is missing: {raw}")
                try:
                    file_stat = path.stat()
                except OSError as error:
                    raise CorpusError(f"cannot inspect corpus file: {raw}: {error}") from error
                if file_stat.st_nlink != 1:
                    raise CorpusError(
                        f"corpus XML hardlink alias references the same file "
                        f"(link count {file_stat.st_nlink}): {raw}"
                    )
                identity = (file_stat.st_dev, file_stat.st_ino)
                if identity in seen_identities:
                    raise CorpusError(f"duplicate corpus paths reference the same file: {raw}")
                seen_identities.add(identity)
                declared_hash = entry.get("sha256")
                if not isinstance(declared_hash, str) or re.fullmatch(r"[0-9a-f]{64}", declared_hash) is None:
                    raise CorpusError(f"corpus SHA-256 is invalid: {raw}")
                snapshot, actual_hash = _read_corpus_snapshot(path, raw)
                if actual_hash != declared_hash:
                    raise CorpusError(f"corpus hash mismatch: {raw}")
                if not isinstance(entry.get("family"), str) or not entry["family"].strip():
                    raise CorpusError(f"corpus file has no family: {raw}")
                normalized = {
                    **entry,
                    "caseId": case_id,
                    "toolId": tool_id,
                    "xmlImpact": xml_impact,
                    "snapshotKind": snapshot_kind,
                    "logicalPath": logical_path,
                    "_snapshot": snapshot,
                }
                if snapshot_kind == "pre":
                    normalized["seed"] = True
                    pre_paths.append(raw)
                else:
                    if not isinstance(entry.get("seed"), bool):
                        raise CorpusError(f"corpus file must explicitly declare seed: {raw}")
                    if "newStandalone" in entry and not isinstance(entry["newStandalone"], bool):
                        raise CorpusError(f"newStandalone must be boolean: {raw}")
                if "ownerPath" in entry and not _is_canonical_relative_path(entry["ownerPath"], suffix=".xml"):
                    raise CorpusError(f"ownerPath must be a canonical corpus XML path: {raw}")
                if snapshot_kind == "pre" and "ownerPath" in entry and not entry["ownerPath"].startswith(f"{pre_snapshot_path}/"):
                    raise CorpusError(f"preFiles ownerPath is outside preSnapshotPath: {raw}")
                files.append(normalized)
        if pre_paths != sorted(set(pre_paths)):
            raise CorpusError(f"preFiles paths must be sorted and unique: {case_id}")

        pre_owner_versions = case.get("preOwnerVersions")
        if not isinstance(pre_owner_versions, dict):
            raise CorpusError(f"preOwnerVersions must be an object: {case_id}")
        if list(pre_owner_versions) != sorted(pre_owner_versions):
            raise CorpusError(f"preOwnerVersions must be sorted: {case_id}")
        for owner_path, version in pre_owner_versions.items():
            if (
                not _is_canonical_relative_path(owner_path, suffix=".xml")
                or not owner_path.startswith(f"{pre_snapshot_path}/")
                or owner_path not in pre_paths
                or version != "2.20"
            ):
                raise CorpusError(f"invalid preOwnerVersions declaration: {case_id}")
    if not files:
        raise CorpusError("empty or unprocessed corpus")
    for path in root.rglob("*"):
        if path.is_symlink():
            raise CorpusError(f"corpus symlink is forbidden: {path.relative_to(root)}")
        if path.suffix.lower() == ".xml" and not path.is_file():
            raise CorpusError(f"special XML entry is forbidden: {path.relative_to(root)}")
    actual_xml = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path.suffix.lower() == ".xml"
    }
    unlisted = sorted(actual_xml - seen_paths)
    if unlisted:
        raise CorpusError(f"unlisted XML in corpus: {unlisted[0]}")
    final_empty_directories = _corpus_empty_directory_paths(root)
    if final_empty_directories != declared_empty_directories:
        raise CorpusError(
            "corpus empty directory topology changed while being validated; "
            f"declared={declared_empty_directories}, actual={final_empty_directories}"
        )
    if canonical_corpus is not None:
        final_snapshot = canonical_verifier.snapshot_regular_tree(root)
        if final_snapshot != canonical_corpus["snapshot"]:
            raise CorpusError(
                "canonical corpus regular-file or directory topology changed "
                "during static verification"
            )
    return manifest, root, files


def _qname(root) -> str:
    return str(etree.QName(root))


def _raw_root_version(payload: bytes, label: str) -> str | None:
    try:
        return raw_root_attribute(payload, "version")
    except LexicalXmlError as error:
        raise CorpusError(
            f"cannot inspect raw root version in corpus XML {label}: {error}"
        ) from error


def _unresolved_qname_prefix(root) -> str | None:
    for node in root.iter():
        if not isinstance(node.tag, str):
            continue
        for name, value in node.attrib.items():
            if name == f"{{{XSI_NS}}}type" and ":" in value:
                prefix = value.split(":", 1)[0]
                if prefix not in node.nsmap:
                    return prefix
        if str(etree.QName(node)) in QNAME_TEXT_ELEMENTS:
            value = (node.text or "").strip()
            if ":" in value:
                prefix = value.split(":", 1)[0]
                if prefix not in node.nsmap:
                    return prefix
    return None


def verify_corpus(manifest_path: Path | str, profile: dict, runtime: dict, edt: dict | None) -> tuple[dict, int]:
    manifest, _root, entries = _load_corpus(Path(manifest_path), profile["profile"])
    listed = {entry["path"]: entry for entry in entries}
    families_by_id = {}
    for expected_qname, configured in profile.get("families", {}).items():
        family_id = configured.get("id")
        if family_id in families_by_id:
            raise SourceError(f"duplicate configured family id: {family_id}")
        families_by_id[family_id] = (expected_qname, configured)
    for case in manifest["cases"]:
        case_id = case["id"]
        declared_pre_owners = case["preOwnerVersions"]
        actual_pre_owners = {}
        for entry in entries:
            if entry["caseId"] != case_id or entry["snapshotKind"] != "pre":
                continue
            selected = families_by_id.get(entry.get("family"))
            if selected is None or selected[1].get("sourceSetOwner") is not True:
                continue
            try:
                xml_root = etree.fromstring(
                    entry["_snapshot"],
                    parser=etree.XMLParser(no_network=True, resolve_entities=False),
                )
            except etree.XMLSyntaxError:
                continue
            if _qname(xml_root) != selected[0]:
                continue
            owner_namespace = etree.QName(xml_root).namespace
            owner_type = next(
                (
                    etree.QName(child).localname
                    for child in xml_root
                    if isinstance(child.tag, str)
                    and etree.QName(child).namespace == owner_namespace
                ),
                None,
            )
            if owner_type in selected[1].get("sourceSetOwnerTypes", []):
                actual_pre_owners[entry["path"]] = _raw_root_version(
                    entry["_snapshot"], entry["path"]
                )
        if actual_pre_owners != declared_pre_owners:
            raise CorpusError(
                f"preOwnerVersions do not exactly match pre-snapshot owners: {case_id}"
            )
    rows = []
    violation = False
    inconclusive = False
    for entry in sorted(entries, key=lambda item: item["path"]):
        checks = []
        try:
            xml_root = etree.fromstring(entry["_snapshot"], parser=etree.XMLParser(no_network=True, resolve_entities=False))
            checks.append({"name": "wellFormed", "status": "pass"})
        except etree.XMLSyntaxError as error:
            rows.append(_file_row(entry, None, "unknown", [{"name": "wellFormed", "status": "fail", "detail": str(error)}], "fail", "repair XML syntax"))
            violation = True
            continue
        qname = _qname(xml_root)
        selected = families_by_id.get(entry.get("family"))
        if selected is None:
            raise CorpusError(f"unknown corpus family: {entry.get('family')}")
        expected_qname, family = selected
        root_status = "pass" if qname == expected_qname else "fail"
        checks.append({"name": "rootQName", "status": root_status, "actual": qname, "expected": expected_qname})
        violation |= root_status == "fail"
        prefix = _unresolved_qname_prefix(xml_root)
        if prefix:
            checks.append({"name": "qnamePrefixes", "status": "fail", "detail": f"unresolved prefix: {prefix}"})
            violation = True
        else:
            checks.append({"name": "qnamePrefixes", "status": "pass"})
        version_mode = family.get("version", "none")
        if version_mode == "root":
            actual = _raw_root_version(entry["_snapshot"], entry["path"])
            status = "pass" if actual == profile["exportVersion"] else "fail"
            checks.append({"name": "exportVersion", "status": status, "actual": actual, "expected": profile["exportVersion"]})
            violation |= status == "fail"
        elif version_mode == "owner":
            owner_path = entry.get("ownerPath")
            if owner_path:
                owner = listed.get(owner_path)
                if owner is None:
                    raise CorpusError(f"ownerPath is not a listed corpus file: {owner_path}")
                owner_selected = families_by_id.get(owner.get("family"))
                if owner_selected is None or owner_selected[1].get("sourceSetOwner") is not True:
                    raise CorpusError(f"ownerPath is not a profile-marked source-set owner descriptor: {owner_path}")
                if owner.get("caseId") != entry.get("caseId"):
                    raise CorpusError(f"ownerPath must reference a source-set owner in the same case: {owner_path}")
                if owner.get("snapshotKind") != entry.get("snapshotKind"):
                    raise CorpusError(
                        f"ownerPath must remain inside the same pre/post snapshot: {owner_path}"
                    )
                try:
                    owner_root = etree.fromstring(owner["_snapshot"], parser=etree.XMLParser(no_network=True, resolve_entities=False))
                    actual = _raw_root_version(owner["_snapshot"], owner_path)
                except etree.XMLSyntaxError as error:
                    actual = None
                    checks.append({"name": "ownerVersion", "status": "fail", "detail": str(error)})
                else:
                    owner_qname = _qname(owner_root)
                    expected_owner_qname = owner_selected[0]
                    owner_namespace = etree.QName(owner_root).namespace
                    owner_type = next(
                        (
                            etree.QName(child).localname
                            for child in owner_root
                            if isinstance(child.tag, str) and etree.QName(child).namespace == owner_namespace
                        ),
                        None,
                    )
                    allowed_owner_types = owner_selected[1].get("sourceSetOwnerTypes", [])
                    if owner_qname != expected_owner_qname:
                        raise CorpusError(
                            f"ownerPath descriptor is not the configured source-set owner: {owner_path}: {owner_qname}"
                        )
                    if owner_type not in allowed_owner_types:
                        raise CorpusError(
                            f"ownerPath descriptor type is not a source-set owner: {owner_path}: {owner_type}"
                        )
                    status = "pass" if actual == profile["exportVersion"] else "fail"
                    checks.append({"name": "ownerVersion", "status": status, "actual": actual, "expected": profile["exportVersion"], "ownerPath": owner_path})
                    violation |= status == "fail"
            elif entry.get("newStandalone") is True and entry.get("xmlImpact") == "created" and entry.get("seed") is False:
                checks.append({"name": "ownerVersion", "status": "pass", "detail": "explicit new standalone content"})
            else:
                checks.append({"name": "ownerVersion", "status": "fail", "detail": "ownerPath or newStandalone=true is required"})
                violation = True

        coverage = family["coverage"]
        xsd_error = None
        if coverage == "strict":
            xsd_error = strict_xsd_error(runtime, family["schemaNamespace"], family["wrapperType"], etree.tostring(xml_root, encoding="unicode"))
            checks.append({"name": "runtimeXsd", "status": "fail" if xsd_error else "pass", "detail": xsd_error or ""})
            violation |= xsd_error is not None
            result = "fail" if any(check["status"] == "fail" for check in checks) else "pass"
            next_evidence = "none" if result == "pass" else "fix document contract or strict XSD violation"
        else:
            if coverage == "not-covered" and family.get("edtDeclaration"):
                declaration = bool(edt and edt.get("declarations", {}).get(family["edtDeclaration"]))
                checks.append({"name": "edtDeclaration", "status": "pass" if declaration else "unavailable"})
            result = "fail" if any(check["status"] == "fail" for check in checks) else "inconclusive"
            next_evidence = "platform 8.3.27.2074 load/dump roundtrip"
            inconclusive |= result == "inconclusive"
        rows.append(_file_row(entry, qname, coverage, checks, result, next_evidence))
    exit_code = 1 if violation or any(row["result"] == "fail" for row in rows) else (3 if inconclusive else 0)
    verdict = {0: "pass", 1: "fail", 3: "inconclusive"}[exit_code]
    report = {
        "schemaVersion": 1,
        "profile": profile["profile"],
        "verdict": verdict,
        "exitCode": exit_code,
        "sources": {"runtime": runtime["source"], "edt": _public_edt(edt)},
        "schemaCompilation": runtime["compilationMatrix"],
        "files": rows,
        "summary": {
            "files": len(rows),
            "passed": sum(row["result"] == "pass" for row in rows),
            "failed": sum(row["result"] == "fail" for row in rows),
            "inconclusive": sum(row["result"] == "inconclusive" for row in rows),
        },
    }
    return report, exit_code


def _file_row(entry, qname, coverage, checks, result, next_evidence):
    return {
        "caseId": entry.get("caseId"), "toolId": entry.get("toolId"),
        "xmlImpact": entry.get("xmlImpact"), "path": entry.get("path"),
        "sha256": entry.get("sha256"), "rootQName": qname,
        "coverage": coverage, "checks": checks, "result": result,
        "requiredNextEvidence": next_evidence,
    }


def _public_edt(edt):
    if not edt:
        return {"provided": False, "evidenceScope": "none"}
    return {key: value for key, value in edt.items() if not key.startswith("_")}


def _unfold_manifest(text: str) -> dict[str, str]:
    logical = []
    for line in text.replace("\r\n", "\n").split("\n"):
        if line.startswith(" ") and logical:
            logical[-1] += line[1:]
        else:
            logical.append(line)
    result = {}
    for line in logical:
        if ": " in line:
            key, value = line.split(": ", 1)
            result[key] = value
    return result


def verified_edt(path: Path | str, profile: dict, runner=subprocess.run) -> dict:
    path = Path(path)
    expected = profile.get("edt", {})
    if not isinstance(expected, dict):
        raise SourceError("profile has no EDT contract")
    _validate_edt_contract(expected)
    with tempfile.TemporaryDirectory(prefix="unica-8-3-27-edt-") as temporary:
        private_jar = Path(temporary) / "edt-evidence.jar"
        actual_hash = _private_copy_with_sha256(path, private_jar, "EDT jar")
        if actual_hash != expected.get("sha256"):
            raise SourceError("EDT jar SHA-256 mismatch or file missing")
        try:
            completed = runner(
                ["jarsigner", "-verify", "-certs", str(private_jar)],
                capture_output=True,
                text=True,
            )
        except OSError as error:
            raise SourceError(f"jarsigner is unavailable for EDT verification: {error}") from error
        if completed.returncode != 0 or "jar verified" not in (completed.stdout + completed.stderr).lower():
            raise SourceError("jarsigner verification failed for EDT jar")
        try:
            archive_context = zipfile.ZipFile(private_jar)
        except (OSError, zipfile.BadZipFile, zipfile.LargeZipFile) as error:
            raise SourceError(f"invalid EDT ZIP/JAR archive: {error}") from error
        with archive_context as archive:
            infos = _safe_zip_members(archive)
            names = {info.filename for info in infos}
            try:
                headers = _unfold_manifest(
                    _read_zip_member(archive, "META-INF/MANIFEST.MF", "EDT OSGi manifest").decode("utf-8")
                )
            except UnicodeDecodeError as error:
                raise SourceError(f"invalid EDT OSGi manifest: {error}") from error
            if headers.get("Bundle-SymbolicName", "").split(";", 1)[0] != expected.get("symbolicName"):
                raise SourceError("EDT Bundle-SymbolicName mismatch")
            if headers.get("Bundle-Version") != expected.get("version"):
                raise SourceError("EDT Bundle-Version mismatch")
            entry_text = {}
            entries = expected.get("entries")
            if not isinstance(entries, dict):
                raise SourceError("EDT entries contract is invalid")
            for label, name in entries.items():
                if not isinstance(name, str) or name not in names:
                    raise SourceError(f"required EDT XDTO entry is missing: {name}")
                try:
                    entry_text[label] = _read_zip_member(archive, name, "EDT XDTO").decode("utf-8")
                except UnicodeDecodeError as error:
                    raise SourceError(f"invalid EDT XDTO entry encoding: {name}") from error
            declarations = {}
            declaration_rules = expected.get("declarations")
            if not isinstance(declaration_rules, dict):
                raise SourceError("EDT declarations contract is invalid")
            for label, rule in declaration_rules.items():
                if not isinstance(rule, dict) or not isinstance(rule.get("entry"), str) or not isinstance(rule.get("tokens"), list):
                    raise SourceError(f"invalid EDT declaration rule: {label}")
                text = entry_text.get(rule["entry"], "")
                declarations[label] = all(isinstance(token, str) and token in text for token in rule["tokens"])
                if not declarations[label]:
                    raise SourceError(f"EDT declaration evidence is missing: {label}")
    return {
        "provided": True, "sha256": actual_hash,
        "bundleSymbolicName": expected["symbolicName"], "bundleVersion": expected["version"],
        "evidenceScope": "8.3.27-line declaration evidence; not proof of patch build 8.3.27.2074",
        "identityStatement": "Configured SHA-256 proves identity with approved local evidence, not download provenance.",
        "declarations": declarations,
    }


def _json_values_equal(left, right) -> bool:
    left_is_number = isinstance(left, (int, float)) and not isinstance(left, bool)
    right_is_number = isinstance(right, (int, float)) and not isinstance(right, bool)
    if left_is_number or right_is_number:
        return left_is_number and right_is_number and left == right
    if isinstance(left, bool) or isinstance(right, bool):
        return isinstance(left, bool) and isinstance(right, bool) and left == right
    if isinstance(left, list) or isinstance(right, list):
        return (
            isinstance(left, list)
            and isinstance(right, list)
            and len(left) == len(right)
            and all(_json_values_equal(left_item, right_item) for left_item, right_item in zip(left, right))
        )
    if isinstance(left, dict) or isinstance(right, dict):
        return (
            isinstance(left, dict)
            and isinstance(right, dict)
            and set(left) == set(right)
            and all(_json_values_equal(left[key], right[key]) for key in left)
        )
    return type(left) is type(right) and left == right


def _matches_schema(value, schema) -> bool:
    if "const" in schema and not _json_values_equal(value, schema["const"]):
        return False
    if "oneOf" in schema and sum(_matches_schema(value, choice) for choice in schema["oneOf"]) != 1:
        return False
    if "enum" in schema and not any(_json_values_equal(value, choice) for choice in schema["enum"]):
        return False
    expected_type = schema.get("type")
    if isinstance(expected_type, list):
        if not any(_matches_schema(value, {"type": choice}) for choice in expected_type):
            return False
        expected_type = None
    if expected_type == "object":
        if not isinstance(value, dict):
            return False
        if any(key not in value for key in schema.get("required", [])):
            return False
        properties = schema.get("properties", {})
        if not all(key not in value or _matches_schema(value[key], child) for key, child in properties.items()):
            return False
        additional = schema.get("additionalProperties", True)
        unknown = set(value) - set(properties)
        if additional is False and unknown:
            return False
        if isinstance(additional, dict) and any(not _matches_schema(value[key], additional) for key in unknown):
            return False
        return True
    if expected_type == "array":
        if not isinstance(value, list):
            return False
        if len(value) < schema.get("minItems", 0):
            return False
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            return False
        return all(_matches_schema(item, schema.get("items", {})) for item in value)
    if expected_type == "string":
        if not isinstance(value, str) or len(value) < schema.get("minLength", 0):
            return False
        return "pattern" not in schema or re.search(schema["pattern"], value) is not None
    if expected_type == "integer":
        if not _is_json_integer(value):
            return False
        return "minimum" not in schema or value >= schema["minimum"]
    if expected_type == "boolean":
        return isinstance(value, bool)
    if expected_type == "null":
        return value is None
    return True


def _report_semantics(report: dict) -> bool:
    if not isinstance(report, dict):
        return False
    summary = report.get("summary")
    files = report.get("files")
    if not isinstance(summary, dict) or not isinstance(files, list):
        return False
    expected_summary = {
        "files": len(files),
        "passed": sum(isinstance(row, dict) and row.get("result") == "pass" for row in files),
        "failed": sum(isinstance(row, dict) and row.get("result") == "fail" for row in files),
        "inconclusive": sum(isinstance(row, dict) and row.get("result") == "inconclusive" for row in files),
    }
    if summary != expected_summary:
        return False
    verdict = report.get("verdict")
    if verdict == "source-error":
        return (
            isinstance(report.get("sourceError"), str)
            and bool(report["sourceError"].strip())
            and not files
            and report.get("schemaCompilation") == []
            and report.get("sources") == {}
        )
    if "sourceError" in report:
        return False
    if not files or not isinstance(report.get("schemaCompilation"), list) or not report["schemaCompilation"]:
        return False
    for row in files:
        if not isinstance(row, dict) or not isinstance(row.get("checks"), list) or not row["checks"]:
            return False
        statuses = [check.get("status") for check in row["checks"] if isinstance(check, dict)]
        if len(statuses) != len(row["checks"]):
            return False
        result = row.get("result")
        if "fail" in statuses:
            if result != "fail":
                return False
            continue
        if result == "fail":
            return False
        if row.get("coverage") == "strict":
            if not all(status == "pass" for status in statuses) or result != "pass":
                return False
        elif result != "inconclusive":
            return False
    sources = report.get("sources")
    if not isinstance(sources, dict) or not isinstance(sources.get("runtime"), dict) or not isinstance(sources.get("edt"), dict):
        return False
    manifest_summary = sources["runtime"].get("manifestSummary")
    if not isinstance(manifest_summary, dict) or len(report["schemaCompilation"]) != manifest_summary.get("schemas"):
        return False
    if verdict == "pass":
        return expected_summary["passed"] == expected_summary["files"]
    if verdict == "fail":
        return expected_summary["failed"] > 0
    if verdict == "inconclusive":
        return expected_summary["failed"] == 0 and expected_summary["inconclusive"] > 0
    return False


def report_matches_schema(value, schema) -> bool:
    return _matches_schema(value, schema) and _report_semantics(value)


def _error_report(profile_id: object, error: Exception) -> dict:
    detail = str(error).strip() or error.__class__.__name__
    safe_profile_id = profile_id if _is_nonempty_string(profile_id) else "unknown"
    return {"schemaVersion": 1, "profile": safe_profile_id, "verdict": "source-error", "exitCode": 2, "sourceError": detail, "sources": {}, "schemaCompilation": [], "files": [], "summary": {"files": 0, "passed": 0, "failed": 0, "inconclusive": 0}}


def _read_json_object(path: Path, label: str) -> dict:
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=_unique_json_object,
        )
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise SourceError(f"invalid {label}: {error}") from error
    if not isinstance(value, dict):
        raise SourceError(f"invalid {label}: root must be an object")
    return value


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-xsd-zip", required=True, type=Path)
    parser.add_argument("--edt-xdto-jar", required=True, type=Path)
    parser.add_argument(
        "--corpus",
        required=True,
        type=Path,
        help="path to the immutable corpus-manifest.json",
    )
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--profile", type=Path, default=Path(__file__).with_name("verify-8-3-27-xml-profile.json"), help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    profile = {}
    report_schema = None
    runtime = None
    try:
        report_schema = _read_json_object(Path(__file__).with_name(REPORT_SCHEMA_NAME), "report schema")
        profile = _read_json_object(args.profile, "verification profile")
        _validate_profile(profile)
        runtime = verified_runtime(args.runtime_xsd_zip, profile)
        edt = verified_edt(args.edt_xdto_jar, profile)
        report, status = verify_corpus(args.corpus, profile, runtime, edt)
    except (SourceError, CorpusError) as error:
        report, status = _error_report(profile.get("profile", "unknown"), error), 2
    finally:
        if runtime is not None:
            runtime.close()
    if report_schema is not None and not report_matches_schema(report, report_schema):
        report, status = _error_report(
            profile.get("profile", "unknown"),
            SourceError("generated report failed internal report-schema validation"),
        ), 2
    try:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except OSError as error:
        print(f"cannot write verification report: {error}", file=sys.stderr)
        return 2
    return status


if __name__ == "__main__":
    sys.exit(main())
