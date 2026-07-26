import hashlib
import importlib.util
import json
import os
import shutil
import stat
import struct
import subprocess
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dev/verify-8-3-27-xml.py"
PROFILE = ROOT / "scripts/dev/verify-8-3-27-xml-profile.json"
REPORT_SCHEMA = ROOT / "scripts/dev/verify-8-3-27-xml-report.schema.json"


def load_verifier():
    spec = importlib.util.spec_from_file_location("verify_8_3_27_xml", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


XS = "http://www.w3.org/2001/XMLSchema"
DCS_NS = "urn:test:dcs"
ROLE_NS = "urn:test:roles"
MXL_NS = "urn:test:mxl"
MD_NS = "urn:test:md"
REAL_MD_NS = "http://v8.1c.ru/8.3/MDClasses"
REAL_CAI_NS = "http://v8.1c.ru/8.2/managed-application/core"
REAL_SCHEME_NS = "http://v8.1c.ru/8.3/xcf/scheme"
REAL_FLOWCHART_QNAME = f"{{{REAL_SCHEME_NS}}}GraphicalSchema"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def schema(namespace, body, imports=""):
    return (
        f'<xs:schema xmlns:xs="{XS}" xmlns:tns="{namespace}" '
        f'targetNamespace="{namespace}" elementFormDefault="qualified">'
        f"{imports}{body}</xs:schema>"
    ).encode()


def runtime_archive(root, schemas, *, extra=None, summary=None):
    entries = []
    exports = []
    for index, (namespace, payload) in enumerate(schemas, 1):
        name = f"schemas/{index:04}.xsd"
        entries.append({"file": name, "targetNamespace": namespace, "sha256": sha(payload), "size": len(payload)})
        exports.append({"sourceNamespace": namespace, "files": [name], "error": ""})
    count = len(entries)
    manifest = {
        "formatVersion": 1,
        "platformVersion": "test-platform",
        "summary": summary or {"packages": count, "namespaces": count, "success": count, "schemas": count, "errors": 0},
        "exports": exports,
        "schemas": entries,
    }
    target = root / "runtime.zip"
    with zipfile.ZipFile(target, "w") as archive:
        archive.writestr("export/manifest.json", json.dumps(manifest))
        for entry, (_, payload) in zip(entries, schemas):
            archive.writestr("export/" + entry["file"], payload)
        for name, payload, attrs in extra or []:
            info = zipfile.ZipInfo(name)
            info.external_attr = attrs
            archive.writestr(info, payload)
    return target


def test_profile(runtime_zip):
    return {
        "profile": "test-2.20",
        "exportVersion": "2.20",
        "runtime": {
            "sha256": sha(runtime_zip.read_bytes()),
            "formatVersion": 1,
            "platformVersion": "test-platform",
            "summary": {"packages": 2, "namespaces": 2, "success": 2, "schemas": 2, "errors": 0},
            "knownCompileFailures": {},
        },
        "edt": {"sha256": "unused", "symbolicName": "test", "version": "1", "entries": {}, "declarations": {}},
        "families": {
            f"{{{DCS_NS}}}DataCompositionSchema": {
                "id": "dcs", "coverage": "strict", "schemaNamespace": DCS_NS,
                "wrapperType": "DataCompositionSchema", "wrapper": "dcs-root-alias",
                "version": "owner"
            },
            f"{{{ROLE_NS}}}Rights": {
                "id": "roles", "coverage": "strict", "schemaNamespace": ROLE_NS,
                "wrapperType": "Rights", "wrapper": "roles-global-element", "version": "root"
            },
            f"{{{MXL_NS}}}document": {
                "id": "mxl", "coverage": "known-schema-incompatibility", "version": "owner"
            },
            f"{{{MD_NS}}}MetaDataObject": {
                "id": "metadata", "coverage": "not-covered", "version": "root",
                "edtDeclaration": "metadata", "sourceSetOwner": True,
                "sourceSetOwnerTypes": ["Configuration", "ConfigurationExtension", "ExternalDataProcessor", "ExternalReport"]
            }
        }
    }


def rewrite_manifest(source, target, mutate):
    with zipfile.ZipFile(source) as archive:
        members = [(info.filename, archive.read(info.filename)) for info in archive.infolist()]
    with zipfile.ZipFile(target, "w") as archive:
        for name, payload in members:
            if name.endswith("manifest.json"):
                value = json.loads(payload)
                mutate(value)
                payload = json.dumps(value).encode()
            archive.writestr(name, payload)
    return target


def basic_schemas():
    dcs = schema(DCS_NS, '<xs:complexType name="DataCompositionSchema"><xs:sequence><xs:element name="name" type="xs:string"/></xs:sequence></xs:complexType>')
    roles = schema(ROLE_NS, '<xs:complexType name="Rights"><xs:sequence><xs:element name="setForNewObjects" type="xs:boolean"/></xs:sequence><xs:attribute name="version" type="xs:string" use="required"/></xs:complexType>')
    return [(DCS_NS, dcs), (ROLE_NS, roles)]


def write_corpus(root, files):
    cases = []
    listed = []
    case_dir = root / "case"
    case_dir.mkdir(parents=True)
    pre_dir = root / "pre-case"
    pre_dir.mkdir()
    for name, text, extra in files:
        path = case_dir / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        parsed = __import__("lxml.etree", fromlist=["etree"]).fromstring(text.encode())
        qname = str(__import__("lxml.etree", fromlist=["etree"]).QName(parsed))
        family = {
            f"{{{DCS_NS}}}DataCompositionSchema": "dcs",
            f"{{{ROLE_NS}}}Rights": "roles",
            f"{{{MXL_NS}}}document": "mxl",
            f"{{{MD_NS}}}MetaDataObject": "metadata",
        }.get(qname, "unknown")
        item = {"path": path.relative_to(root).as_posix(), "sha256": sha(path.read_bytes()), "seed": False, "family": family}
        item.update(extra)
        listed.append(item)
    pre_files = []
    pre_owner_versions = {}
    for item in listed:
        if item["seed"] is not True:
            continue
        logical = item["path"].removeprefix("case/")
        post_path = root / item["path"]
        pre_path = pre_dir / logical
        pre_path.parent.mkdir(parents=True, exist_ok=True)
        pre_path.write_bytes(post_path.read_bytes())
        pre_item = {
            "path": f"pre-case/{logical}",
            "sha256": sha(pre_path.read_bytes()),
            "family": item["family"],
        }
        if "ownerPath" in item:
            pre_item["ownerPath"] = item["ownerPath"].replace("case/", "pre-case/", 1)
        pre_files.append(pre_item)
        parsed = __import__("lxml.etree", fromlist=["etree"]).fromstring(pre_path.read_bytes())
        children = [child for child in parsed if isinstance(child.tag, str)]
        if (
            str(__import__("lxml.etree", fromlist=["etree"]).QName(parsed))
            == f"{{{MD_NS}}}MetaDataObject"
            and children
            and __import__("lxml.etree", fromlist=["etree"]).QName(children[0]).localname
            in {"Configuration", "ConfigurationExtension", "ExternalDataProcessor", "ExternalReport"}
        ):
            pre_owner_versions[pre_item["path"]] = "2.20"
    pre_files.sort(key=lambda item: item["path"])
    cases.append({
        "id": "case",
        "workspacePath": "case",
        "preSnapshotPath": "pre-case",
        "toolId": "unica.test",
        "xmlImpact": "created",
        "preFiles": pre_files,
        "files": listed,
        "preOwnerVersions": dict(sorted(pre_owner_versions.items())),
    })
    manifest = root / "corpus-manifest.json"
    empty_directory_paths = sorted(
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_dir() and not any(path.iterdir())
    )
    manifest.write_text(
        json.dumps({
            "schemaVersion": 2,
            "profile": "test-2.20",
            "emptyDirectoryPaths": empty_directory_paths,
            "cases": cases,
        }),
        encoding="utf-8",
    )
    return manifest


def edt_jar(root):
    target = root / "edt.jar"
    manifest = "Manifest-Version: 1.0\r\nBundle-SymbolicName: test\r\nBundle-Version: 1\r\n\r\n"
    with zipfile.ZipFile(target, "w") as archive:
        archive.writestr("META-INF/MANIFEST.MF", manifest)
        archive.writestr("backend.xdto", 'targetNamespace="urn:test:md" name="MetaDataObject"')
    return target


def corrupt_zip_member(path, member, mode):
    with zipfile.ZipFile(path) as archive:
        info = archive.getinfo(member)
    payload = bytearray(path.read_bytes())
    if mode == "crc":
        name_size, extra_size = struct.unpack_from("<HH", payload, info.header_offset + 26)
        data_offset = info.header_offset + 30 + name_size + extra_size
        if info.compress_size == 0:
            raise AssertionError("test member must not be empty")
        payload[data_offset] ^= 1
    elif mode == "encrypted":
        local_flags = struct.unpack_from("<H", payload, info.header_offset + 6)[0]
        struct.pack_into("<H", payload, info.header_offset + 6, local_flags | 1)
        offset = 0
        while True:
            offset = payload.find(b"PK\x01\x02", offset)
            if offset < 0:
                raise AssertionError(f"central directory member is missing: {member}")
            name_size, extra_size, comment_size = struct.unpack_from("<HHH", payload, offset + 28)
            name = bytes(payload[offset + 46:offset + 46 + name_size]).decode()
            if name == member:
                central_flags = struct.unpack_from("<H", payload, offset + 8)[0]
                struct.pack_into("<H", payload, offset + 8, central_flags | 1)
                break
            offset += 46 + name_size + extra_size + comment_size
    else:
        raise AssertionError(f"unknown corruption mode: {mode}")
    path.write_bytes(payload)
    return path


class VerifierContractTests(unittest.TestCase):
    def test_checked_in_verifier_contract_files_exist(self):
        self.assertTrue(SCRIPT.is_file())
        self.assertTrue(PROFILE.is_file())
        self.assertTrue(REPORT_SCHEMA.is_file())

    def test_runtime_rejects_hash_mismatch_and_zip_traversal(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            profile["runtime"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(verifier.SourceError, "SHA-256"):
                verifier.verified_runtime(archive, profile)
            traversal = runtime_archive(root, basic_schemas(), extra=[("../escape.xsd", b"x", 0)])
            profile = test_profile(traversal)
            with self.assertRaisesRegex(verifier.SourceError, "unsafe ZIP"):
                verifier.verified_runtime(traversal, profile)

            for unsafe_name in ("C:/escape.xsd", "..\\escape.xsd"):
                windows_escape = runtime_archive(root, basic_schemas(), extra=[(unsafe_name, b"x", 0)])
                with self.subTest(unsafe_name=unsafe_name):
                    with self.assertRaisesRegex(verifier.SourceError, "unsafe ZIP"):
                        verifier.verified_runtime(windows_escape, test_profile(windows_escape))

    def test_runtime_rejects_symlink_and_manifest_hash_or_namespace_mismatch(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            attrs = (stat.S_IFLNK | 0o777) << 16
            archive = runtime_archive(root, basic_schemas(), extra=[("export/link", b"target", attrs)])
            with self.assertRaisesRegex(verifier.SourceError, "symlink"):
                verifier.verified_runtime(archive, test_profile(archive))

            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            duplicate = root / "duplicate.zip"
            duplicate.write_bytes(archive.read_bytes())
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(duplicate, "a") as out:
                    out.writestr("export/schemas/0001.xsd", b"changed")
            profile["runtime"]["sha256"] = sha(duplicate.read_bytes())
            with self.assertRaisesRegex(verifier.SourceError, "duplicate ZIP member"):
                verifier.verified_runtime(duplicate, profile)

            bad_hash = rewrite_manifest(archive, root / "bad-hash.zip", lambda value: value["schemas"][0].update(sha256="0" * 64))
            profile = test_profile(bad_hash)
            with self.assertRaisesRegex(verifier.SourceError, "manifest hash mismatch"):
                verifier.verified_runtime(bad_hash, profile)

            bad_namespace = rewrite_manifest(archive, root / "bad-namespace.zip", lambda value: value["schemas"][0].update(targetNamespace="urn:wrong"))
            profile = test_profile(bad_namespace)
            with self.assertRaisesRegex(verifier.SourceError, "manifest targetNamespace mismatch"):
                verifier.verified_runtime(bad_namespace, profile)

    def test_import_resolution_duplicate_namespace_and_strict_compile_failure(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            imported = schema("urn:test:base", '<xs:simpleType name="Name"><xs:restriction base="xs:string"/></xs:simpleType>')
            imports = '<xs:import namespace="urn:test:base"/>'
            consumer = schema(DCS_NS, '<xs:complexType name="DataCompositionSchema"><xs:sequence><xs:element name="name" type="b:Name" xmlns:b="urn:test:base"/></xs:sequence></xs:complexType>', imports)
            archive = runtime_archive(root, [(DCS_NS, consumer), ("urn:test:base", imported)])
            profile = test_profile(archive)
            profile["families"].pop(f"{{{ROLE_NS}}}Rights")
            verified = verifier.verified_runtime(archive, profile)
            self.assertTrue(all(row["status"] == "compiled" for row in verified["compilationMatrix"]))
            verified.close()

            duplicate = runtime_archive(root, [(DCS_NS, consumer), (DCS_NS, consumer)])
            profile = test_profile(duplicate)
            with self.assertRaisesRegex(verifier.SourceError, "duplicate targetNamespace"):
                verifier.verified_runtime(duplicate, profile)

            broken = schema(DCS_NS, '<xs:complexType name="DataCompositionSchema"><xs:restriction/></xs:complexType>')
            archive = runtime_archive(root, [(DCS_NS, broken), (ROLE_NS, basic_schemas()[1][1])])
            profile = test_profile(archive)
            with self.assertRaisesRegex(verifier.SourceError, "XSD compile failure|strict schema"):
                verifier.verified_runtime(archive, profile)

            unresolved = schema(DCS_NS, '<xs:complexType name="DataCompositionSchema"/>', '<xs:import namespace="urn:missing"/>')
            archive = runtime_archive(root, [(DCS_NS, unresolved), (ROLE_NS, basic_schemas()[1][1])])
            profile = test_profile(archive)
            with self.assertRaisesRegex(verifier.SourceError, "unresolved namespace import"):
                verifier.verified_runtime(archive, profile)

    def test_runtime_rejects_external_import_include_and_redefine_locations(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            imported = root / "imported.xsd"
            imported.write_bytes(schema("urn:test:outside", '<xs:simpleType name="Name"><xs:restriction base="xs:string"/></xs:simpleType>'))
            included = root / "included.xsd"
            included.write_bytes(schema(DCS_NS, '<xs:simpleType name="Name"><xs:restriction base="xs:string"/></xs:simpleType>'))
            references = (
                f'<xs:import namespace="urn:test:outside" schemaLocation="{imported.as_uri()}"/>',
                f'<xs:include schemaLocation="{included.as_uri()}"/>',
                f'<xs:redefine schemaLocation="{included.as_uri()}"><xs:simpleType name="Name"><xs:restriction base="tns:Name"><xs:maxLength value="10"/></xs:restriction></xs:simpleType></xs:redefine>',
            )
            for reference in references:
                consumer = schema(
                    DCS_NS,
                    '<xs:complexType name="DataCompositionSchema"><xs:sequence><xs:element name="name" type="xs:string"/></xs:sequence></xs:complexType>',
                    reference,
                )
                archive = runtime_archive(root, [(DCS_NS, consumer), basic_schemas()[1]])
                with self.subTest(reference=reference.split()[0]):
                    with self.assertRaisesRegex(verifier.SourceError, "manifest|external|schemaLocation|reference"):
                        verifier.verified_runtime(archive, test_profile(archive))

            known_namespace_external = schema(
                DCS_NS,
                '<xs:complexType name="DataCompositionSchema"><xs:sequence><xs:element name="name" type="xs:string"/></xs:sequence></xs:complexType>',
                f'<xs:import namespace="{ROLE_NS}" schemaLocation="{imported.as_uri()}"/>',
            )
            archive = runtime_archive(root, [(DCS_NS, known_namespace_external), basic_schemas()[1]])
            with self.assertRaisesRegex(verifier.SourceError, "manifest|external|schemaLocation|reference"):
                verifier.verified_runtime(archive, test_profile(archive))

    def test_controlled_known_external_import_is_a_non_pass_matrix_row(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            external_holder = schema(
                "urn:test:known-external",
                '<xs:complexType name="Holder"/>',
                '<xs:import namespace="http://www.w3.org/XML/1998/namespace" schemaLocation="http://www.w3.org/2001/xml.xsd"/>',
            )
            archive = runtime_archive(root, [*basic_schemas(), ("urn:test:known-external", external_holder)])
            profile = test_profile(archive)
            profile["runtime"]["summary"] = {"packages": 3, "namespaces": 3, "success": 3, "schemas": 3, "errors": 0}
            profile["runtime"]["knownCompileFailures"] = {"0003.xsd": "controlled external dependency"}
            profile["runtime"]["knownExternalImports"] = [{
                "file": "schemas/0003.xsd",
                "namespace": "http://www.w3.org/XML/1998/namespace",
                "schemaLocation": "http://www.w3.org/2001/xml.xsd",
            }]
            runtime = verifier.verified_runtime(archive, profile)
            failed = [row for row in runtime["compilationMatrix"] if row["status"] != "compiled"]
            self.assertEqual([(row["file"], row["status"]) for row in failed], [("schemas/0003.xsd", "known-source-incompatibility")])
            runtime.close()

    def test_strict_dependency_compile_failure_is_a_source_error(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            broken_dependency = schema("urn:test:base", '<xs:simpleType name="Name"><xs:restriction/></xs:simpleType>')
            consumer = schema(
                DCS_NS,
                '<xs:complexType name="DataCompositionSchema"><xs:sequence><xs:element name="name" type="b:Name" xmlns:b="urn:test:base"/></xs:sequence></xs:complexType>',
                '<xs:import namespace="urn:test:base"/>',
            )
            archive = runtime_archive(root, [(DCS_NS, consumer), ("urn:test:base", broken_dependency)])
            profile = test_profile(archive)
            profile["families"].pop(f"{{{ROLE_NS}}}Rights")
            with self.assertRaisesRegex(verifier.SourceError, "compile failure|strict schema"):
                verifier.verified_runtime(archive, profile)

    def test_runtime_export_rows_must_match_declared_namespace_and_summary_counts(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            swapped = rewrite_manifest(
                archive,
                root / "swapped.zip",
                lambda value: (
                    value["exports"][0].update(files=[value["schemas"][1]["file"]]),
                    value["exports"][1].update(files=[value["schemas"][0]["file"]]),
                ),
            )
            with self.assertRaisesRegex(verifier.SourceError, "export|namespace"):
                verifier.verified_runtime(swapped, test_profile(swapped))

            for field, value in (("packages", 3), ("namespaces", 3), ("schemas", 3), ("success", 1), ("errors", 1)):
                changed = rewrite_manifest(
                    archive,
                    root / f"bad-{field}.zip",
                    lambda manifest, field=field, value=value: manifest["summary"].update({field: value}),
                )
                profile = test_profile(changed)
                with zipfile.ZipFile(changed) as source:
                    manifest = json.loads(source.read("export/manifest.json"))
                profile["runtime"]["summary"] = manifest["summary"]
                with self.subTest(field=field):
                    with self.assertRaisesRegex(verifier.SourceError, "count|summary|success|error|export"):
                        verifier.verified_runtime(changed, profile)

    def test_runtime_manifest_rejects_boolean_format_version(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            boolean_version = rewrite_manifest(
                archive,
                root / "boolean-format-version.zip",
                lambda value: value.update(formatVersion=True),
            )
            profile = test_profile(boolean_version)
            with self.subTest(api="verified_runtime"):
                try:
                    runtime = verifier.verified_runtime(boolean_version, profile)
                except verifier.SourceError as error:
                    self.assertIn("formatVersion", str(error))
                else:
                    runtime.close()
                    self.fail("boolean runtime formatVersion must not equal integer 1")

            profile["edt"] = {
                "sha256": "0" * 64, "symbolicName": "test", "version": "1",
                "entries": {"backend": "backend.xdto"},
                "declarations": {
                    "metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]},
                },
            }
            profile_path = root / "profile.json"
            profile_path.write_text(json.dumps(profile))
            report_path = root / "report.json"
            with self.subTest(api="cli"):
                status = verifier.main([
                    "--runtime-xsd-zip", str(boolean_version),
                    "--edt-xdto-jar", str(root / "unused.jar"),
                    "--corpus", str(root / "unused-corpus.json"),
                    "--report", str(report_path),
                    "--profile", str(profile_path),
                ])
                report = json.loads(report_path.read_text())
                self.assertEqual(status, 2)
                self.assertIn("formatVersion", report["sourceError"])
                self.assertTrue(verifier.report_matches_schema(report, schema_value))

    def test_runtime_and_edt_process_the_same_private_bytes_that_were_hashed(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            original_dir = root / "original"
            replacement_dir = root / "replacement"
            original_dir.mkdir()
            replacement_dir.mkdir()
            archive = runtime_archive(original_dir, basic_schemas())
            broken = schema(DCS_NS, '<xs:complexType name="DataCompositionSchema"><xs:restriction/></xs:complexType>')
            replacement = runtime_archive(replacement_dir, [(DCS_NS, broken), basic_schemas()[1]])
            profile = test_profile(archive)
            real_private_copy = verifier._private_copy_with_sha256

            def copy_then_replace(source, destination, label):
                digest = real_private_copy(source, destination, label)
                shutil.copyfile(replacement, source)
                return digest

            with mock.patch.object(verifier, "_private_copy_with_sha256", side_effect=copy_then_replace):
                runtime = verifier.verified_runtime(archive, profile)
            runtime.close()

            jar = edt_jar(original_dir)
            profile["edt"] = {
                "sha256": sha(jar.read_bytes()), "symbolicName": "test", "version": "1",
                "entries": {"backend": "backend.xdto"},
                "declarations": {"metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]}},
            }
            invalid_jar = replacement_dir / "invalid.jar"
            invalid_jar.write_bytes(b"not a jar")

            def copy_jar_then_replace(source, destination, label):
                digest = real_private_copy(source, destination, label)
                shutil.copyfile(invalid_jar, source)
                return digest

            verified_runner = lambda *_args, **_kwargs: subprocess.CompletedProcess([], 0, stdout="jar verified.\n", stderr="")
            with mock.patch.object(verifier, "_private_copy_with_sha256", side_effect=copy_jar_then_replace):
                edt = verifier.verified_edt(jar, profile, runner=verified_runner)
            self.assertTrue(edt["provided"])

    def test_dcs_and_rights_wrappers_preserve_schema_content(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            runtime = verifier.verified_runtime(archive, test_profile(archive))
            self.assertIsNone(verifier.strict_xsd_error(runtime, DCS_NS, "DataCompositionSchema", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>"))
            self.assertIn("name", verifier.strict_xsd_error(runtime, DCS_NS, "DataCompositionSchema", "<DataCompositionSchema xmlns='urn:test:dcs'/>"))
            self.assertIn("setForNewObjects", verifier.strict_xsd_error(runtime, ROLE_NS, "Rights", "<Rights xmlns='urn:test:roles' version='2.20'/>"))

    def test_corpus_contract_and_all_exit_classes(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)

            corpus = write_corpus(root / "strict", [("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {})])
            report, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 0)
            self.assertEqual((report["verdict"], report["exitCode"]), ("pass", 0))
            self.assertTrue(verifier.report_matches_schema(report, json.loads(REPORT_SCHEMA.read_text())))

            corpus = write_corpus(root / "entity-version", [(
                "rights.xml",
                "<Rights xmlns='urn:test:roles' version='2.&#50;0'><setForNewObjects>true</setForNewObjects></Rights>",
                {},
            )])
            entity_report, status = verifier.verify_corpus(
                corpus, profile, runtime, None
            )
            self.assertEqual(status, 1)
            version_check = next(
                check
                for check in entity_report["files"][0]["checks"]
                if check["name"] == "exportVersion"
            )
            self.assertEqual(version_check["status"], "fail")
            self.assertEqual(version_check["actual"], "2.&#50;0")

            corpus = write_corpus(root / "bad", [("rights.xml", "<Rights xmlns='urn:test:roles' version='2.19'/>", {})])
            failed_report, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 1)
            self.assertTrue(verifier.report_matches_schema(failed_report, json.loads(REPORT_SCHEMA.read_text())))

            corpus = write_corpus(root / "inconclusive", [("mxl.xml", "<document xmlns='urn:test:mxl'/>", {"newStandalone": True})])
            report, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 3)
            self.assertEqual((report["verdict"], report["exitCode"]), ("inconclusive", 3))
            self.assertEqual(report["files"][0]["result"], "inconclusive")
            self.assertTrue(verifier.report_matches_schema(report, json.loads(REPORT_SCHEMA.read_text())))

            source_report = verifier._error_report(profile["profile"], verifier.SourceError("missing evidence"))
            self.assertTrue(verifier.report_matches_schema(source_report, json.loads(REPORT_SCHEMA.read_text())))

            profile["runtime"]["sha256"] = "f" * 64
            with self.assertRaises(verifier.SourceError):
                verifier.verified_runtime(archive, profile)

    def test_owner_version_qname_and_corpus_tampering_are_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)
            corpus = write_corpus(root / "owned", [
                ("owner.xml", "<MetaDataObject xmlns='urn:test:md' version='2.19'><Configuration/></MetaDataObject>", {}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/owner.xml"}),
            ])
            _, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 1)

            corpus = write_corpus(root / "wrong-root", [("wrong.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {"family": "dcs", "newStandalone": True})])
            report, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 1)
            self.assertEqual(next(check for check in report["files"][0]["checks"] if check["name"] == "rootQName")["status"], "fail")

            corpus = write_corpus(root / "qname", [("owner.xml", "<MetaDataObject xmlns='urn:test:md' version='2.20' xmlns:xsi='http://www.w3.org/2001/XMLSchema-instance' xsi:type='missing:T'/>", {})])
            _, status = verifier.verify_corpus(corpus, profile, runtime, {"declarations": {"metadata": True}})
            self.assertEqual(status, 1)

            corpus = write_corpus(root / "text-qname", [("owner.xml", "<MetaDataObject xmlns='urn:test:md' xmlns:v8='http://v8.1c.ru/8.1/data/core' version='2.20'><v8:TypeSet>missing:T</v8:TypeSet></MetaDataObject>", {})])
            _, status = verifier.verify_corpus(corpus, profile, runtime, {"declarations": {"metadata": True}})
            self.assertEqual(status, 1)

            corpus = write_corpus(root / "unlisted", [("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {})])
            (corpus.parent / "case/extra.xml").write_text("<x/>")
            with self.assertRaisesRegex(verifier.CorpusError, "unlisted XML"):
                verifier.verify_corpus(corpus, profile, runtime, None)

    def test_pre_snapshot_inventory_shape_family_and_version_are_verified(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)

            def seeded_corpus(name):
                return write_corpus(
                    root / name,
                    [
                        (
                            "owner.xml",
                            "<MetaDataObject xmlns='urn:test:md' version='2.20'><Configuration/></MetaDataObject>",
                            {"seed": True},
                        )
                    ],
                )

            listed_pre = seeded_corpus("listed-pre")
            report, status = verifier.verify_corpus(
                listed_pre,
                profile,
                runtime,
                {"declarations": {"metadata": True}},
            )
            self.assertEqual(status, 3)
            self.assertEqual(
                [row["path"] for row in report["files"]],
                ["case/owner.xml", "pre-case/owner.xml"],
            )

            corpus = seeded_corpus("old-pre")
            manifest = json.loads(corpus.read_text())
            pre_path = corpus.parent / manifest["cases"][0]["preFiles"][0]["path"]
            pre_path.write_text(
                "<MetaDataObject xmlns='urn:test:md' version='2.19'><Configuration/></MetaDataObject>"
            )
            new_hash = sha(pre_path.read_bytes())
            manifest["cases"][0]["preFiles"][0]["sha256"] = new_hash
            corpus.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(
                verifier.CorpusError, "preOwnerVersions|pre-snapshot owners"
            ):
                verifier.verify_corpus(corpus, profile, runtime, None)

            mutations = {
                "removed": lambda manifest: manifest["cases"][0]["preFiles"].clear(),
                "wrong-family": lambda manifest: manifest["cases"][0]["preFiles"][0].update(
                    {"family": "dcs"}
                ),
                "bad-shape": lambda manifest: manifest["cases"][0]["preFiles"][0].update(
                    {"newStandalone": False}
                ),
                "boolean-hash": lambda manifest: manifest["cases"][0]["preFiles"][0].update(
                    {"sha256": True}
                ),
            }
            for label, mutate in mutations.items():
                with self.subTest(label=label):
                    corpus = seeded_corpus(label)
                    manifest = json.loads(corpus.read_text())
                    mutate(manifest)
                    corpus.write_text(json.dumps(manifest))
                    with self.assertRaises(verifier.CorpusError):
                        verifier.verify_corpus(corpus, profile, runtime, None)

            corpus = seeded_corpus("tampered")
            manifest = json.loads(corpus.read_text())
            pre_path = corpus.parent / manifest["cases"][0]["preFiles"][0]["path"]
            pre_path.write_text(
                "<MetaDataObject xmlns='urn:test:md' version='2.20'><Configuration><Changed/></Configuration></MetaDataObject>"
            )
            with self.assertRaisesRegex(verifier.CorpusError, "hash mismatch"):
                verifier.verify_corpus(corpus, profile, runtime, None)
            runtime.close()

    def test_qname_text_scan_uses_schema_expanded_names_not_local_names(self):
        verifier = load_verifier()
        core = verifier.etree.fromstring(
            b'<r xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:Type>missing:T</v8:Type></r>'
        )
        plain = verifier.etree.fromstring(
            b'<r xmlns:l="http://v8.1c.ru/8.3/xcf/logform"><l:Type>missing:CommandBarButton</l:Type><ValueType>missing:T</ValueType></r>'
        )

        self.assertEqual(verifier._unresolved_qname_prefix(core), "missing")
        self.assertIsNone(verifier._unresolved_qname_prefix(plain))

    def test_owner_path_requires_a_same_case_source_set_owner_descriptor(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)

            fake_owner = write_corpus(root / "fake-owner", [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/rights.xml"}),
            ])
            with self.assertRaisesRegex(verifier.CorpusError, "source-set owner|descriptor"):
                verifier.verify_corpus(fake_owner, profile, runtime, None)

            mislabeled_owner = write_corpus(root / "mislabeled-owner", [
                ("owner.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {"family": "metadata"}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/owner.xml"}),
            ])
            with self.assertRaisesRegex(verifier.CorpusError, "source-set owner|descriptor"):
                verifier.verify_corpus(mislabeled_owner, profile, runtime, {"declarations": {"metadata": True}})

            valid_owner = write_corpus(root / "valid-owner", [
                ("owner.xml", "<MetaDataObject xmlns='urn:test:md' version='2.20'><Configuration/></MetaDataObject>", {}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/owner.xml"}),
            ])
            report, status = verifier.verify_corpus(valid_owner, profile, runtime, {"declarations": {"metadata": True}})
            self.assertEqual(status, 3)
            self.assertEqual(next(row for row in report["files"] if row["path"].endswith("dcs.xml"))["result"], "pass")

            object_owner = write_corpus(root / "object-owner", [
                ("owner.xml", "<MetaDataObject xmlns='urn:test:md' version='2.20'><Catalog/></MetaDataObject>", {}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/owner.xml"}),
            ])
            with self.assertRaisesRegex(verifier.CorpusError, "source-set owner|descriptor type"):
                verifier.verify_corpus(object_owner, profile, runtime, {"declarations": {"metadata": True}})

            cross_root = root / "cross-case"
            (cross_root / "one").mkdir(parents=True)
            (cross_root / "two").mkdir(parents=True)
            (cross_root / "pre-one").mkdir()
            (cross_root / "pre-two").mkdir()
            dcs_path = cross_root / "one/dcs.xml"
            owner_path = cross_root / "two/owner.xml"
            dcs_path.write_text("<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>")
            owner_path.write_text("<MetaDataObject xmlns='urn:test:md' version='2.20'><Configuration/></MetaDataObject>")
            cross_manifest = {
                "schemaVersion": 2,
                "profile": "test-2.20",
                "emptyDirectoryPaths": ["pre-one", "pre-two"],
                "cases": [
                    {"id": "one", "workspacePath": "one", "preSnapshotPath": "pre-one",
                     "toolId": "unica.dcs.edit", "xmlImpact": "modified",
                     "preFiles": [], "preOwnerVersions": {}, "files": [{
                        "path": "one/dcs.xml", "sha256": sha(dcs_path.read_bytes()), "seed": False,
                        "family": "dcs", "ownerPath": "two/owner.xml",
                    }]},
                    {"id": "two", "workspacePath": "two", "preSnapshotPath": "pre-two",
                     "toolId": "unica.seed", "xmlImpact": "unchanged",
                     "preFiles": [], "preOwnerVersions": {}, "files": [{
                        "path": "two/owner.xml", "sha256": sha(owner_path.read_bytes()), "seed": True,
                        "family": "metadata",
                    }]},
                ],
            }
            cross_path = cross_root / "corpus-manifest.json"
            cross_path.write_text(json.dumps(cross_manifest))
            with self.assertRaisesRegex(verifier.CorpusError, "same case|source-set owner"):
                verifier.verify_corpus(cross_path, profile, runtime, {"declarations": {"metadata": True}})
            runtime.close()

    def test_client_application_interface_uses_configuration_owner_version(self):
        verifier = load_verifier()
        profile = json.loads(PROFILE.read_text())
        profile["profile"] = "test-fixed-families"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = root / "case"
            case.mkdir()
            (root / "pre-case").mkdir()
            owner = case / "Configuration.xml"
            owner.write_text(
                f"<MetaDataObject xmlns='{REAL_MD_NS}' version='2.20'><Configuration/></MetaDataObject>"
            )
            interface = case / "ClientApplicationInterface.xml"
            interface.write_text(f"<ClientApplicationInterface xmlns='{REAL_CAI_NS}' uuid='00000000-0000-0000-0000-000000000000'/>")
            manifest = {
                "schemaVersion": 2,
                "profile": profile["profile"],
                "emptyDirectoryPaths": ["pre-case"],
                "cases": [{
                    "id": "configuration-interface",
                    "workspacePath": "case",
                    "preSnapshotPath": "pre-case",
                    "toolId": "unica.interface.edit",
                    "xmlImpact": "modified",
                    "preFiles": [],
                    "preOwnerVersions": {},
                    "files": [
                        {
                            "path": "case/Configuration.xml",
                            "sha256": sha(owner.read_bytes()),
                            "seed": True,
                            "family": "metadata",
                        },
                        {
                            "path": "case/ClientApplicationInterface.xml",
                            "sha256": sha(interface.read_bytes()),
                            "seed": False,
                            "family": "client-application-interface",
                            "ownerPath": "case/Configuration.xml",
                        },
                    ],
                }],
            }
            manifest_path = root / "corpus-manifest.json"
            manifest_path.write_text(json.dumps(manifest))
            runtime = {
                "source": {
                    "kind": "runtime-xsd",
                    "sha256": "0" * 64,
                    "formatVersion": 1,
                    "platformVersion": "8.3.27.2074",
                    "manifestSummary": {"packages": 1, "namespaces": 1, "success": 1, "schemas": 1, "errors": 0},
                    "identityStatement": "synthetic",
                },
                "compilationMatrix": [{
                    "file": "synthetic.xsd",
                    "targetNamespace": "urn:synthetic",
                    "status": "compiled",
                    "detail": "",
                }],
                "wrappers": {},
            }
            edt = {
                "provided": True,
                "sha256": "0" * 64,
                "bundleSymbolicName": "synthetic",
                "bundleVersion": "synthetic",
                "evidenceScope": "synthetic",
                "identityStatement": "synthetic",
                "declarations": {"metadata": True},
            }
            report, status = verifier.verify_corpus(manifest_path, profile, runtime, edt)
            self.assertEqual(status, 3)
            interface_row = next(row for row in report["files"] if row["path"].endswith("ClientApplicationInterface.xml"))
            self.assertEqual(next(check for check in interface_row["checks"] if check["name"] == "ownerVersion")["status"], "pass")
            self.assertNotIn("exportVersion", {check["name"] for check in interface_row["checks"]})

    def test_flowchart_is_known_not_covered_and_requires_root_version_2_20(self):
        verifier = load_verifier()
        profile = json.loads(PROFILE.read_text())
        profile["profile"] = "test-fixed-families"
        self.assertIn(REAL_FLOWCHART_QNAME, profile["families"])
        self.assertEqual(
            profile["families"][REAL_FLOWCHART_QNAME],
            {"id": "flowchart", "coverage": "not-covered", "version": "root"},
        )
        runtime = {
            "source": {
                "kind": "runtime-xsd",
                "sha256": "0" * 64,
                "formatVersion": 1,
                "platformVersion": "8.3.27.2074",
                "manifestSummary": {"packages": 1, "namespaces": 1, "success": 1, "schemas": 1, "errors": 0},
                "identityStatement": "synthetic",
            },
            "compilationMatrix": [{
                "file": "synthetic.xsd", "targetNamespace": "urn:synthetic",
                "status": "compiled", "detail": "",
            }],
            "wrappers": {},
        }

        def verify(xml):
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = root / "case"
                case.mkdir()
                (root / "pre-case").mkdir()
                path = case / "Flowchart.xml"
                path.write_text(xml)
                manifest = root / "corpus-manifest.json"
                manifest.write_text(json.dumps({
                    "schemaVersion": 2,
                    "profile": profile["profile"],
                    "emptyDirectoryPaths": ["pre-case"],
                    "cases": [{
                        "id": "flowchart", "workspacePath": "case", "preSnapshotPath": "pre-case",
                        "toolId": "unica.meta.compile", "xmlImpact": "created",
                        "preFiles": [], "preOwnerVersions": {},
                        "files": [{
                            "path": "case/Flowchart.xml", "sha256": sha(path.read_bytes()),
                            "seed": False, "family": "flowchart",
                        }],
                    }],
                }))
                return verifier.verify_corpus(manifest, profile, runtime, None)

        report, status = verify(
            f"<GraphicalSchema xmlns='{REAL_SCHEME_NS}' version='2.20'/>"
        )
        self.assertEqual(status, 3)
        self.assertEqual((report["files"][0]["coverage"], report["files"][0]["result"]), ("not-covered", "inconclusive"))
        self.assertNotIn("edtDeclaration", {check["name"] for check in report["files"][0]["checks"]})

        report, status = verify(
            f"<GraphicalSchema xmlns='{REAL_SCHEME_NS}' version='2.19'/>"
        )
        self.assertEqual(status, 1)
        self.assertEqual(next(check for check in report["files"][0]["checks"] if check["name"] == "exportVersion")["status"], "fail")

        report, status = verify(
            f"<NotGraphicalSchema xmlns='{REAL_SCHEME_NS}' version='2.20'/>"
        )
        self.assertEqual(status, 1)
        self.assertEqual(next(check for check in report["files"][0]["checks"] if check["name"] == "rootQName")["status"], "fail")

    def test_corpus_requires_complete_cases_and_canonical_unique_files(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = write_corpus(root / "base", [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            original = json.loads(corpus.read_text())

            for field in ("id", "toolId", "xmlImpact"):
                changed = json.loads(json.dumps(original))
                changed["cases"][0].pop(field)
                corpus.write_text(json.dumps(changed))
                with self.subTest(missing=field):
                    with self.assertRaisesRegex(verifier.CorpusError, field):
                        verifier._load_corpus(corpus, "test-2.20")

            changed = json.loads(json.dumps(original))
            changed["cases"] = None
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "cases"):
                verifier._load_corpus(corpus, "test-2.20")

            changed = json.loads(json.dumps(original))
            alias = dict(changed["cases"][0]["files"][0])
            alias["path"] = "case/./rights.xml"
            changed["cases"][0]["files"].append(alias)
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "canonical|duplicate"):
                verifier._load_corpus(corpus, "test-2.20")

            changed = json.loads(json.dumps(original))
            changed["cases"][0]["files"][0]["ownerPath"] = ""
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "ownerPath"):
                verifier._load_corpus(corpus, "test-2.20")

            hardlink_root = root / "hardlink"
            hardlink_corpus = write_corpus(hardlink_root, [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            os.link(hardlink_root / "case/rights.xml", hardlink_root / "case/rights-copy.xml")
            hardlink_data = json.loads(hardlink_corpus.read_text())
            hardlink_entry = dict(hardlink_data["cases"][0]["files"][0])
            hardlink_entry["path"] = "case/rights-copy.xml"
            hardlink_data["cases"][0]["files"].append(hardlink_entry)
            hardlink_corpus.write_text(json.dumps(hardlink_data))
            with self.assertRaisesRegex(verifier.CorpusError, "same file|duplicate"):
                verifier._load_corpus(hardlink_corpus, "test-2.20")

            corpus.write_text(json.dumps(original))
            real_os_open = os.open

            def deny_xml_open(path, flags, mode=0o777, *, dir_fd=None):
                if Path(path).suffix.lower() == ".xml":
                    raise PermissionError("denied")
                if dir_fd is None:
                    return real_os_open(path, flags, mode)
                return real_os_open(path, flags, mode, dir_fd=dir_fd)

            with mock.patch("os.open", side_effect=deny_xml_open):
                with self.assertRaisesRegex(
                    verifier.CorpusError,
                    "capture|snapshot|read",
                ):
                    verifier._load_corpus(corpus, "test-2.20")

    def test_corpus_schema_v2_binds_exact_empty_directory_topology(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = write_corpus(root, [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            (root / "case/Ext/Empty").mkdir(parents=True)
            manifest = json.loads(corpus.read_text())
            manifest["schemaVersion"] = 2
            manifest["emptyDirectoryPaths"] = ["case/Ext/Empty", "pre-case"]
            corpus.write_text(json.dumps(manifest))

            loaded, loaded_root, files = verifier._load_corpus(corpus, "test-2.20")

            self.assertEqual(loaded["schemaVersion"], 2)
            self.assertEqual(loaded_root.resolve(), root.resolve())
            self.assertEqual(len(files), 1)

            for declared in (
                [],
                ["case/Ext/Empty"],
                ["case/Ext/Empty", "pre-case", "pre-case"],
                ["pre-case", "case/Ext/Empty"],
                ["case/Ext/Empty", "missing"],
                ["../escape", "pre-case"],
                [r"case\Ext\Empty", "pre-case"],
            ):
                changed = json.loads(json.dumps(manifest))
                changed["emptyDirectoryPaths"] = declared
                corpus.write_text(json.dumps(changed))
                with self.subTest(declared=declared):
                    with self.assertRaisesRegex(
                        verifier.CorpusError,
                        "empty director|sorted|canonical",
                    ):
                        verifier._load_corpus(corpus, "test-2.20")

            missing_inventory = json.loads(json.dumps(manifest))
            missing_inventory.pop("emptyDirectoryPaths")
            corpus.write_text(json.dumps(missing_inventory))
            with self.assertRaisesRegex(verifier.CorpusError, "emptyDirectoryPaths"):
                verifier._load_corpus(corpus, "test-2.20")

            unknown_field = json.loads(json.dumps(manifest))
            unknown_field["unexpected"] = True
            corpus.write_text(json.dumps(unknown_field))
            with self.assertRaisesRegex(verifier.CorpusError, "manifest.*shape|keys"):
                verifier._load_corpus(corpus, "test-2.20")

            legacy = json.loads(json.dumps(manifest))
            legacy["schemaVersion"] = 1
            legacy.pop("emptyDirectoryPaths")
            corpus.write_text(json.dumps(legacy))
            with self.assertRaisesRegex(verifier.CorpusError, "schemaVersion"):
                verifier._load_corpus(corpus, "test-2.20")

    def test_corpus_schema_v2_is_rechecked_after_topology_capture(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = write_corpus(root, [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            removed_after_capture = root / "case/Ext/Empty"
            removed_after_capture.mkdir(parents=True)
            manifest = json.loads(corpus.read_text())
            manifest["emptyDirectoryPaths"] = ["case/Ext/Empty", "pre-case"]
            corpus.write_text(json.dumps(manifest))
            real_capture = verifier._corpus_empty_directory_paths

            def capture_then_remove(path):
                captured = real_capture(path)
                if removed_after_capture.exists():
                    removed_after_capture.rmdir()
                return captured

            with mock.patch.object(
                verifier,
                "_corpus_empty_directory_paths",
                side_effect=capture_then_remove,
            ):
                with self.assertRaisesRegex(
                    verifier.CorpusError,
                    "empty director|topology|changed",
                ):
                    verifier._load_corpus(corpus, "test-2.20")

    def test_corpus_snapshot_rejects_path_replacement_between_stat_and_open(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as outside_tmp:
            root = Path(tmp)
            corpus = write_corpus(root, [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            target = root / "case/rights.xml"
            declared_payload = target.read_bytes()
            invalid_payload = declared_payload.replace(b"2.20", b"2.19")
            target.write_bytes(invalid_payload)
            outside = Path(outside_tmp) / "declared.xml"
            outside.write_bytes(declared_payload)
            real_path_open = Path.open
            real_os_open = os.open
            attacked = False

            def restore_target():
                if target.is_symlink():
                    target.unlink()
                target.write_bytes(invalid_payload)

            def path_open_with_replacement(path, *args, **kwargs):
                nonlocal attacked
                if Path(path) == target and not attacked:
                    attacked = True
                    target.unlink()
                    target.symlink_to(outside)
                    try:
                        stream = real_path_open(path, *args, **kwargs)
                    finally:
                        restore_target()
                    return stream
                return real_path_open(path, *args, **kwargs)

            def os_open_with_replacement(path, flags, mode=0o777, *, dir_fd=None):
                nonlocal attacked
                if Path(path) == target and not attacked:
                    attacked = True
                    target.unlink()
                    target.symlink_to(outside)
                    try:
                        if dir_fd is None:
                            return real_os_open(path, flags, mode)
                        return real_os_open(path, flags, mode, dir_fd=dir_fd)
                    finally:
                        restore_target()
                if dir_fd is None:
                    return real_os_open(path, flags, mode)
                return real_os_open(path, flags, mode, dir_fd=dir_fd)

            with mock.patch.object(
                Path, "open", new=path_open_with_replacement
            ), mock.patch("os.open", side_effect=os_open_with_replacement):
                with self.assertRaisesRegex(
                    verifier.CorpusError,
                    "changed|identity|symlink|snapshot",
                ):
                    verifier._load_corpus(corpus, "test-2.20")
            self.assertTrue(attacked)
            self.assertEqual(target.read_bytes(), invalid_payload)

    def test_fixed_profile_uses_the_canonical_platform_corpus_schema(self):
        verifier = load_verifier()
        from tests.dev.test_verify_8_3_27_platform import (
            load_verifier as load_platform_verifier,
            write_manifest,
            write_platform_case,
        )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case_ids = sorted(load_platform_verifier().MANDATORY_CASE_IDS)
            cases = [write_platform_case(root, case_id) for case_id in case_ids]
            manifest_path = write_manifest(root, cases)
            manifest = json.loads(manifest_path.read_text())
            manifest["cases"][0]["gateVerdict"] = "pass-from-case"
            manifest_path.write_text(json.dumps(manifest, sort_keys=True))

            with self.assertRaisesRegex(
                verifier.CorpusError,
                "unknown.*case|fields.*exact",
            ):
                verifier._load_corpus(
                    manifest_path,
                    "1c-8.3.27-export-2.20",
                )

    def test_corpus_hash_and_validation_use_the_same_immutable_snapshot(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)
            real_snapshot = verifier._read_corpus_snapshot

            rights_corpus = write_corpus(root / "rights-swap", [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            original_sha = json.loads(rights_corpus.read_text())["cases"][0]["files"][0]["sha256"]

            def snapshot_then_swap_rights(path, raw):
                payload, digest = real_snapshot(path, raw)
                path.write_text("<Rights xmlns='urn:test:roles' version='2.19'><setForNewObjects>invalid</setForNewObjects></Rights>")
                return payload, digest

            with mock.patch.object(verifier, "_read_corpus_snapshot", side_effect=snapshot_then_swap_rights):
                report, status = verifier.verify_corpus(rights_corpus, profile, runtime, None)
            self.assertEqual(status, 0)
            self.assertEqual(report["files"][0]["sha256"], original_sha)
            self.assertIn("version='2.19'", (rights_corpus.parent / "case/rights.xml").read_text())
            self.assertEqual(next(check for check in report["files"][0]["checks"] if check["name"] == "runtimeXsd")["status"], "pass")

            owner_corpus = write_corpus(root / "owner-swap", [
                ("owner.xml", "<MetaDataObject xmlns='urn:test:md' version='2.20'><Configuration/></MetaDataObject>", {}),
                ("dcs.xml", "<DataCompositionSchema xmlns='urn:test:dcs'><name>x</name></DataCompositionSchema>", {"ownerPath": "case/owner.xml"}),
            ])

            def snapshot_then_swap_owner(path, raw):
                payload, digest = real_snapshot(path, raw)
                if raw == "case/owner.xml":
                    path.write_text("<MetaDataObject xmlns='urn:test:md' version='2.19'><Catalog/></MetaDataObject>")
                return payload, digest

            with mock.patch.object(verifier, "_read_corpus_snapshot", side_effect=snapshot_then_swap_owner):
                report, status = verifier.verify_corpus(
                    owner_corpus,
                    profile,
                    runtime,
                    {"declarations": {"metadata": True}},
                )
            self.assertEqual(status, 3)
            dcs_row = next(row for row in report["files"] if row["path"] == "case/dcs.xml")
            owner_check = next(check for check in dcs_row["checks"] if check["name"] == "ownerVersion")
            self.assertEqual((owner_check["status"], owner_check["actual"]), ("pass", "2.20"))
            runtime.close()

    def test_qname_scan_ignores_comments_processing_instructions_and_entities(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)
            corpus = write_corpus(root / "non-elements", [(
                "owner.xml",
                """<!DOCTYPE MetaDataObject [<!ENTITY label "value">]>
<MetaDataObject xmlns='urn:test:md' version='2.20'>
  <Configuration><!-- comment --><?review instruction?><Name>&label;</Name></Configuration>
</MetaDataObject>""",
                {},
            )])
            try:
                report, status = verifier.verify_corpus(
                    corpus,
                    profile,
                    runtime,
                    {"declarations": {"metadata": True}},
                )
            except Exception as error:
                self.fail(f"valid non-element XML nodes must not crash QName scanning: {error!r}")
            self.assertEqual(status, 3)
            self.assertEqual(report["files"][0]["result"], "inconclusive")
            self.assertEqual(
                next(check for check in report["files"][0]["checks"] if check["name"] == "qnamePrefixes")["status"],
                "pass",
            )
            runtime.close()

    def test_malformed_corpus_cli_is_exit_2_with_a_valid_report(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = root / "corpus-manifest.json"
            corpus.write_text(json.dumps({
                "schemaVersion": 2,
                "profile": "1c-8.3.27-export-2.20",
                "emptyDirectoryPaths": [],
                "cases": None,
            }))
            report_path = root / "report.json"
            runtime = verifier.RuntimeEvidence({"source": {}, "compilationMatrix": [], "wrappers": {}})
            with mock.patch.object(verifier, "verified_runtime", return_value=runtime), mock.patch.object(
                verifier, "verified_edt", return_value={"provided": True}
            ):
                status = verifier.main([
                    "--runtime-xsd-zip", str(root / "runtime.zip"),
                    "--edt-xdto-jar", str(root / "edt.jar"),
                    "--corpus", str(corpus),
                    "--report", str(report_path),
                ])
            self.assertEqual(status, 2)
            report = json.loads(report_path.read_text())
            self.assertEqual((report["verdict"], report["exitCode"]), ("source-error", 2))
            self.assertTrue(verifier.report_matches_schema(report, json.loads(REPORT_SCHEMA.read_text())))

    def test_boolean_corpus_schema_version_is_a_deterministic_source_error(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = root / "corpus-manifest.json"
            corpus.write_text(json.dumps({
                "schemaVersion": True,
                "profile": "1c-8.3.27-export-2.20",
                "emptyDirectoryPaths": [],
                "cases": [],
            }))
            reports = []
            for index in range(2):
                report_path = root / f"report-{index}.json"
                runtime = verifier.RuntimeEvidence({"source": {}, "compilationMatrix": [], "wrappers": {}})
                with mock.patch.object(verifier, "verified_runtime", return_value=runtime), mock.patch.object(
                    verifier, "verified_edt", return_value={"provided": True}
                ):
                    status = verifier.main([
                        "--runtime-xsd-zip", str(root / "runtime.zip"),
                        "--edt-xdto-jar", str(root / "edt.jar"),
                        "--corpus", str(corpus),
                        "--report", str(report_path),
                    ])
                report = json.loads(report_path.read_text())
                self.assertEqual(status, 2)
                self.assertIn("schemaVersion", report["sourceError"])
                self.assertTrue(verifier.report_matches_schema(report, schema_value))
                reports.append(report)
            self.assertEqual(reports[0], reports[1])

    def test_malformed_profile_cli_is_exit_2_with_a_valid_report(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = root / "case"
            case.mkdir()
            xml = case / "x.xml"
            xml.write_text("<x/>")
            corpus = root / "corpus-manifest.json"
            corpus.write_text(json.dumps({
                "schemaVersion": 2,
                "profile": "broken-profile",
                "emptyDirectoryPaths": [],
                "cases": [{
                    "id": "case", "toolId": "unica.test", "xmlImpact": "created",
                    "files": [{"path": "case/x.xml", "sha256": sha(xml.read_bytes()), "seed": False, "family": "x"}],
                }],
            }))
            missing_families = {
                "profile": "broken-profile",
                "exportVersion": "2.20",
                "runtime": {},
                "edt": {},
                "families": None,
            }
            unhashable_owner_type = json.loads(PROFILE.read_text())
            unhashable_owner_type["profile"] = "broken-profile"
            owner_family = next(
                family
                for family in unhashable_owner_type["families"].values()
                if family.get("sourceSetOwner") is True
            )
            owner_family["sourceSetOwnerTypes"] = [{}]
            null_known_failures = json.loads(PROFILE.read_text())
            null_known_failures["profile"] = "broken-profile"
            null_known_failures["runtime"]["knownCompileFailures"] = None
            malformed_external_imports = json.loads(PROFILE.read_text())
            malformed_external_imports["profile"] = "broken-profile"
            malformed_external_imports["runtime"]["knownExternalImports"] = {}
            boolean_summary_count = json.loads(PROFILE.read_text())
            boolean_summary_count["profile"] = "broken-profile"
            boolean_summary_count["runtime"]["summary"]["schemas"] = True
            malformed_edt_declaration = json.loads(PROFILE.read_text())
            malformed_edt_declaration["profile"] = "broken-profile"
            malformed_edt_declaration["edt"]["declarations"]["metadata"]["tokens"] = None
            malformed_coverage_list = json.loads(PROFILE.read_text())
            malformed_coverage_list["profile"] = "broken-profile"
            next(iter(malformed_coverage_list["families"].values()))["coverage"] = []
            malformed_coverage_object = json.loads(PROFILE.read_text())
            malformed_coverage_object["profile"] = "broken-profile"
            next(iter(malformed_coverage_object["families"].values()))["coverage"] = {}
            malformed_version_list = json.loads(PROFILE.read_text())
            malformed_version_list["profile"] = "broken-profile"
            next(iter(malformed_version_list["families"].values()))["version"] = []
            malformed_version_object = json.loads(PROFILE.read_text())
            malformed_version_object["profile"] = "broken-profile"
            next(iter(malformed_version_object["families"].values()))["version"] = {}
            broken_profile = root / "profile.json"
            report_path = root / "report.json"
            runtime = verifier.RuntimeEvidence({"source": {}, "compilationMatrix": [], "wrappers": {}})
            for label, value, expected_error in (
                ("missing families", missing_families, "families"),
                ("unhashable owner type", unhashable_owner_type, "sourceSetOwnerTypes"),
                ("null known failures", null_known_failures, "knownCompileFailures"),
                ("malformed external imports", malformed_external_imports, "knownExternalImports"),
                ("boolean summary count", boolean_summary_count, "summary"),
                ("malformed EDT declaration", malformed_edt_declaration, "declaration"),
                ("coverage list", malformed_coverage_list, "coverage"),
                ("coverage object", malformed_coverage_object, "coverage"),
                ("version list", malformed_version_list, "version mode"),
                ("version object", malformed_version_object, "version mode"),
            ):
                with self.subTest(label=label):
                    broken_profile.write_text(json.dumps(value))
                    with mock.patch.object(verifier, "verified_runtime", return_value=runtime) as runtime_check, mock.patch.object(
                        verifier, "verified_edt", return_value={"provided": True}
                    ):
                        try:
                            status = verifier.main([
                                "--runtime-xsd-zip", str(root / "runtime.zip"),
                                "--edt-xdto-jar", str(root / "edt.jar"),
                                "--corpus", str(corpus),
                                "--report", str(report_path),
                                "--profile", str(broken_profile),
                            ])
                        except Exception as error:
                            self.fail(f"malformed profile must produce a source-error report: {error!r}")
                    runtime_check.assert_not_called()
                    self.assertEqual(status, 2)
                    report = json.loads(report_path.read_text())
                    self.assertIn(expected_error, report["sourceError"])
                    self.assertTrue(verifier.report_matches_schema(report, json.loads(REPORT_SCHEMA.read_text())))

    def test_source_error_report_sanitizes_non_string_profile_ids(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            profile_path = root / "profile.json"
            report_path = root / "report.json"
            for profile_id in ([1], 123, {"bad": True}):
                value = json.loads(PROFILE.read_text())
                value["profile"] = profile_id
                profile_path.write_text(json.dumps(value))
                with self.subTest(profile_id=profile_id):
                    status = verifier.main([
                        "--runtime-xsd-zip", str(root / "runtime.zip"),
                        "--edt-xdto-jar", str(root / "edt.jar"),
                        "--corpus", str(root / "corpus.json"),
                        "--report", str(report_path),
                        "--profile", str(profile_path),
                    ])
                    report = json.loads(report_path.read_text())
                    self.assertEqual(status, 2)
                    self.assertEqual(report["profile"], "unknown")
                    self.assertTrue(verifier.report_matches_schema(report, schema_value))

    def test_report_is_deterministic(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)
            corpus = write_corpus(root / "corpus", [("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {})])
            first, _ = verifier.verify_corpus(corpus, profile, runtime, None)
            second, _ = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(json.dumps(first, sort_keys=True), json.dumps(second, sort_keys=True))

    def test_report_validation_rejects_empty_pass_and_inconsistent_summary(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())
        empty_pass = {
            "schemaVersion": 1,
            "profile": "test",
            "verdict": "pass",
            "exitCode": 0,
            "sources": {},
            "schemaCompilation": [],
            "files": [],
            "summary": {"files": -1, "passed": 99, "failed": -7, "inconclusive": 42},
        }
        self.assertFalse(verifier.report_matches_schema(empty_pass, schema_value))
        source_error_without_detail = {
            **empty_pass,
            "verdict": "source-error",
            "exitCode": 2,
            "summary": {"files": 0, "passed": 0, "failed": 0, "inconclusive": 0},
        }
        self.assertFalse(verifier.report_matches_schema(source_error_without_detail, schema_value))

    def test_schema_const_and_enum_distinguish_booleans_from_numbers(self):
        verifier = load_verifier()
        report_schema = json.loads(REPORT_SCHEMA.read_text())
        edt_choices = report_schema["properties"]["sources"]["properties"]["edt"]["oneOf"]
        false_schema = edt_choices[0]["properties"]["provided"]
        true_schema = edt_choices[1]["properties"]["provided"]

        self.assertTrue(verifier._matches_schema(False, false_schema))
        self.assertTrue(verifier._matches_schema(True, true_schema))
        self.assertFalse(verifier._matches_schema(0, false_schema))
        self.assertFalse(verifier._matches_schema(1, true_schema))
        self.assertFalse(verifier._matches_schema({"provided": 0}, {"const": {"provided": False}}))
        self.assertFalse(verifier._matches_schema(1, {"enum": [True]}))
        self.assertFalse(verifier._matches_schema(True, {"enum": [1]}))
        self.assertTrue(verifier._matches_schema(1.0, {"const": 1}))

    def test_report_semantics_require_check_result_and_coverage_consistency(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            runtime = verifier.verified_runtime(archive, profile)
            corpus = write_corpus(root / "corpus", [
                ("rights.xml", "<Rights xmlns='urn:test:roles' version='2.20'><setForNewObjects>true</setForNewObjects></Rights>", {}),
            ])
            valid, status = verifier.verify_corpus(corpus, profile, runtime, None)
            self.assertEqual(status, 0)
            self.assertTrue(verifier.report_matches_schema(valid, schema_value))

            failed_check_pass_row = json.loads(json.dumps(valid))
            failed_check_pass_row["files"][0]["checks"][0]["status"] = "fail"
            self.assertFalse(verifier.report_matches_schema(failed_check_pass_row, schema_value))

            advisory_pass_row = json.loads(json.dumps(valid))
            advisory_pass_row["files"][0]["coverage"] = "advisory"
            self.assertFalse(verifier.report_matches_schema(advisory_pass_row, schema_value))

            failed_without_failed_check = json.loads(json.dumps(valid))
            failed_without_failed_check["files"][0]["result"] = "fail"
            failed_without_failed_check["summary"] = {"files": 1, "passed": 0, "failed": 1, "inconclusive": 0}
            failed_without_failed_check["verdict"] = "fail"
            failed_without_failed_check["exitCode"] = 1
            self.assertFalse(verifier.report_matches_schema(failed_without_failed_check, schema_value))

            strict_unavailable = json.loads(json.dumps(valid))
            strict_unavailable["files"][0]["checks"][0]["status"] = "unavailable"
            strict_unavailable["files"][0]["result"] = "inconclusive"
            strict_unavailable["summary"] = {"files": 1, "passed": 0, "failed": 0, "inconclusive": 1}
            strict_unavailable["verdict"] = "inconclusive"
            strict_unavailable["exitCode"] = 3
            self.assertFalse(verifier.report_matches_schema(strict_unavailable, schema_value))

            advisory_inconclusive = json.loads(json.dumps(advisory_pass_row))
            advisory_inconclusive["files"][0]["result"] = "inconclusive"
            advisory_inconclusive["files"][0]["requiredNextEvidence"] = "platform roundtrip"
            advisory_inconclusive["summary"] = {"files": 1, "passed": 0, "failed": 0, "inconclusive": 1}
            advisory_inconclusive["verdict"] = "inconclusive"
            advisory_inconclusive["exitCode"] = 3
            self.assertTrue(verifier.report_matches_schema(advisory_inconclusive, schema_value))
            runtime.close()

    def test_main_converts_an_invalid_generated_report_to_source_error(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            invalid = {
                "schemaVersion": 1,
                "profile": "1c-8.3.27-export-2.20",
                "verdict": "pass",
                "exitCode": 0,
                "sources": {},
                "schemaCompilation": [],
                "files": [],
                "summary": {"files": 0, "passed": 0, "failed": 0, "inconclusive": 0},
            }
            runtime = verifier.RuntimeEvidence({"source": {}, "compilationMatrix": [], "wrappers": {}})
            report_path = root / "report.json"
            with mock.patch.object(verifier, "verified_runtime", return_value=runtime), mock.patch.object(
                verifier, "verified_edt", return_value={"provided": True}
            ), mock.patch.object(verifier, "verify_corpus", return_value=(invalid, 0)):
                status = verifier.main([
                    "--runtime-xsd-zip", str(root / "runtime.zip"),
                    "--edt-xdto-jar", str(root / "edt.jar"),
                    "--corpus", str(root / "corpus.json"),
                    "--report", str(report_path),
                ])
            self.assertEqual(status, 2)
            report = json.loads(report_path.read_text())
            self.assertEqual((report["verdict"], report["exitCode"]), ("source-error", 2))
            self.assertIn("report", report["sourceError"])
            self.assertTrue(verifier.report_matches_schema(report, json.loads(REPORT_SCHEMA.read_text())))

    def test_edt_declaration_is_line_evidence_not_xsd_coverage(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = runtime_archive(root, basic_schemas())
            profile = test_profile(archive)
            jar = edt_jar(root)
            profile["edt"] = {
                "sha256": sha(jar.read_bytes()), "symbolicName": "test", "version": "1",
                "entries": {"backend": "backend.xdto"},
                "declarations": {"metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]}}
            }

            def verified_runner(*_args, **_kwargs):
                return subprocess.CompletedProcess([], 0, stdout="jar verified.\n", stderr="")

            edt = verifier.verified_edt(jar, profile, runner=verified_runner)
            runtime = verifier.verified_runtime(archive, profile)
            corpus = write_corpus(root / "corpus", [("meta.xml", "<MetaDataObject xmlns='urn:test:md' version='2.20'/>", {})])
            report, status = verifier.verify_corpus(corpus, profile, runtime, edt)
            self.assertEqual(status, 3)
            self.assertEqual(report["files"][0]["coverage"], "not-covered")
            self.assertEqual(report["files"][0]["checks"][-1]["name"], "edtDeclaration")
            self.assertIn("not proof of patch build", report["sources"]["edt"]["evidenceScope"])

    def test_invalid_zip_and_unavailable_jarsigner_are_source_errors(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            invalid = root / "runtime.zip"
            invalid.write_bytes(b"not a zip")
            profile = test_profile(invalid)
            with self.assertRaisesRegex(verifier.SourceError, "ZIP|archive"):
                verifier.verified_runtime(invalid, profile)

            jar = edt_jar(root)
            profile["edt"] = {
                "sha256": sha(jar.read_bytes()), "symbolicName": "test", "version": "1",
                "entries": {"backend": "backend.xdto"},
                "declarations": {"metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]}},
            }

            def missing_jarsigner(*_args, **_kwargs):
                raise FileNotFoundError("jarsigner")

            with self.assertRaisesRegex(verifier.SourceError, "jarsigner"):
                verifier.verified_edt(jar, profile, runner=missing_jarsigner)

    def test_corrupt_or_encrypted_zip_members_produce_valid_source_error_reports(self):
        verifier = load_verifier()
        schema_value = json.loads(REPORT_SCHEMA.read_text())

        def assert_source_error_report(call, report_path):
            try:
                status = call()
            except Exception as error:
                self.fail(f"ZIP member read failure must be reported, not raised: {error!r}")
            self.assertEqual(status, 2)
            report = json.loads(report_path.read_text())
            self.assertEqual((report["verdict"], report["exitCode"]), ("source-error", 2))
            self.assertTrue(verifier.report_matches_schema(report, schema_value))
            self.assertRegex(report["sourceError"], "CRC|crc|encrypted|member|archive")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for mode in ("crc", "encrypted"):
                case_root = root / f"runtime-{mode}"
                case_root.mkdir()
                archive = runtime_archive(case_root, basic_schemas())
                corrupt_zip_member(archive, "export/schemas/0001.xsd", mode)
                with zipfile.ZipFile(archive) as opened:
                    self.assertIn("export/schemas/0001.xsd", opened.namelist())
                profile = test_profile(archive)
                profile["edt"] = {
                    "sha256": "0" * 64, "symbolicName": "test", "version": "1",
                    "entries": {"backend": "backend.xdto"},
                    "declarations": {
                        "metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]},
                    },
                }
                profile_path = case_root / "profile.json"
                profile_path.write_text(json.dumps(profile))
                report_path = case_root / "report.json"
                with self.subTest(source="runtime", mode=mode):
                    assert_source_error_report(
                        lambda: verifier.main([
                            "--runtime-xsd-zip", str(archive),
                            "--edt-xdto-jar", str(case_root / "unused.jar"),
                            "--corpus", str(case_root / "unused-corpus.json"),
                            "--report", str(report_path),
                            "--profile", str(profile_path),
                        ]),
                        report_path,
                    )

            for mode in ("crc", "encrypted"):
                case_root = root / f"edt-{mode}"
                case_root.mkdir()
                archive = runtime_archive(case_root, basic_schemas())
                jar = edt_jar(case_root)
                corrupt_zip_member(jar, "backend.xdto", mode)
                with zipfile.ZipFile(jar) as opened:
                    self.assertIn("backend.xdto", opened.namelist())
                profile = test_profile(archive)
                profile["edt"] = {
                    "sha256": sha(jar.read_bytes()), "symbolicName": "test", "version": "1",
                    "entries": {"backend": "backend.xdto"},
                    "declarations": {
                        "metadata": {"entry": "backend", "tokens": ["urn:test:md", "MetaDataObject"]},
                    },
                }
                profile_path = case_root / "profile.json"
                profile_path.write_text(json.dumps(profile))
                report_path = case_root / "report.json"
                real_verified_edt = verifier.verified_edt

                def verified_edt_with_test_runner(path, selected_profile):
                    runner = lambda *_args, **_kwargs: subprocess.CompletedProcess(
                        [], 0, stdout="jar verified.\n", stderr=""
                    )
                    return real_verified_edt(path, selected_profile, runner=runner)

                with self.subTest(source="edt", mode=mode), mock.patch.object(
                    verifier, "verified_edt", side_effect=verified_edt_with_test_runner
                ):
                    assert_source_error_report(
                        lambda: verifier.main([
                            "--runtime-xsd-zip", str(archive),
                            "--edt-xdto-jar", str(jar),
                            "--corpus", str(case_root / "unused-corpus.json"),
                            "--report", str(report_path),
                            "--profile", str(profile_path),
                        ]),
                        report_path,
                    )

    def test_corpus_rejects_escape_absolute_duplicate_and_hash_mismatch(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = write_corpus(root, [("x.xml", "<x/>", {})])
            original = corpus.read_text()
            data = json.loads(original)
            corpus.write_text(
                original.replace(
                    '"schemaVersion": 2,',
                    '"schemaVersion": 2, "schemaVersion": 2,',
                    1,
                )
            )
            with self.assertRaisesRegex(verifier.CorpusError, "duplicate JSON key"):
                verifier._load_corpus(corpus, "test-2.20")
            for raw in ["../x.xml", "/tmp/x.xml"]:
                changed = json.loads(json.dumps(data))
                changed["cases"][0]["files"][0]["path"] = raw
                corpus.write_text(json.dumps(changed))
                with self.assertRaisesRegex(verifier.CorpusError, "unsafe corpus path"):
                    verifier._load_corpus(corpus, "test-2.20")
            changed = json.loads(json.dumps(data))
            changed["cases"][0]["files"][0]["sha256"] = "0" * 64
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "hash mismatch"):
                verifier._load_corpus(corpus, "test-2.20")
            changed = json.loads(json.dumps(data))
            changed["cases"][0]["files"].append(dict(changed["cases"][0]["files"][0]))
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "duplicate"):
                verifier._load_corpus(corpus, "test-2.20")
            (root / "alias").symlink_to(root / "case", target_is_directory=True)
            changed = json.loads(json.dumps(data))
            changed["cases"][0]["files"][0]["path"] = "alias/x.xml"
            corpus.write_text(json.dumps(changed))
            with self.assertRaisesRegex(verifier.CorpusError, "symlink"):
                verifier._load_corpus(corpus, "test-2.20")

    def test_profile_and_schema_reader_rejects_duplicate_json_keys(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "duplicate.json"
            path.write_text('{"schemaVersion": 1, "schemaVersion": 2}')

            with self.assertRaisesRegex(verifier.SourceError, "duplicate JSON key"):
                verifier._read_json_object(path, "test object")


if __name__ == "__main__":
    unittest.main()
