import copy
import hashlib
import importlib.util
import json
import os
import sys
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dev/verify-8-3-27-platform.py"
_VERIFIER = None


def load_verifier():
    global _VERIFIER
    if _VERIFIER is not None:
        return _VERIFIER
    spec = importlib.util.spec_from_file_location("verify_8_3_27_platform", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    _VERIFIER = module
    return _VERIFIER


def sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


class DocumentedContractTests(unittest.TestCase):
    def test_current_case_contract_digest_is_documented(self):
        verifier = load_verifier()
        expected = f"`{verifier.EXPECTED_CASE_CONTRACT_SHA256}`"
        documents = (
            ROOT / "spec/0126-platform-8-3-27-deviation-matrix.md",
            ROOT
            / "docs/superpowers/specs/2026-07-23-platform-8-3-27-format-2-20-design.md",
        )

        for path in documents:
            with self.subTest(path=path.relative_to(ROOT).as_posix()):
                self.assertIn(expected, path.read_text(encoding="utf-8"))


CONFIG_XML = '''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="11111111-1111-1111-1111-111111111111"/></MetaDataObject>'''


def write_platform_case(
    corpus_root: Path,
    case_id: str = "cf-init-default",
    *,
    kind: str = "configuration",
    impact_class: str = "CreateOrModify",
    source_name: str = "src",
):
    workspace_rel = f"cases/{case_id}/workspace"
    pre_snapshot_rel = f"cases/{case_id}/pre-xml"
    source_rel = f"{workspace_rel}/{source_name}"
    owner_rel = f"{source_rel}/Configuration.xml"
    owner = write(corpus_root / owner_rel, CONFIG_XML)
    owner_hash = sha256(owner.read_bytes())
    workspace_owner_rel = f"{source_name}/Configuration.xml"
    before = {} if impact_class != "None" else {workspace_owner_rel: owner_hash}
    after = {workspace_owner_rel: owner_hash}
    delta = {
        "created": [workspace_owner_rel] if not before else [],
        "modified": [],
        "removed": [],
        "unchanged": [workspace_owner_rel] if before else [],
    }
    checkpoint = {
        "kind": kind,
        "sourcePath": source_rel,
        "coveredCaseIds": [case_id],
    }
    pre_root = corpus_root / pre_snapshot_rel
    pre_root.mkdir(parents=True)
    pre_non_xml_root = corpus_root / f"cases/{case_id}/pre-non-xml"
    pre_non_xml_root.mkdir(parents=True)
    pre_files = []
    pre_owner_versions = {}
    if before:
        pre_owner_rel = f"{pre_snapshot_rel}/{workspace_owner_rel}"
        pre_owner = write(pre_root / workspace_owner_rel, CONFIG_XML)
        pre_files.append(
            {
                "path": pre_owner_rel,
                "sha256": sha256(pre_owner.read_bytes()),
                "family": "metadata",
            }
        )
        pre_owner_versions[pre_owner_rel] = "2.20"
    pre_non_xml_files = []
    non_xml_files = []
    removed_non_xml_paths = []
    if impact_class == "None":
        non_xml_rel = f"{source_name}/Ext/ManagedApplicationModule.bsl"
        non_xml_corpus_rel = f"{workspace_rel}/{non_xml_rel}"
        before_payload = b"before\r\n"
        after_payload = b"after\r\n"
        non_xml_path = corpus_root / non_xml_corpus_rel
        non_xml_path.parent.mkdir(parents=True, exist_ok=True)
        non_xml_path.write_bytes(after_payload)
        pre_non_xml_path = pre_non_xml_root / non_xml_rel
        pre_non_xml_path.parent.mkdir(parents=True, exist_ok=True)
        pre_non_xml_path.write_bytes(before_payload)
        pre_non_xml_files.append(
            {
                "path": f"cases/{case_id}/pre-non-xml/{non_xml_rel}",
                "sha256": sha256(before_payload),
            }
        )
        non_xml_files.append(
            {
                "path": non_xml_corpus_rel,
                "sha256": sha256(after_payload),
                "seed": True,
                "delta": "modified",
            }
        )
    report_rel = f"cases/{case_id}/case-report.json"
    report = {
        "schemaVersion": 1,
        "profile": "1c-8.3.27-export-2.20",
        "id": case_id,
        "toolId": "unica.cf.init",
        "operation": "cf-init",
        "branch": "default",
        "impactClass": impact_class,
        "workspacePath": workspace_rel,
        "preSnapshotPath": pre_snapshot_rel,
        "platformCheckpoint": checkpoint,
        "publicArguments": {"cwd": "$CASE_WORKSPACE", "dryRun": False},
        "targetCall": {"sequence": 1, "resultOk": True, "errors": [], "summary": "ok"},
        "preFiles": pre_files,
        "preXmlSha256": before,
        "postXmlSha256": after,
        "delta": delta,
        "seedOutputs": sorted(before),
        "remainingXml": sorted(after),
        "removedPaths": [],
        "ownerLinks": {},
        "preOwnerVersions": pre_owner_versions,
        "ownerVersions": {owner_rel: "2.20"},
        "preNonXmlFiles": pre_non_xml_files,
        "nonXmlFiles": non_xml_files,
        "removedNonXmlPaths": removed_non_xml_paths,
        "auxiliaryFiles": [],
    }
    write(corpus_root / report_rel, json.dumps(report, sort_keys=True))
    case = {
        "id": case_id,
        "workspacePath": workspace_rel,
        "preSnapshotPath": pre_snapshot_rel,
        "platformCheckpoint": checkpoint,
        "checkpoint": report_rel,
        "toolId": "unica.cf.init",
        "operation": "cf-init",
        "branch": "default",
        "impactClass": impact_class,
        "xmlImpact": "unchanged" if impact_class == "None" else "created",
        "preFiles": pre_files,
        "files": [
            {
                "path": owner_rel,
                "sha256": owner_hash,
                "family": "metadata",
                "seed": bool(before),
                "delta": "unchanged" if before else "created",
            }
        ],
        "removedPaths": [],
        "preOwnerVersions": pre_owner_versions,
        "ownerVersions": {owner_rel: "2.20"},
        "preNonXmlFiles": pre_non_xml_files,
        "nonXmlFiles": non_xml_files,
        "removedNonXmlPaths": removed_non_xml_paths,
        "auxiliaryFiles": [],
    }
    return case


def add_pre_file(
    corpus_root: Path,
    case: dict,
    logical_path: str,
    text: str,
    family: str,
    *,
    owner_logical_path: str | None = None,
    source_set_owner: bool = False,
) -> tuple[str, str]:
    pre_snapshot_rel = case["preSnapshotPath"]
    corpus_path = f"{pre_snapshot_rel}/{logical_path}"
    path = write(corpus_root / corpus_path, text)
    item = {
        "path": corpus_path,
        "sha256": sha256(path.read_bytes()),
        "family": family,
    }
    if owner_logical_path is not None:
        item["ownerPath"] = f"{pre_snapshot_rel}/{owner_logical_path}"
    case["preFiles"].append(item)
    case["preFiles"].sort(key=lambda entry: entry["path"])
    if source_set_owner:
        case["preOwnerVersions"][corpus_path] = "2.20"
        case["preOwnerVersions"] = dict(sorted(case["preOwnerVersions"].items()))
    return item["sha256"], corpus_path


def read_case_report(corpus_root: Path, case: dict) -> dict:
    return json.loads((corpus_root / case["checkpoint"]).read_text(encoding="utf-8"))


def write_case_report(corpus_root: Path, case: dict, report: dict) -> None:
    (corpus_root / case["checkpoint"]).write_text(
        json.dumps(report, sort_keys=True), encoding="utf-8"
    )


def add_non_xml_file(
    corpus_root: Path,
    case: dict,
    workspace_relative_path: str,
    payload: bytes,
    *,
    pre_payload: bytes | None = None,
) -> None:
    corpus_path = f'{case["workspacePath"]}/{workspace_relative_path}'
    path = corpus_root / corpus_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    if pre_payload is not None:
        pre_path = (
            corpus_root
            / f'cases/{case["id"]}/pre-non-xml/{workspace_relative_path}'
        )
        pre_path.parent.mkdir(parents=True, exist_ok=True)
        pre_path.write_bytes(pre_payload)
        case["preNonXmlFiles"].append(
            {
                "path": f'cases/{case["id"]}/pre-non-xml/{workspace_relative_path}',
                "sha256": sha256(pre_payload),
            }
        )
    post_digest = sha256(payload)
    if pre_payload is None:
        delta = "created"
    elif sha256(pre_payload) == post_digest:
        delta = "unchanged"
    else:
        delta = "modified"
    case["nonXmlFiles"].append(
        {
            "path": corpus_path,
            "sha256": post_digest,
            "seed": pre_payload is not None,
            "delta": delta,
        }
    )
    case["preNonXmlFiles"].sort(key=lambda item: item["path"])
    case["nonXmlFiles"].sort(key=lambda item: item["path"])
    report = read_case_report(corpus_root, case)
    report["preNonXmlFiles"] = copy.deepcopy(case["preNonXmlFiles"])
    report["nonXmlFiles"] = copy.deepcopy(case["nonXmlFiles"])
    report["removedNonXmlPaths"] = copy.deepcopy(case["removedNonXmlPaths"])
    write_case_report(corpus_root, case, report)


def add_auxiliary_file(
    corpus_root: Path,
    case: dict,
    workspace_relative_path: str,
    payload: bytes,
) -> None:
    corpus_path = f'{case["workspacePath"]}/{workspace_relative_path}'
    path = corpus_root / corpus_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    case["auxiliaryFiles"].append(
        {"path": corpus_path, "sha256": sha256(payload)}
    )
    case["auxiliaryFiles"].sort(key=lambda item: item["path"])
    report = read_case_report(corpus_root, case)
    report["auxiliaryFiles"] = copy.deepcopy(case["auxiliaryFiles"])
    write_case_report(corpus_root, case, report)


def write_manifest(corpus_root: Path, cases: list[dict]) -> Path:
    for sequence, case in enumerate(cases, 1):
        report = read_case_report(corpus_root, case)
        report["targetCall"]["sequence"] = sequence
        write_case_report(corpus_root, case, report)
    empty_directory_paths = sorted(
        path.relative_to(corpus_root).as_posix()
        for path in corpus_root.rglob("*")
        if path.is_dir() and not any(path.iterdir())
    )
    manifest = corpus_root / "corpus-manifest.json"
    manifest.write_text(
        json.dumps(
            {
                "schemaVersion": 2,
                "profile": "1c-8.3.27-export-2.20",
                "emptyDirectoryPaths": empty_directory_paths,
                "cases": cases,
            },
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    return manifest


class SemanticXmlTests(unittest.TestCase):
    def test_checked_in_platform_verifier_exists(self):
        self.assertTrue(SCRIPT.is_file())

    def test_root_version_preserves_the_raw_lexical_attribute(self):
        verifier = load_verifier()
        cases = (
            (
                b'<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.&#50;0"><Configuration/></MetaDataObject>',
                "2.&#50;0",
            ),
            (
                b"\xef\xbb\xbf<?xml version='1.0'?><!--before--><MetaDataObject xmlns='http://v8.1c.ru/8.3/MDClasses' note='x > y' version = '2.20'><Configuration/></MetaDataObject>",
                "2.20",
            ),
            (
                b'<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:x="urn:x" x:version="2.20"><Configuration/></MetaDataObject>',
                None,
            ),
        )

        for payload, expected in cases:
            with self.subTest(expected=expected):
                version, owner_type, root_qname = (
                    verifier._parse_root_details_payload(
                        payload,
                        "entity-version.xml",
                    )
                )
                self.assertEqual(version, expected)
                self.assertEqual(owner_type, "Configuration")
                self.assertEqual(
                    root_qname,
                    "{http://v8.1c.ru/8.3/MDClasses}MetaDataObject",
                )

    def test_lexical_only_differences_are_semantically_equal(self):
        verifier = load_verifier()
        left = b'''\xef\xbb\xbf<?xml version="1.0" encoding="UTF-8"?>\r\n<old:Root xmlns:old="http://v8.1c.ru/8.1/data/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:t="urn:type" b="2" a="1">\r\n  <old:Type>t:Thing</old:Type>\r\n  <old:Item xsi:type="t:Thing" />\r\n</old:Root>'''
        right = b'''<r:Root a="1" b="2" xmlns:r="http://v8.1c.ru/8.1/data/core" xmlns:i="http://www.w3.org/2001/XMLSchema-instance" xmlns:q="urn:type"><r:Type>q:Thing</r:Type><r:Item i:type="q:Thing"></r:Item></r:Root>'''

        self.assertEqual(verifier.semantic_xml(left, "left.xml"), verifier.semantic_xml(right, "right.xml"))

    def test_semantic_tree_detects_structural_and_value_changes(self):
        verifier = load_verifier()
        base = b'<r xmlns="urn:r"><a>one</a><b x="1"/></r>'
        variants = {
            "added": b'<r xmlns="urn:r"><a>one</a><b x="1"/><c/></r>',
            "removed": b'<r xmlns="urn:r"><a>one</a></r>',
            "reordered": b'<r xmlns="urn:r"><b x="1"/><a>one</a></r>',
            "value": b'<r xmlns="urn:r"><a>two</a><b x="1"/></r>',
            "attribute": b'<r xmlns="urn:r"><a>one</a><b x="2"/></r>',
        }

        for label, payload in variants.items():
            with self.subTest(label=label):
                self.assertNotEqual(
                    verifier.semantic_xml(base, "base.xml"),
                    verifier.semantic_xml(payload, f"{label}.xml"),
                )

    def test_type_description_repeated_value_groups_are_semantic_multisets(self):
        verifier = load_verifier()
        left = b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg">
          <v8:Type>cfg:CatalogRef.Z</v8:Type>
          <v8:Type>xs:string</v8:Type>
          <v8:TypeSet>cfg:DefinedType.Z</v8:TypeSet>
          <v8:TypeSet>cfg:DefinedType.A</v8:TypeSet>
          <v8:TypeId>bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb</v8:TypeId>
          <v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId>
          <v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>
        </Type>'''
        right = b'''<Type xmlns:c="http://v8.1c.ru/8.1/data/core" xmlns:s="http://www.w3.org/2001/XMLSchema" xmlns:q="urn:cfg">
          <c:Type>s:string</c:Type>
          <c:Type>q:CatalogRef.Z</c:Type>
          <c:TypeSet>q:DefinedType.A</c:TypeSet>
          <c:TypeSet>q:DefinedType.Z</c:TypeSet>
          <c:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</c:TypeId>
          <c:TypeId>bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb</c:TypeId>
          <c:StringQualifiers><c:Length>10</c:Length><c:AllowedLength>Variable</c:AllowedLength></c:StringQualifiers>
        </Type>'''

        self.assertEqual(
            verifier.semantic_xml(left, "left.xml"),
            verifier.semantic_xml(right, "right.xml"),
        )

    def test_type_description_group_order_multiplicity_and_qualifiers_remain_significant(self):
        verifier = load_verifier()
        base = b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg">
          <v8:Type>xs:string</v8:Type>
          <v8:Type>cfg:CatalogRef.Z</v8:Type>
          <v8:TypeSet>cfg:AnyRef</v8:TypeSet>
          <v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId>
          <v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>
        </Type>'''
        variants = {
            "wrong-group-order": b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg"><v8:TypeSet>cfg:AnyRef</v8:TypeSet><v8:Type>xs:string</v8:Type><v8:Type>cfg:CatalogRef.Z</v8:Type><v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>''',
            "interleaved-qualifier": b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg"><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers><v8:Type>cfg:CatalogRef.Z</v8:Type><v8:TypeSet>cfg:AnyRef</v8:TypeSet><v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId></Type>''',
            "duplicate-type": b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg"><v8:Type>xs:string</v8:Type><v8:Type>cfg:CatalogRef.Z</v8:Type><v8:Type>xs:string</v8:Type><v8:TypeSet>cfg:AnyRef</v8:TypeSet><v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>''',
            "qualifier-value": b'''<Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:cfg="urn:cfg"><v8:Type>xs:string</v8:Type><v8:Type>cfg:CatalogRef.Z</v8:Type><v8:TypeSet>cfg:AnyRef</v8:TypeSet><v8:TypeId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</v8:TypeId><v8:StringQualifiers><v8:Length>11</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>''',
        }

        for label, payload in variants.items():
            with self.subTest(label=label):
                self.assertNotEqual(
                    verifier.semantic_xml(base, "base.xml"),
                    verifier.semantic_xml(payload, f"{label}.xml"),
                )

    def test_semantic_digest_alpha_renames_uuids_but_keeps_content(self):
        verifier = load_verifier()
        first = {
            "a.xml": b'<r id="11111111-1111-4111-8111-111111111111"><ref>11111111-1111-4111-8111-111111111111</ref><value>one</value></r>'
        }
        renamed = {
            "a.xml": b'<r id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><ref>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</ref><value>one</value></r>'
        }
        changed = {
            "a.xml": b'<r id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><ref>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</ref><value>two</value></r>'
        }
        stable_platform_id = {
            "a.xml": b'<r><ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</ClassId></r>'
        }
        corrupted_platform_id = {
            "a.xml": b'<r><ClassId>aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa</ClassId></r>'
        }
        fixed_panel_ids = {
            "a.xml": b'<r><panel id="00000000-0000-0000-0000-000000000009"/><panelDef id="13322b22-3960-4d68-93a6-fe2dd7f28ca3"/></r>'
        }
        corrupted_panel_ids = {
            "a.xml": b'<r><panel id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"/><panelDef id="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"/></r>'
        }

        self.assertEqual(
            verifier.uuid_normalized_semantic_sha256(first, "first"),
            verifier.uuid_normalized_semantic_sha256(renamed, "renamed"),
        )
        self.assertNotEqual(
            verifier.uuid_normalized_semantic_sha256(first, "first"),
            verifier.uuid_normalized_semantic_sha256(changed, "changed"),
        )
        self.assertNotEqual(
            verifier.uuid_normalized_semantic_sha256(
                stable_platform_id, "stable-platform-id"
            ),
            verifier.uuid_normalized_semantic_sha256(
                corrupted_platform_id, "corrupted-platform-id"
            ),
        )
        self.assertNotEqual(
            verifier.uuid_normalized_semantic_sha256(
                fixed_panel_ids, "fixed-panel-ids"
            ),
            verifier.uuid_normalized_semantic_sha256(
                corrupted_panel_ids, "corrupted-panel-ids"
            ),
        )

    def test_transition_digest_uses_one_uuid_map_across_pre_and_post(self):
        verifier = load_verifier()

        def document(uuid: str) -> bytes:
            return (
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" '
                f'version="2.20"><Configuration uuid="{uuid}"/>'
                "</MetaDataObject>"
            ).encode()

        first = "11111111-1111-4111-8111-111111111111"
        second = "22222222-2222-4222-8222-222222222222"
        third = "33333333-3333-4333-8333-333333333333"
        stable_first = verifier.transition_semantic_sha256(
            {"a.xml": document(first)}, {"a.xml": document(first)}, "stable-first"
        )
        stable_renamed = verifier.transition_semantic_sha256(
            {"a.xml": document(second)},
            {"a.xml": document(second)},
            "stable-renamed",
        )
        identity_churn = verifier.transition_semantic_sha256(
            {"a.xml": document(second)},
            {"a.xml": document(third)},
            "identity-churn",
        )

        self.assertEqual(stable_first, stable_renamed)
        self.assertNotEqual(stable_renamed, identity_churn)

    def test_qname_values_compare_by_namespace_and_reject_unresolved_prefixes(self):
        verifier = load_verifier()
        one = b'<r xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:t="urn:one"><v8:TypeSet>t:Thing</v8:TypeSet></r>'
        same = b'<r xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:q="urn:one"><v8:TypeSet>q:Thing</v8:TypeSet></r>'
        other = b'<r xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:t="urn:two"><v8:TypeSet>t:Thing</v8:TypeSet></r>'

        self.assertEqual(verifier.semantic_xml(one, "one.xml"), verifier.semantic_xml(same, "same.xml"))
        self.assertNotEqual(verifier.semantic_xml(one, "one.xml"), verifier.semantic_xml(other, "other.xml"))
        with self.assertRaisesRegex(verifier.SourceError, "unresolved QName prefix"):
            verifier.semantic_xml(
                b'<r xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:Type>missing:Thing</v8:Type></r>',
                "broken.xml",
            )

    def test_qname_values_accept_unicode_xml_ncnames(self):
        verifier = load_verifier()
        one = """<r xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:d5p1="urn:cfg"><v8:TypeSet>d5p1:ВидыСубконтоХозрасчетные</v8:TypeSet></r>""".encode()
        same = """<r xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="urn:cfg"><v8:TypeSet>cfg:ВидыСубконтоХозрасчетные</v8:TypeSet></r>""".encode()

        self.assertEqual(
            verifier.semantic_xml(one, "one.xml"),
            verifier.semantic_xml(same, "same.xml"),
        )
        for lexical in ["1bad", "bad name", "a:b:c"]:
            payload = f'''<r xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:Type>{lexical}</v8:Type></r>'''.encode()
            with self.assertRaisesRegex(verifier.SourceError, "invalid lexical QName"):
                verifier.semantic_xml(payload, "broken.xml")

    def test_non_qname_type_and_value_type_elements_remain_plain_text(self):
        verifier = load_verifier()
        payload = b'''<r xmlns:l="http://v8.1c.ru/8.3/xcf/logform"><l:Type>missing:CommandBarButton</l:Type><ValueType>missing:Thing</ValueType></r>'''

        semantic = verifier.semantic_xml(payload, "plain.xml")

        self.assertIsNotNone(semantic)

    def test_mdclasses_xdto_type_elements_compare_as_qnames(self):
        verifier = load_verifier()
        one = b'''<m:X xmlns:m="http://v8.1c.ru/8.3/MDClasses" xmlns:a="urn:types"><m:XDTOReturningValueType>a:Result</m:XDTOReturningValueType><m:XDTOValueType>a:Value</m:XDTOValueType></m:X>'''
        same = b'''<m:X xmlns:m="http://v8.1c.ru/8.3/MDClasses" xmlns:b="urn:types"><m:XDTOReturningValueType>b:Result</m:XDTOReturningValueType><m:XDTOValueType>b:Value</m:XDTOValueType></m:X>'''

        self.assertEqual(
            verifier.semantic_xml(one, "one.xml"),
            verifier.semantic_xml(same, "same.xml"),
        )

    def test_non_indentation_text_and_element_order_are_preserved(self):
        verifier = load_verifier()
        compact = b'<r><a> x </a><b/>tail</r>'
        indented = b'<r>\n  <a> x </a>\n  <b/>tail</r>'
        changed = b'<r><a>x</a><b/>tail</r>'

        self.assertEqual(verifier.semantic_xml(compact, "compact.xml"), verifier.semantic_xml(indented, "indented.xml"))
        self.assertNotEqual(verifier.semantic_xml(compact, "compact.xml"), verifier.semantic_xml(changed, "changed.xml"))
        self.assertNotEqual(
            verifier.semantic_xml(b"<r><a> </a></r>", "space.xml"),
            verifier.semantic_xml(b"<r><a></a></r>", "empty.xml"),
        )

    def test_doctype_and_entity_nodes_are_rejected(self):
        verifier = load_verifier()
        payload = b'<!DOCTYPE r [<!ENTITY x "value">]><r>&x;</r>'
        with self.assertRaisesRegex(verifier.SourceError, "DOCTYPE|entity"):
            verifier.semantic_xml(payload, "entity.xml")


class SemanticDirectoryTests(unittest.TestCase):
    def test_snapshot_comparison_detects_added_and_removed_empty_directories(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "Configuration.xml", CONFIG_XML)
            write(right / "Configuration.xml", CONFIG_XML)
            (left / "removed-empty").mkdir()
            (right / "added-empty").mkdir()

            comparison = verifier.compare_xml_snapshots(
                verifier.capture_directory_xml_snapshot(left),
                verifier.capture_directory_xml_snapshot(right),
            )

            self.assertFalse(comparison["equal"])
            self.assertEqual(
                comparison["directories"],
                {
                    "equal": False,
                    "added": ["added-empty"],
                    "removed": ["removed-empty"],
                },
            )

    def test_config_dump_info_is_the_only_excluded_xml_file(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "Configuration.xml", '<r version="2.20"><a/></r>')
            write(right / "Configuration.xml", '<r version="2.20">\n<a></a>\n</r>')
            write(left / "ConfigDumpInfo.xml", "<left/>")
            write(right / "ConfigDumpInfo.xml", "<right/>")

            comparison = verifier.compare_xml_directories(left, right)

            self.assertTrue(comparison["equal"])
            self.assertEqual(comparison["excluded"], ["ConfigDumpInfo.xml"])

    def test_every_other_file_set_drift_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "a.xml", "<a/>")
            write(left / "nested" / "b.xml", "<b/>")
            write(right / "a.xml", "<a/>")
            write(right / "nested" / "c.xml", "<c/>")

            comparison = verifier.compare_xml_directories(left, right)

            self.assertFalse(comparison["equal"])
            self.assertEqual(comparison["removed"], ["nested/b.xml"])
            self.assertEqual(comparison["added"], ["nested/c.xml"])
            self.assertEqual(comparison["changed"], [])

    def test_nested_config_dump_info_is_not_an_exclusion(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "Configuration.xml", "<r/>")
            write(right / "Configuration.xml", "<r/>")
            write(left / "nested" / "ConfigDumpInfo.xml", "<before/>")
            write(right / "nested" / "ConfigDumpInfo.xml", "<after/>")

            comparison = verifier.compare_xml_directories(left, right)

            self.assertFalse(comparison["equal"])
            self.assertEqual(comparison["changed"], ["nested/ConfigDumpInfo.xml"])
            self.assertEqual(comparison["excluded"], [])

    def test_changed_xml_is_reported_separately_and_deterministically(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "z.xml", "<r><a/></r>")
            write(right / "z.xml", "<r><b/></r>")
            write(left / "a.xml", "<r>one</r>")
            write(right / "a.xml", "<r>two</r>")

            comparison = verifier.compare_xml_directories(left, right)

            self.assertEqual(comparison["changed"], ["a.xml", "z.xml"])
            self.assertFalse(comparison["equal"])

    def test_directory_reader_rejects_symlinks_and_missing_xml(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            empty = root / "empty"
            empty.mkdir()
            with self.assertRaisesRegex(verifier.SourceError, "no XML"):
                verifier.semantic_xml_set(empty)

            source = write(root / "source.xml", "<r/>")
            linked = root / "linked"
            linked.mkdir()
            (linked / "link.xml").symlink_to(source)
            with self.assertRaisesRegex(verifier.SourceError, "symlink"):
                verifier.semantic_xml_set(linked)

    def test_xml_snapshots_reject_internal_and_external_hardlink_aliases(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as outside_tmp:
            root = Path(tmp)
            original = write(root / "one.xml", "<r/>")
            os.link(original, root / "two.xml")
            with self.assertRaisesRegex(verifier.SourceError, "hardlink|link count"):
                verifier.capture_directory_xml_snapshot(root)

            (root / "two.xml").unlink()
            os.link(original, Path(outside_tmp) / "outside.xml")
            with self.assertRaisesRegex(verifier.SourceError, "hardlink|link count"):
                verifier.capture_directory_xml_snapshot(root)

    def test_xml_snapshot_rejects_special_entries_instead_of_ignoring_them(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "Configuration.xml", CONFIG_XML)
            os.mkfifo(root / "Ghost.xml")

            with self.assertRaisesRegex(verifier.SourceError, "special.*entry|FIFO"):
                verifier.capture_directory_xml_snapshot(root)

    def test_artifact_pair_maps_the_descriptor_and_content_as_one_xml_tree(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_descriptor = write(root / "source/CorpusProcessor.xml", CONFIG_XML)
            source_content = root / "source/CorpusProcessor"
            write(source_content / "Forms/CorpusForm.xml", "<Form><Value>one</Value></Form>")
            export_descriptor = write(root / "round1/export.xml", CONFIG_XML)
            export_content = root / "round1/export"
            write(export_content / "Forms/CorpusForm.xml", "<Form><Value>one</Value></Form>")

            comparison = verifier.compare_artifact_xml_pairs(
                (source_descriptor, source_content),
                (export_descriptor, export_content),
            )

            self.assertTrue(comparison["equal"])
            self.assertEqual(comparison["added"], [])
            self.assertEqual(comparison["removed"], [])
            self.assertEqual(
                sorted(verifier.artifact_raw_xml_hashes(export_descriptor, export_content)),
                ["content/Forms/CorpusForm.xml", "descriptor.xml"],
            )

            write(export_descriptor, CONFIG_XML.replace("2.20", "2.19"))
            changed = verifier.compare_artifact_xml_pairs(
                (source_descriptor, source_content),
                (export_descriptor, export_content),
            )
            self.assertEqual(changed["changed"], ["descriptor.xml"])

    def test_snapshot_comparison_tracks_non_xml_bytes_exactly(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            left = root / "left"
            right = root / "right"
            write(left / "Configuration.xml", CONFIG_XML)
            write(right / "Configuration.xml", CONFIG_XML)
            (left / "changed.bsl").write_bytes(b"\xef\xbb\xbfbefore\r\n")
            (right / "changed.bsl").write_bytes(b"\xef\xbb\xbfafter\r\n")
            (left / "removed.bin").write_bytes(b"removed")
            (right / "added.html").write_bytes(b"<html></html>\r\n")

            comparison = verifier.compare_xml_snapshots(
                verifier.capture_directory_xml_snapshot(left),
                verifier.capture_directory_xml_snapshot(right),
            )

            self.assertFalse(comparison["equal"])
            self.assertEqual(comparison["changed"], ["changed.bsl"])
            self.assertEqual(comparison["removed"], ["removed.bin"])
            self.assertEqual(comparison["added"], ["added.html"])
            self.assertTrue(comparison["xml"]["equal"])
            self.assertFalse(comparison["nonXml"]["equal"])

    def test_artifact_snapshot_maps_non_xml_content_to_logical_content_paths(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            descriptor = write(root / "Artifact.xml", CONFIG_XML)
            content = root / "Artifact"
            module = content / "Forms/Main/Ext/Form/Module.bsl"
            module.parent.mkdir(parents=True)
            module.write_bytes(b"\xef\xbb\xbfProcedure Test()\r\nEndProcedure\r\n")

            snapshot = verifier.capture_artifact_xml_snapshot(descriptor, content)

            self.assertEqual(
                set(snapshot["rawNonXmlHashes"]),
                {"content/Forms/Main/Ext/Form/Module.bsl"},
            )

    def test_snapshot_rejects_non_xml_hardlinks_and_special_entries(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "Configuration.xml", CONFIG_XML)
            module = root / "Module.bsl"
            module.write_bytes(b"module")
            os.link(module, root / "Alias.bsl")
            with self.assertRaisesRegex(verifier.SourceError, "hardlink|link count"):
                verifier.capture_directory_xml_snapshot(root)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "Configuration.xml", CONFIG_XML)
            os.mkfifo(root / "Payload.bin")
            with self.assertRaisesRegex(verifier.SourceError, "special.*entry"):
                verifier.capture_directory_xml_snapshot(root)

    def test_artifact_pair_allows_descriptor_only_xml_with_non_xml_content(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_descriptor = write(root / "source/Processor.xml", CONFIG_XML)
            source_content = root / "source/Processor"
            write(source_content / "Ext/ObjectModule.bsl", "Procedure Run()\nEndProcedure")
            export_descriptor = write(root / "round1/export.xml", CONFIG_XML)
            export_content = root / "round1/export"
            write(export_content / "Ext/ObjectModule.bsl", "Procedure Run()\nEndProcedure")

            comparison = verifier.compare_artifact_xml_pairs(
                (source_descriptor, source_content),
                (export_descriptor, export_content),
            )

            self.assertTrue(comparison["equal"])
            self.assertEqual(
                verifier.artifact_raw_xml_hashes(export_descriptor, export_content),
                {"descriptor.xml": sha256(CONFIG_XML.encode("utf-8"))},
            )

            write(export_descriptor, CONFIG_XML.replace("2.20", "2.19"))
            changed = verifier.compare_artifact_xml_pairs(
                (source_descriptor, source_content),
                (export_descriptor, export_content),
            )
            self.assertEqual(changed["changed"], ["descriptor.xml"])

    def test_artifact_checkpoint_rejects_unpaired_source_siblings(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Artifact.xml", CONFIG_XML)
            write(source / "Artifact/Ext/ObjectModule.bsl", "Procedure Run()\nEndProcedure")
            write(source / "Ghost.xml", CONFIG_XML)
            item = CheckpointExecutionTests().checkpoint_item(source, kind="epf")

            with self.assertRaisesRegex(verifier.SourceError, "unpaired|sibling"):
                verifier._artifact_source_pair(item)

    def test_artifact_content_never_applies_configuration_dump_exclusion(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_descriptor = write(root / "source/Artifact.xml", CONFIG_XML)
            source_content = root / "source/Artifact"
            write(source_content / "ConfigDumpInfo.xml", "<before/>")
            export_descriptor = write(root / "round1/export.xml", CONFIG_XML)
            export_content = root / "round1/export"
            write(export_content / "ConfigDumpInfo.xml", "<after/>")

            comparison = verifier.compare_artifact_xml_pairs(
                (source_descriptor, source_content),
                (export_descriptor, export_content),
            )

            self.assertFalse(comparison["equal"])
            self.assertEqual(comparison["changed"], ["content/ConfigDumpInfo.xml"])
            self.assertEqual(comparison["excluded"], [])


class CorpusAdapterTests(unittest.TestCase):
    def test_mandatory_corpus_includes_order_sensitive_dcs_edits(self):
        verifier = load_verifier()

        self.assertTrue(
            {
                "dcs-edit-owned-template",
                "dcs-edit-add-parameter-after-settings",
                "dcs-edit-set-structure-after-settings",
                "dcs-edit-modify-field-role-restriction",
            }.issubset(verifier.MANDATORY_CASE_IDS)
        )

    def test_mandatory_corpus_includes_every_cfe_patch_module_layout(self):
        verifier = load_verifier()

        self.assertEqual(len(verifier.MANDATORY_CASE_IDS), 63)
        self.assertTrue(
            {
                "cfe-patch-method-bsl-only",
                "cfe-patch-method-catalog-object-module",
                "cfe-patch-method-catalog-manager-module",
                "cfe-patch-method-information-register-record-set-module",
                "cfe-patch-method-catalog-form-module",
                "cfe-patch-method-constant-value-manager-module",
            }.issubset(verifier.MANDATORY_CASE_IDS)
        )

    def load_single(self, root: Path, case: dict | None = None, **kwargs):
        verifier = load_verifier()
        case = case or write_platform_case(root)
        manifest = write_manifest(root, [case])
        return verifier.load_corpus(
            manifest,
            repo_root=ROOT,
            home_root=Path.home(),
            mandatory_case_ids={case["id"]},
            **kwargs,
        )

    def test_manifest_empty_directory_inventory_rejects_added_and_removed_paths(self):
        verifier = load_verifier()
        for mutation in ("added", "removed"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                declared_empty = root / "cases/cf-init-default/workspace/declared-empty"
                declared_empty.mkdir()
                manifest = write_manifest(root, [case])
                if mutation == "added":
                    (root / "cases/cf-init-default/workspace/late-empty").mkdir()
                else:
                    declared_empty.rmdir()

                with self.assertRaisesRegex(
                    verifier.CorpusError, "empty directory inventory"
                ):
                    verifier.load_corpus(
                        manifest,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"cf-init-default"},
                    )

    def test_explicit_per_case_checkpoint_is_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus = self.load_single(root)

            self.assertEqual(corpus["profile"], "1c-8.3.27-export-2.20")
            self.assertEqual(corpus["selected"][0]["id"], "cf-init-default")
            self.assertEqual(corpus["selected"][0]["checkpoint"]["kind"], "configuration")

    def test_actual_pre_snapshot_version_cannot_be_masked_by_post_claims(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, impact_class="None")
            pre_entry = case["preFiles"][0]
            pre_path = root / pre_entry["path"]
            pre_path.write_text(
                CONFIG_XML.replace('version="2.20"', 'version="2.19"'),
                encoding="utf-8",
            )
            actual_hash = sha256(pre_path.read_bytes())
            pre_entry["sha256"] = actual_hash
            report = read_case_report(root, case)
            report["preFiles"] = case["preFiles"]
            logical_path = pre_entry["path"].removeprefix(
                f'{case["preSnapshotPath"]}/'
            )
            report["preXmlSha256"][logical_path] = actual_hash
            write_case_report(root, case, report)

            with self.assertRaisesRegex(
                verifier.CorpusError, "pre-snapshot.*2.20|preOwnerVersions"
            ):
                self.load_single(root, case)

    def test_pre_snapshot_inventory_family_and_bytes_are_exact(self):
        verifier = load_verifier()
        mutations = {
            "removed": lambda root, case, report: case["preFiles"].clear(),
            "wrong-family": lambda root, case, report: case["preFiles"][0].update(
                {"family": "dcs"}
            ),
            "tampered": lambda root, case, report: (root / case["preFiles"][0]["path"]).write_text(
                CONFIG_XML.replace("11111111", "aaaaaaaa"), encoding="utf-8"
            ),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root, impact_class="None")
                report = read_case_report(root, case)
                mutate(root, case, report)
                report["preFiles"] = case["preFiles"]
                write_case_report(root, case, report)

                with self.assertRaisesRegex(
                    verifier.CorpusError, "preFiles|pre-snapshot|hash|inventory|family"
                ):
                    self.load_single(root, case)

    def test_pre_snapshot_shape_is_strict(self):
        verifier = load_verifier()
        mutations = (
            lambda case, report: case.update({"preFiles": True}),
            lambda case, report: case["preFiles"][0].update(
                {"newStandalone": False}
            ),
            lambda case, report: case["preFiles"][0].update({"sha256": True}),
            lambda case, report: case.update({"preOwnerVersions": True}),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root, impact_class="None")
                report = read_case_report(root, case)
                mutate(case, report)
                report["preFiles"] = case.get("preFiles")
                report["preOwnerVersions"] = case.get("preOwnerVersions")
                write_case_report(root, case, report)

                with self.assertRaisesRegex(
                    verifier.CorpusError, "preFiles|preOwnerVersions|shape|hash"
                ):
                    self.load_single(root, case)

    def test_pre_snapshot_rejects_hardlinks_special_entries_and_report_drift(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as outside_tmp:
            root = Path(tmp)
            case = write_platform_case(root, impact_class="None")
            pre_path = root / case["preFiles"][0]["path"]
            os.link(pre_path, Path(outside_tmp) / "external.xml")
            with self.assertRaisesRegex(
                verifier.SourceError, "hardlink|link count"
            ):
                self.load_single(root, case)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, impact_class="None")
            os.mkfifo(root / case["preSnapshotPath"] / "Ghost.xml")
            with self.assertRaisesRegex(
                verifier.SourceError, "special.*entry|FIFO"
            ):
                self.load_single(root, case)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, impact_class="None")
            report = read_case_report(root, case)
            report["preFiles"] = []
            write_case_report(root, case, report)
            with self.assertRaisesRegex(
                verifier.CorpusError, "manifest/report mismatch for preFiles"
            ):
                self.load_single(root, case)

    def test_schema_versions_require_json_integer_one(self):
        verifier = load_verifier()
        for invalid in (True, 1.0):
            with self.subTest(document="manifest", invalid=invalid), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                manifest = write_manifest(root, [case])
                payload = json.loads(manifest.read_text(encoding="utf-8"))
                payload["schemaVersion"] = invalid
                manifest.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")

                with self.assertRaisesRegex(verifier.CorpusError, "schemaVersion"):
                    verifier.load_corpus(
                        manifest,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={case["id"]},
                    )

            with self.subTest(document="case-report", invalid=invalid), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                report = read_case_report(root, case)
                report["schemaVersion"] = invalid
                write_case_report(root, case, report)

                with self.assertRaisesRegex(verifier.CorpusError, "profile/schema"):
                    self.load_single(root, case)

    def test_manifest_case_report_and_target_schemas_reject_unknown_fields(self):
        verifier = load_verifier()
        mutations = ("manifest", "case", "report", "target")
        for document in mutations:
            with self.subTest(document=document), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                manifest = write_manifest(root, [case])
                if document == "manifest":
                    payload = json.loads(manifest.read_text(encoding="utf-8"))
                    payload["gateVerdict"] = "pass-from-manifest"
                    manifest.write_text(
                        json.dumps(payload, sort_keys=True), encoding="utf-8"
                    )
                elif document == "case":
                    payload = json.loads(manifest.read_text(encoding="utf-8"))
                    payload["cases"][0]["gateVerdict"] = "pass-from-case"
                    manifest.write_text(
                        json.dumps(payload, sort_keys=True), encoding="utf-8"
                    )
                else:
                    report = read_case_report(root, case)
                    if document == "report":
                        report["gateVerdict"] = "pass-from-report"
                    else:
                        report["targetCall"]["gateVerdict"] = "pass-from-target"
                    write_case_report(root, case, report)

                with self.assertRaisesRegex(
                    verifier.CorpusError, "fields are not exact|unknown"
                ):
                    verifier.load_corpus(
                        manifest,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={case["id"]},
                    )

    def test_production_case_inventory_is_exact_not_a_mandatory_subset(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            manifest = write_manifest(root, [case])

            with self.assertRaisesRegex(verifier.CorpusError, "exact.*inventory|missing"):
                verifier.load_corpus(
                    manifest,
                    repo_root=ROOT,
                    home_root=Path.home(),
                )

    def test_unreferenced_corpus_xml_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            write(root / "orphan/Unlisted.xml", "<unlisted/>")

            with self.assertRaisesRegex(verifier.CorpusError, "inventory.*exact|unreferenced"):
                self.load_single(root, case)

    def test_undeclared_or_tampered_platform_non_xml_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            path = root / case["platformCheckpoint"]["sourcePath"] / "Ext/Module.bsl"
            path.parent.mkdir(parents=True)
            path.write_bytes(b"undeclared")
            with self.assertRaisesRegex(
                verifier.CorpusError, "nonXmlFiles inventory/hash"
            ):
                self.load_single(root, case)

    def test_undeclared_workspace_side_output_and_pre_snapshot_non_xml_are_rejected(self):
        verifier = load_verifier()
        for location in ("workspace", "pre-xml"):
            with self.subTest(location=location), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                if location == "workspace":
                    rogue = root / case["workspacePath"] / "outside/Untracked.bin"
                else:
                    rogue = root / case["preSnapshotPath"] / "outside/Untracked.bin"
                rogue.parent.mkdir(parents=True, exist_ok=True)
                rogue.write_bytes(b"not declared")

                with self.assertRaisesRegex(
                    verifier.CorpusError,
                    "inventory.*exact|unreferenced|undeclared",
                ):
                    self.load_single(root, case)

    def test_declared_auxiliary_file_is_bound_by_path_and_hash(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            add_auxiliary_file(root, case, "inputs/request.json", b'{"name":"x"}')
            self.load_single(root, case)

            path = root / case["workspacePath"] / "inputs/request.json"
            path.write_bytes(b'{"name":"tampered"}')
            with self.assertRaisesRegex(
                verifier.CorpusError,
                "auxiliaryFiles|inventory/hash|mismatched",
            ):
                self.load_single(root, case)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            add_non_xml_file(
                root,
                case,
                "src/Ext/Module.bsl",
                b"after",
                pre_payload=b"before",
            )
            pre_path = root / case["preNonXmlFiles"][0]["path"]
            pre_path.write_bytes(b"forged-preimage")
            with self.assertRaisesRegex(
                verifier.CorpusError, "preNonXmlFiles inventory/hash"
            ):
                self.load_single(root, case)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            add_non_xml_file(
                root,
                case,
                "src/Ext/Module.bsl",
                b"\xef\xbb\xbfAfter\r\n",
                pre_payload=b"\xef\xbb\xbfBefore\r\n",
            )
            path = root / case["nonXmlFiles"][0]["path"]
            path.write_bytes(b"tampered")
            with self.assertRaisesRegex(
                verifier.CorpusError, "nonXmlFiles inventory/hash"
            ):
                self.load_single(root, case)

    def test_non_xml_contract_shape_hash_path_and_report_are_strict(self):
        verifier = load_verifier()
        mutations = {
            "bad-hash": lambda case, report: case["nonXmlFiles"][0].update(
                {"sha256": "0" * 64}
            ),
            "xml-suffix": lambda case, report: case["nonXmlFiles"][0].update(
                {"path": case["nonXmlFiles"][0]["path"] + ".xml"}
            ),
            "outside-source": lambda case, report: case["nonXmlFiles"][0].update(
                {"path": f'{case["workspacePath"]}/inputs/module.bsl'}
            ),
            "unknown-field": lambda case, report: case["nonXmlFiles"][0].update(
                {"extra": True}
            ),
            "report-drift": lambda case, report: report.update(
                {"nonXmlFiles": []}
            ),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                add_non_xml_file(
                    root,
                    case,
                    "src/Ext/Module.bsl",
                    b"same\r\n",
                    pre_payload=b"same\r\n",
                )
                report = read_case_report(root, case)
                mutate(case, report)
                if label != "report-drift":
                    report["nonXmlFiles"] = copy.deepcopy(case["nonXmlFiles"])
                write_case_report(root, case, report)
                with self.assertRaisesRegex(
                    verifier.CorpusError,
                    "nonXmlFiles|non-XML|checkpoint input boundary|manifest/report",
                ):
                    self.load_single(root, case)

    def test_non_xml_source_rejects_symlink_hardlink_and_special_entries(self):
        verifier = load_verifier()
        for entry_kind in ("symlink", "hardlink", "special"):
            with self.subTest(entry_kind=entry_kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                source = root / case["platformCheckpoint"]["sourcePath"]
                payload = source / "Ext/Payload.bin"
                payload.parent.mkdir(parents=True)
                if entry_kind == "symlink":
                    outside = root / "outside.bin"
                    outside.write_bytes(b"outside")
                    payload.symlink_to(outside)
                elif entry_kind == "hardlink":
                    outside = root / "outside.bin"
                    outside.write_bytes(b"outside")
                    os.link(outside, payload)
                else:
                    os.mkfifo(payload)
                with self.assertRaisesRegex(
                    verifier.SourceError, "symlink|hardlink|link count|special"
                ):
                    self.load_single(root, case)

    def test_case_contract_digest_binds_every_non_xml_field(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            add_non_xml_file(
                root,
                case,
                "src/Ext/Module.bsl",
                b"post",
                pre_payload=b"pre",
            )
            normalized = [{"id": case["id"], "report": read_case_report(root, case)}]
            baseline = verifier.case_contract_sha256([case], normalized)
            for field, mutate in (
                (
                    "preNonXmlFiles",
                    lambda value: value[0].update({"sha256": "1" * 64}),
                ),
                (
                    "nonXmlFiles",
                    lambda value: value[0].update({"delta": "unchanged"}),
                ),
                (
                    "removedNonXmlPaths",
                    lambda value: value.append(
                        f'{case["workspacePath"]}/src/Ext/Removed.bin'
                    ),
                ),
            ):
                changed_case = copy.deepcopy(case)
                mutate(changed_case[field])
                self.assertNotEqual(
                    baseline,
                    verifier.case_contract_sha256([changed_case], normalized),
                    field,
                )

    def test_case_contract_digest_binds_empty_directory_paths(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            normalized = [{"id": case["id"], "report": read_case_report(root, case)}]

            baseline = verifier.case_contract_sha256(
                [case],
                normalized,
                ["cases/cf-init-default/workspace/src/empty"],
            )
            changed = verifier.case_contract_sha256(
                [case],
                normalized,
                ["cases/cf-init-default/workspace/src/other-empty"],
            )

            self.assertNotEqual(baseline, changed)

    def test_checkpoint_shape_is_strict_and_source_path_is_never_inferred(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            missing = write_platform_case(root)
            del missing["platformCheckpoint"]["sourcePath"]
            with self.assertRaisesRegex(load_verifier().CorpusError, "sourcePath"):
                self.load_single(root, missing)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            unknown = write_platform_case(root)
            unknown["platformCheckpoint"]["guessedSource"] = True
            with self.assertRaisesRegex(load_verifier().CorpusError, "unknown checkpoint"):
                self.load_single(root, unknown)

    def test_extension_requires_explicit_base_source_and_none_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            extension = write_platform_case(root, kind="extension")
            with self.assertRaisesRegex(verifier.CorpusError, "baseSourcePath"):
                self.load_single(root, extension)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            none_case = write_platform_case(root, impact_class="None")
            none_case["platformCheckpoint"]["kind"] = "none"
            with self.assertRaisesRegex(verifier.CorpusError, "invalid checkpoint kind"):
                self.load_single(root, none_case)

    def test_unsafe_or_ambiguous_paths_are_rejected(self):
        verifier = load_verifier()
        for field, value in (
            ("workspacePath", "../escape"),
            ("sourcePath", "/absolute/source"),
            ("sourcePath", "cases\\escape"),
            ("workspacePath", "cases/evil\x00path"),
            ("sourcePath", "cases/evil\npath"),
        ):
            with self.subTest(field=field, value=value), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                if field == "workspacePath":
                    case[field] = value
                else:
                    case["platformCheckpoint"][field] = value
                with self.assertRaisesRegex(verifier.CorpusError, "path|Path"):
                    self.load_single(root, case)

    def test_canonical_paths_reject_c0_controls_before_filesystem_access(self):
        verifier = load_verifier()
        for control in ("\x00", "\n", "\x1f"):
            with self.subTest(control=ord(control)), self.assertRaisesRegex(
                verifier.CorpusError, "control"
            ):
                verifier._canonical_relative_path(
                    f"cases/evil{control}path", "synthetic path"
                )

    def test_case_id_must_be_one_safe_kebab_case_filename_component(self):
        verifier = load_verifier()
        for unsafe_id in ("../escape", "a/b", ".", "/absolute", "a_b"):
            with self.subTest(case_id=unsafe_id), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                manifest = write_manifest(root, [case])
                payload = json.loads(manifest.read_text(encoding="utf-8"))
                payload["cases"][0]["id"] = unsafe_id
                manifest.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")

                with self.assertRaisesRegex(verifier.CorpusError, "case id"):
                    verifier.load_corpus(
                        manifest,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids=set(),
                    )

    def test_checkpoint_evidence_path_stays_a_direct_child(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            evidence = root / "evidence"
            evidence.mkdir()
            self.assertEqual(
                verifier.checkpoint_evidence_path(evidence, "valid-case"),
                evidence / "valid-case",
            )
            for unsafe_id in ("../escape", "a/b", ".", "/absolute"):
                with self.subTest(case_id=unsafe_id), self.assertRaisesRegex(
                    verifier.SourceError, "checkpoint id|direct child"
                ):
                    verifier.checkpoint_evidence_path(evidence, unsafe_id)

            external = root / "external"
            external.mkdir()
            (evidence / "linked-case").symlink_to(external)
            with self.assertRaisesRegex(verifier.SourceError, "direct child"):
                verifier.checkpoint_evidence_path(evidence, "linked-case")

    def test_post_hash_delta_remaining_and_disk_must_agree(self):
        verifier = load_verifier()
        mutations = [
            lambda report: report["postXmlSha256"].update(
                {next(iter(report["postXmlSha256"])): "0" * 64}
            ),
            lambda report: report["delta"]["created"].clear(),
            lambda report: report["remainingXml"].clear(),
            lambda report: report["seedOutputs"].append("ghost.xml"),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                report = read_case_report(root, case)
                mutate(report)
                write_case_report(root, case, report)
                with self.assertRaisesRegex(verifier.CorpusError, "hash|delta|remaining|seed"):
                    self.load_single(root, case)

    def test_every_post_xml_is_inside_the_platform_checkpoint_inputs(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            workspace = case["workspacePath"]
            owner_rel = next(iter(case["ownerVersions"]))
            outside_rel = f"{workspace}/outside/Unvalidated.xml"
            outside_workspace_rel = "outside/Unvalidated.xml"
            outside = write(
                root / outside_rel,
                '<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>',
            )
            outside_hash = sha256(outside.read_bytes())

            report = read_case_report(root, case)
            report["postXmlSha256"][outside_workspace_rel] = outside_hash
            report["delta"]["created"].append(outside_workspace_rel)
            report["delta"]["created"].sort()
            report["remainingXml"].append(outside_workspace_rel)
            report["remainingXml"].sort()
            report["ownerLinks"][outside_rel] = owner_rel
            write_case_report(root, case, report)

            case["files"].append(
                {
                    "path": outside_rel,
                    "sha256": outside_hash,
                    "family": "dcs",
                    "seed": False,
                    "delta": "created",
                    "ownerPath": owner_rel,
                }
            )
            case["files"].sort(key=lambda item: item["path"])

            with self.assertRaisesRegex(
                verifier.CorpusError, "outside.*platform checkpoint input|sourcePath"
            ):
                self.load_single(root, case)

    def test_removed_xml_is_inside_the_platform_checkpoint_inputs(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, impact_class="RemoveOrModify")
            workspace = case["workspacePath"]
            owner_workspace_rel = "src/Configuration.xml"
            removed_workspace_rel = "outside/Removed.xml"
            removed_corpus_rel = f"{workspace}/{removed_workspace_rel}"
            pre_owner_hash, _ = add_pre_file(
                root,
                case,
                owner_workspace_rel,
                CONFIG_XML.replace("11111111", "22222222"),
                "metadata",
                source_set_owner=True,
            )
            removed_hash, _ = add_pre_file(
                root,
                case,
                removed_workspace_rel,
                CONFIG_XML.replace("11111111", "33333333"),
                "metadata",
                source_set_owner=True,
            )

            report = read_case_report(root, case)
            owner_hash = report["postXmlSha256"][owner_workspace_rel]
            report["preXmlSha256"] = {
                owner_workspace_rel: pre_owner_hash,
                removed_workspace_rel: removed_hash,
            }
            report["preFiles"] = case["preFiles"]
            report["preOwnerVersions"] = case["preOwnerVersions"]
            report["delta"] = {
                "created": [],
                "modified": [owner_workspace_rel],
                "removed": [removed_workspace_rel],
                "unchanged": [],
            }
            report["seedOutputs"] = sorted(report["preXmlSha256"])
            report["removedPaths"] = [
                {"path": removed_workspace_rel, "sha256": removed_hash}
            ]
            write_case_report(root, case, report)
            case["files"][0]["seed"] = True
            case["files"][0]["delta"] = "modified"
            case["files"][0]["sha256"] = owner_hash
            case["removedPaths"] = [removed_corpus_rel]
            case["xmlImpact"] = "removed"

            with self.assertRaisesRegex(
                verifier.CorpusError, "outside.*platform checkpoint input|sourcePath"
            ):
                self.load_single(root, case)

    def test_extension_delta_cannot_hide_inside_the_uncompared_base_input(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, kind="extension")
            workspace = case["workspacePath"]
            base_source_rel = f"{workspace}/base"
            base_owner_rel = f"{base_source_rel}/Configuration.xml"
            base_workspace_rel = "base/Configuration.xml"
            base_owner = write(root / base_owner_rel, CONFIG_XML)
            base_hash = sha256(base_owner.read_bytes())
            pre_base_hash, _ = add_pre_file(
                root,
                case,
                base_workspace_rel,
                CONFIG_XML.replace("11111111", "22222222"),
                "metadata",
                source_set_owner=True,
            )
            case["platformCheckpoint"]["baseSourcePath"] = base_source_rel
            case["ownerVersions"][base_owner_rel] = "2.20"
            case["files"].append(
                {
                    "path": base_owner_rel,
                    "sha256": base_hash,
                    "family": "metadata",
                    "seed": True,
                    "delta": "modified",
                }
            )
            case["files"].sort(key=lambda item: item["path"])

            report = read_case_report(root, case)
            report["platformCheckpoint"] = case["platformCheckpoint"]
            report["preFiles"] = case["preFiles"]
            report["preOwnerVersions"] = case["preOwnerVersions"]
            report["ownerVersions"][base_owner_rel] = "2.20"
            report["preXmlSha256"][base_workspace_rel] = pre_base_hash
            report["postXmlSha256"][base_workspace_rel] = base_hash
            report["delta"]["modified"].append(base_workspace_rel)
            report["delta"]["modified"].sort()
            report["seedOutputs"].append(base_workspace_rel)
            report["seedOutputs"].sort()
            report["remainingXml"].append(base_workspace_rel)
            report["remainingXml"].sort()
            write_case_report(root, case, report)

            with self.assertRaisesRegex(
                verifier.CorpusError, "extension.*delta|baseSourcePath"
            ):
                self.load_single(root, case)

    def test_corpus_cannot_claim_a_writer_generated_config_dump_info(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            workspace = case["workspacePath"]
            owner_rel = next(iter(case["ownerVersions"]))
            sidecar_rel = f"{workspace}/src/ConfigDumpInfo.xml"
            sidecar_workspace_rel = "src/ConfigDumpInfo.xml"
            sidecar = write(root / sidecar_rel, "<UnicaGeneratedInvalidSidecar/>")
            sidecar_hash = sha256(sidecar.read_bytes())

            report = read_case_report(root, case)
            report["postXmlSha256"][sidecar_workspace_rel] = sidecar_hash
            report["delta"]["created"].append(sidecar_workspace_rel)
            report["delta"]["created"].sort()
            report["remainingXml"].append(sidecar_workspace_rel)
            report["remainingXml"].sort()
            report["ownerLinks"][sidecar_rel] = owner_rel
            write_case_report(root, case, report)
            case["files"].append(
                {
                    "path": sidecar_rel,
                    "sha256": sidecar_hash,
                    "family": "dump-info",
                    "seed": False,
                    "delta": "created",
                    "ownerPath": owner_rel,
                }
            )
            case["files"].sort(key=lambda item: item["path"])

            with self.assertRaisesRegex(verifier.CorpusError, "ConfigDumpInfo"):
                self.load_single(root, case)

    def test_owner_declaration_and_actual_root_must_be_version_2_20(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            owner_rel = next(iter(case["ownerVersions"]))
            case["ownerVersions"][owner_rel] = "2.19"
            report = read_case_report(root, case)
            report["ownerVersions"][owner_rel] = "2.19"
            write_case_report(root, case, report)
            with self.assertRaisesRegex(verifier.CorpusError, "2.20"):
                self.load_single(root, case)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            owner_rel = next(iter(case["ownerVersions"]))
            write(root / owner_rel, CONFIG_XML.replace('version="2.20"', 'version="2.19"'))
            new_hash = sha256((root / owner_rel).read_bytes())
            case["files"][0]["sha256"] = new_hash
            report = read_case_report(root, case)
            workspace_owner = next(iter(report["postXmlSha256"]))
            report["postXmlSha256"][workspace_owner] = new_hash
            write_case_report(root, case, report)
            with self.assertRaisesRegex(verifier.CorpusError, "owner.*2.20|version.*2.20"):
                self.load_single(root, case)

    def test_versionless_xml_requires_an_explicit_same_checkpoint_owner_link(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            workspace = case["workspacePath"]
            dcs_rel = f"{workspace}/src/Reports/R/Templates/T/Ext/Template.xml"
            dcs = write(
                root / dcs_rel,
                '<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>',
            )
            dcs_hash = sha256(dcs.read_bytes())
            workspace_dcs = dcs_rel.removeprefix(f"{workspace}/")
            report = read_case_report(root, case)
            report["postXmlSha256"][workspace_dcs] = dcs_hash
            report["delta"]["created"].append(workspace_dcs)
            report["delta"]["created"].sort()
            report["remainingXml"].append(workspace_dcs)
            report["remainingXml"].sort()
            write_case_report(root, case, report)
            case["files"].append(
                {
                    "path": dcs_rel,
                    "sha256": dcs_hash,
                    "family": "dcs",
                    "seed": False,
                    "delta": "created",
                }
            )
            case["files"].sort(key=lambda item: item["path"])

            with self.assertRaisesRegex(verifier.CorpusError, "ownerPath"):
                self.load_single(root, case)

            case["files"][-1]["newStandalone"] = True
            with self.assertRaisesRegex(verifier.CorpusError, "newStandalone|platform corpus"):
                self.load_single(root, case)

    def test_versionless_xml_cannot_claim_an_owner_from_another_source_root(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root, kind="extension")
            workspace = case["workspacePath"]
            base_source_rel = f"{workspace}/base"
            base_owner_rel = f"{base_source_rel}/Configuration.xml"
            base_workspace_rel = "base/Configuration.xml"
            base_owner = write(root / base_owner_rel, CONFIG_XML)
            base_hash = sha256(base_owner.read_bytes())
            pre_base_hash, _ = add_pre_file(
                root,
                case,
                base_workspace_rel,
                CONFIG_XML,
                "metadata",
                source_set_owner=True,
            )
            case["platformCheckpoint"]["baseSourcePath"] = base_source_rel
            case["ownerVersions"][base_owner_rel] = "2.20"
            case["files"].append(
                {
                    "path": base_owner_rel,
                    "sha256": base_hash,
                    "family": "metadata",
                    "seed": True,
                    "delta": "unchanged",
                }
            )

            dcs_rel = f"{workspace}/src/Reports/R/Templates/T/Ext/Template.xml"
            dcs_workspace_rel = "src/Reports/R/Templates/T/Ext/Template.xml"
            dcs = write(
                root / dcs_rel,
                '<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>',
            )
            dcs_hash = sha256(dcs.read_bytes())
            case["files"].append(
                {
                    "path": dcs_rel,
                    "sha256": dcs_hash,
                    "family": "dcs",
                    "seed": False,
                    "delta": "created",
                    "ownerPath": base_owner_rel,
                }
            )
            case["files"].sort(key=lambda item: item["path"])

            report = read_case_report(root, case)
            report["platformCheckpoint"] = case["platformCheckpoint"]
            report["preFiles"] = case["preFiles"]
            report["preOwnerVersions"] = case["preOwnerVersions"]
            report["ownerVersions"][base_owner_rel] = "2.20"
            report["ownerLinks"][dcs_rel] = base_owner_rel
            report["preXmlSha256"][base_workspace_rel] = pre_base_hash
            report["postXmlSha256"][base_workspace_rel] = base_hash
            report["postXmlSha256"][dcs_workspace_rel] = dcs_hash
            report["delta"]["created"].append(dcs_workspace_rel)
            report["delta"]["created"].sort()
            report["delta"]["unchanged"].append(base_workspace_rel)
            report["delta"]["unchanged"].sort()
            report["seedOutputs"].append(base_workspace_rel)
            report["seedOutputs"].sort()
            report["remainingXml"].extend([base_workspace_rel, dcs_workspace_rel])
            report["remainingXml"].sort()
            write_case_report(root, case, report)

            with self.assertRaisesRegex(
                verifier.CorpusError, "owner.*root|containing owner|deepest owner"
            ):
                self.load_single(root, case)

    def test_every_mutating_case_is_covered_once_and_mandatory_cases_are_dedicated(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = write_platform_case(root, "cf-init-default")
            second = write_platform_case(root, "form-add-managed")
            second["platformCheckpoint"]["coveredCaseIds"] = []
            second_report = read_case_report(root, second)
            second_report["platformCheckpoint"] = second["platformCheckpoint"]
            write_case_report(root, second, second_report)
            manifest = write_manifest(root, [first, second])
            with self.assertRaisesRegex(verifier.CorpusError, "only itself|coveredCaseIds"):
                verifier.load_corpus(
                    manifest,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default", "form-add-managed"},
                )

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = write_platform_case(root, "cf-init-default")
            second = write_platform_case(root, "form-add-managed")
            second["platformCheckpoint"]["coveredCaseIds"] = [
                "cf-init-default",
                "form-add-managed",
            ]
            second_report = read_case_report(root, second)
            second_report["platformCheckpoint"] = second["platformCheckpoint"]
            write_case_report(root, second, second_report)
            manifest = write_manifest(root, [first, second])
            with self.assertRaisesRegex(verifier.CorpusError, "only itself|coveredCaseIds"):
                verifier.load_corpus(
                    manifest,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default", "form-add-managed"},
                )

    def test_checkpoint_coverage_cannot_be_attributed_to_another_case(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = write_platform_case(root, "alpha")
            second = write_platform_case(root, "beta")
            first["platformCheckpoint"]["coveredCaseIds"] = ["beta"]
            second["platformCheckpoint"]["coveredCaseIds"] = ["alpha"]
            for case in (first, second):
                report = read_case_report(root, case)
                report["platformCheckpoint"] = case["platformCheckpoint"]
                write_case_report(root, case, report)
            manifest = write_manifest(root, [first, second])

            with self.assertRaisesRegex(
                verifier.CorpusError, "checkpoint.*only itself|coveredCaseIds"
            ):
                verifier.load_corpus(
                    manifest,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"alpha", "beta"},
                )

    def test_case_cannot_reuse_another_cases_workspace_or_checkpoint(self):
        verifier = load_verifier()
        for field in ("workspacePath", "checkpoint"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                first = write_platform_case(root, "alpha")
                second = write_platform_case(root, "beta")
                second[field] = first[field]
                if field == "workspacePath":
                    report = read_case_report(root, second)
                    report[field] = first[field]
                    write_case_report(root, second, report)
                manifest = write_manifest(root, [first, second])

                with self.assertRaisesRegex(
                    verifier.CorpusError, "dedicated|canonical|workspace|checkpoint"
                ):
                    verifier.load_corpus(
                        manifest,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"alpha", "beta"},
                    )

    def test_cases_cannot_replay_one_hardlinked_xml_payload(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            first = write_platform_case(root, "alpha")
            second = write_platform_case(root, "beta")
            first_owner = root / next(iter(first["ownerVersions"]))
            second_owner = root / next(iter(second["ownerVersions"]))
            second_owner.unlink()
            os.link(first_owner, second_owner)
            manifest = write_manifest(root, [first, second])

            with self.assertRaisesRegex(
                verifier.SourceError, "hardlink|file identity|multiple corpus paths"
            ):
                verifier.load_corpus(
                    manifest,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"alpha", "beta"},
                )

    def test_corpus_file_cannot_have_an_external_hardlink_alias(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as outside_tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            owner = root / next(iter(case["ownerVersions"]))
            outside_alias = Path(outside_tmp) / "alias.xml"
            os.link(owner, outside_alias)

            with self.assertRaisesRegex(
                verifier.SourceError, "hardlink|link count|external alias"
            ):
                self.load_single(root, case)

    def test_manifest_xml_impact_must_equal_the_validated_delta(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            case["xmlImpact"] = "unchanged"

            with self.assertRaisesRegex(verifier.CorpusError, "xmlImpact.*delta"):
                self.load_single(root, case)

    def test_case_contract_binds_output_signature_and_public_arguments(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            corpus = self.load_single(root, case)
            baseline = verifier.case_contract_sha256([case], corpus["cases"])

            changed_files = copy.deepcopy(case)
            changed_files["files"][0]["path"] = changed_files["files"][0][
                "path"
            ].replace("Configuration.xml", "Unrelated.xml")
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([changed_files], corpus["cases"]),
            )

            changed_call = copy.deepcopy(corpus["cases"])
            changed_call[0]["report"]["publicArguments"]["definition"] = {
                "kind": "unrelated"
            }
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([case], changed_call),
            )

            changed_semantics = copy.deepcopy(corpus["cases"])
            changed_semantics[0]["postSemanticSha256"] = "0" * 64
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([case], changed_semantics),
            )

            changed_pre_semantics = copy.deepcopy(corpus["cases"])
            changed_pre_semantics[0]["preSemanticSha256"] = "0" * 64
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([case], changed_pre_semantics),
            )

            changed_transition = copy.deepcopy(corpus["cases"])
            changed_transition[0]["transitionSemanticSha256"] = "0" * 64
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([case], changed_transition),
            )

            changed_pre_signature = copy.deepcopy(corpus["cases"])
            changed_pre_signature[0]["preSignature"].append(
                {"path": "src/ghost.xml", "family": "dcs", "ownerPath": None}
            )
            self.assertNotEqual(
                baseline,
                verifier.case_contract_sha256([case], changed_pre_signature),
            )

    def test_file_family_and_standalone_claims_must_match_the_xml(self):
        verifier = load_verifier()
        mutations = (
            lambda file: file.update({"family": "dcs"}),
            lambda file: file.update({"newStandalone": True}),
            lambda file: file.update({"newStandalone": False}),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                case = write_platform_case(root)
                mutate(case["files"][0])

                with self.assertRaisesRegex(
                    verifier.CorpusError, "family|newStandalone|standalone"
                ):
                    self.load_single(root, case)

    def test_manifest_and_corpus_root_must_be_absolute_safe_and_outside_repo(self):
        verifier = load_verifier()
        with self.assertRaisesRegex(verifier.CorpusError, "absolute"):
            verifier.load_corpus(Path("relative/corpus-manifest.json"), mandatory_case_ids=set())

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            case = write_platform_case(root)
            repo_manifest = write_manifest(root, [case])
            with self.assertRaisesRegex(verifier.CorpusError, "repository|home"):
                verifier.load_corpus(
                    repo_manifest,
                    repo_root=root,
                    home_root=Path.home(),
                    mandatory_case_ids=set(),
                )


class CommandRunnerTests(unittest.TestCase):
    def make_executable(self, root: Path, body: str, name: str = "fake-command") -> Path:
        script = root / name
        script.write_text(
            "#!/usr/bin/env python3\n" + textwrap.dedent(body), encoding="utf-8"
        )
        script.chmod(script.stat().st_mode | stat.S_IXUSR)
        return script

    def test_records_full_hashes_bounded_text_and_sanitized_argv(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            executable = self.make_executable(
                root,
                """
                import sys
                sys.stdout.write("x" * 200)
                sys.stderr.write("warning")
                """,
            )
            runner = verifier.CommandRunner(timeout_seconds=15, diagnostic_limit=32)

            record = runner.run(
                [str(executable), str(root / "argument")],
                cwd=root,
                redactions=[(root, "$EVIDENCE")],
            )

            self.assertEqual(record["exitCode"], 0)
            self.assertEqual(record["stdoutSha256"], sha256(b"x" * 200))
            self.assertEqual(record["stderrSha256"], sha256(b"warning"))
            self.assertLessEqual(len(record["stdout"]), 32)
            self.assertEqual(record["argv"][0], "$EVIDENCE/fake-command")
            self.assertEqual(record["argv"][1], "$EVIDENCE/argument")
            self.assertNotIn(str(root), json.dumps(record))

    def test_command_environment_does_not_repurpose_home(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with mock.patch.dict(os.environ, {"HOME": "/trusted/original-home"}):
                record = verifier.CommandRunner(timeout_seconds=15).run(
                    ["/usr/bin/env"], cwd=root
                )

            self.assertEqual(record["exitCode"], 0)
            environment = record["stdout"].splitlines()
            self.assertIn("HOME=/trusted/original-home", environment)
            self.assertNotIn(f"HOME={root}", environment)
            self.assertIn(f"TMPDIR={root}", environment)

    def test_nonzero_is_a_command_result_but_spawn_and_timeout_are_source_errors(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            failing = self.make_executable(root, "raise SystemExit(7)\n")
            runner = verifier.CommandRunner(timeout_seconds=15)
            self.assertEqual(runner.run([str(failing)], cwd=root)["exitCode"], 7)

            with self.assertRaisesRegex(verifier.SourceError, "start command"):
                runner.run([str(root / "missing")], cwd=root)

            slow = self.make_executable(
                root,
                """
                import subprocess, sys, time
                subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
                time.sleep(30)
                """,
            )
            timeout_runner = verifier.CommandRunner(timeout_seconds=0.1)
            with self.assertRaisesRegex(verifier.SourceError, "timed out.*process tree"):
                timeout_runner.run([str(slow)], cwd=root)

    def test_argv_must_be_an_array_and_credential_options_are_forbidden(self):
        verifier = load_verifier()
        runner = verifier.CommandRunner(timeout_seconds=1)
        with self.assertRaisesRegex(verifier.SourceError, "argument array"):
            runner.run("echo unsafe", cwd=Path("/tmp"))
        with self.assertRaisesRegex(verifier.SourceError, "credential"):
            runner.run(["/bin/echo", "--password=secret"], cwd=Path("/tmp"))
        for timeout in (0, float("inf"), float("nan")):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(
                ValueError, "finite.*positive"
            ):
                verifier.CommandRunner(timeout_seconds=timeout)

    def test_platform_version_must_equal_exact_8_3_27_build(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exact = self.make_executable(root, 'print("8.3.27.2074")\n', "exact")
            wrong = self.make_executable(root, 'print("8.3.27.9999")\n', "wrong")
            runner = verifier.CommandRunner(timeout_seconds=15)

            version_record = verifier.verify_platform_version(
                exact,
                runner,
                root,
                expected_sha256=sha256(exact.read_bytes()),
            )
            self.assertEqual(version_record["version"], "8.3.27.2074")
            with self.assertRaisesRegex(verifier.SourceError, "8.3.27.2074"):
                verifier.verify_platform_version(
                    wrong,
                    runner,
                    root,
                    expected_sha256=sha256(wrong.read_bytes()),
                )

    def test_platform_binary_hash_must_match_the_pinned_8_3_27_binary(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake = self.make_executable(root, 'print("8.3.27.2074")\n')
            runner = verifier.CommandRunner(timeout_seconds=15)

            with self.assertRaisesRegex(
                verifier.PlatformBinaryError, "SHA-256"
            ) as error:
                verifier.verify_platform_version(
                    fake,
                    runner,
                    root,
                    expected_sha256="0" * 64,
                )

            self.assertEqual(error.exception.ibcmd_sha256, sha256(fake.read_bytes()))


class PlatformInstallInventoryTests(unittest.TestCase):
    def fake_install(self, root: Path):
        verifier = load_verifier()
        install = root / "8.3.27.2074"
        install.mkdir()
        ibcmd = write(install / "ibcmd", "#!/bin/sh\nexit 0\n")
        ibcmd.chmod(ibcmd.stat().st_mode | stat.S_IXUSR)
        (install / "runtime").mkdir()
        (install / "runtime/dynamic.dylib").write_bytes(b"dynamic-one")
        (install / "locale").mkdir()
        (install / "locale/messages.res").write_bytes(b"resource-one")
        snapshot = verifier.capture_platform_install_inventory(ibcmd)
        return verifier, install, ibcmd, snapshot

    def test_checked_in_8_3_27_2074_install_inventory_pin_is_exact(self):
        verifier = load_verifier()
        self.assertEqual(verifier.EXPECTED_PLATFORM_INSTALL_FILE_COUNT, 4337)
        self.assertEqual(
            verifier.EXPECTED_PLATFORM_INSTALL_SHA256,
            "5eb8897c4f7e95876572f2f36943439b0d57e47688314b622f5771e5a22df0ef",
        )

    def test_inventory_is_recursive_deterministic_and_relative(self):
        with tempfile.TemporaryDirectory() as tmp:
            verifier, install, ibcmd, first = self.fake_install(Path(tmp))
            second = verifier.capture_platform_install_inventory(ibcmd)

            self.assertEqual(first, second)
            self.assertEqual(first["root"], install.resolve())
            self.assertEqual(first["fileCount"], 3)
            self.assertRegex(first["sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(
                [entry["path"] for entry in first["files"]],
                ["ibcmd", "locale/messages.res", "runtime/dynamic.dylib"],
            )
            self.assertTrue(
                all(
                    set(entry) == {"path", "type", "mode", "size", "sha256"}
                    and entry["type"] == "file"
                    and entry["mode"].startswith("0")
                    and not Path(entry["path"]).is_absolute()
                    for entry in first["files"]
                )
            )
            self.assertEqual(
                [entry["path"] for entry in first["directories"]],
                [".", "locale", "runtime"],
            )
            self.assertTrue(
                all(
                    set(entry) == {"path", "type", "mode"}
                    and entry["type"] == "directory"
                    and entry["mode"].startswith("0")
                    for entry in first["directories"]
                )
            )

    def test_mode_or_empty_directory_change_never_matches_pin(self):
        for mutation in ("file-mode", "directory-mode", "empty-directory"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                verifier, install, ibcmd, expected = self.fake_install(Path(tmp))
                if mutation == "file-mode":
                    target = install / "runtime/dynamic.dylib"
                    target.chmod(0o600)
                elif mutation == "directory-mode":
                    (install / "runtime").chmod(0o700)
                else:
                    (install / "empty-plugin-directory").mkdir()

                with self.assertRaisesRegex(
                    verifier.PlatformInstallError, "inventory.*does not match"
                ):
                    verifier.verify_platform_install_inventory(
                        ibcmd,
                        expected_sha256=expected["sha256"],
                        expected_file_count=expected["fileCount"],
                    )

    def test_substituted_dynamic_library_or_resource_never_matches_pin(self):
        for relative in ("runtime/dynamic.dylib", "locale/messages.res"):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as tmp:
                verifier, install, ibcmd, expected = self.fake_install(Path(tmp))
                target = install / relative
                target.write_bytes(b"x" * target.stat().st_size)

                with self.assertRaisesRegex(
                    verifier.PlatformInstallError, "inventory.*does not match"
                ):
                    verifier.verify_platform_install_inventory(
                        ibcmd,
                        expected_sha256=expected["sha256"],
                        expected_file_count=expected["fileCount"],
                    )

    def test_added_or_removed_install_file_never_matches_pin(self):
        for mutation in ("added", "removed"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                verifier, install, ibcmd, expected = self.fake_install(Path(tmp))
                if mutation == "added":
                    (install / "late-plugin.dylib").write_bytes(b"late")
                else:
                    (install / "locale/messages.res").unlink()

                with self.assertRaisesRegex(
                    verifier.PlatformInstallError, "inventory.*does not match"
                ):
                    verifier.verify_platform_install_inventory(
                        ibcmd,
                        expected_sha256=expected["sha256"],
                        expected_file_count=expected["fileCount"],
                    )

    def test_symlink_special_entry_and_noncanonical_ibcmd_path_are_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _module, install, ibcmd, _snapshot = self.fake_install(root)
            outside = write(root / "outside.dylib", "outside")
            (install / "escaped-link.dylib").symlink_to(outside)
            with self.assertRaisesRegex(verifier.SourceError, "symlink"):
                verifier.capture_platform_install_inventory(ibcmd)

        if hasattr(os, "mkfifo"):
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                _module, install, ibcmd, _snapshot = self.fake_install(root)
                os.mkfifo(install / "runtime.pipe")
                with self.assertRaisesRegex(verifier.SourceError, "non-regular|special"):
                    verifier.capture_platform_install_inventory(ibcmd)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _module, install, ibcmd, _snapshot = self.fake_install(root)
            noncanonical = install / "runtime" / ".." / ibcmd.name
            with self.assertRaisesRegex(verifier.SourceError, "canonical|escape"):
                verifier.capture_platform_install_inventory(noncanonical)


class CommandBuilderTests(unittest.TestCase):
    def test_configuration_round_uses_proven_option_order(self):
        verifier = load_verifier()
        ibcmd = Path("/opt/1cv8/8.3.27.2074/ibcmd")
        round_root = Path("/evidence/case/round1")
        source = Path("/corpus/case/src")
        export = round_root / "export"

        commands = verifier.build_checkpoint_round_commands(
            ibcmd, "configuration", round_root, source, export
        )
        options = [
            "--db-path=/evidence/case/round1/ib/db",
            "--data=/evidence/case/round1/ib/data",
            "--temp=/evidence/case/round1/ib/temp",
            "--users-data=/evidence/case/round1/ib/users",
            "--session-data=/evidence/case/round1/ib/session",
            "--log-data=/evidence/case/round1/ib/log",
        ]
        self.assertEqual(
            commands,
            [
                {
                    "stage": "import-apply",
                    "argv": [
                        str(ibcmd),
                        "infobase",
                        "create",
                        *options,
                        "--import=/corpus/case/src",
                        "--apply",
                        "--force",
                    ],
                },
                {
                    "stage": "check",
                    "argv": [str(ibcmd), "config", *options, "check", "--force"],
                },
                {
                    "stage": "export",
                    "argv": [str(ibcmd), "config", *options, "export", str(export)],
                },
            ],
        )

    def test_extension_round_imports_fresh_base_then_extension_in_proven_order(self):
        verifier = load_verifier()
        ibcmd = Path("/ibcmd")
        root = Path("/evidence/ext/round2")
        source = Path("/corpus/ext")
        base = Path("/corpus/base")
        output = root / "export"
        commands = verifier.build_checkpoint_round_commands(
            ibcmd, "extension", root, source, output, base_source=base
        )

        self.assertEqual([command["stage"] for command in commands], [
            "base-import-apply",
            "extension-create",
            "extension-import",
            "extension-check",
            "extension-apply",
            "extension-export",
        ])
        self.assertEqual(commands[0]["argv"][-3:], ["--import=/corpus/base", "--apply", "--force"])
        self.assertEqual(
            commands[1]["argv"][-4:],
            [
                "create",
                "--name=CorpusExtension",
                "--name-prefix=CorpusExtension_",
                "--purpose=customization",
            ],
        )
        self.assertEqual(
            commands[2]["argv"][-3:],
            ["import", "--extension=CorpusExtension", "/corpus/ext"],
        )
        self.assertEqual(commands[3]["argv"][-3:], ["check", "--extension=CorpusExtension", "--force"])
        self.assertEqual(commands[4]["argv"][-3:], ["apply", "--extension=CorpusExtension", "--force"])
        self.assertEqual(
            commands[5]["argv"][-3:],
            ["export", "--extension=CorpusExtension", str(output)],
        )

    def test_epf_and_erf_rounds_use_out_and_file_without_credentials(self):
        verifier = load_verifier()
        for kind in ("epf", "erf"):
            with self.subTest(kind=kind):
                root = Path(f"/evidence/{kind}/round1")
                commands = verifier.build_checkpoint_round_commands(
                    Path("/ibcmd"), kind, root, Path(f"/corpus/{kind}"), root / "export"
                )
                artifact = root / f"artifact.{kind}"
                self.assertEqual(
                    [command["stage"] for command in commands],
                    ["empty-infobase-create", "artifact-import", "artifact-export"],
                )
                self.assertEqual(commands[0]["argv"][1:3], ["infobase", "create"])
                self.assertEqual(
                    commands[1]["argv"][-3:],
                    ["import", f"--out={artifact}", f"/corpus/{kind}"],
                )
                self.assertEqual(
                    commands[2]["argv"][-3:],
                    ["export", f"--file={artifact}", f"{root}/export"],
                )
                self.assertFalse(
                    any("password" in argument.lower() for command in commands for argument in command["argv"])
                )

    def test_builder_rejects_missing_extension_base_or_unknown_kind(self):
        verifier = load_verifier()
        with self.assertRaisesRegex(verifier.SourceError, "baseSource"):
            verifier.build_checkpoint_round_commands(
                Path("/ibcmd"), "extension", Path("/evidence"), Path("/source"), Path("/export")
            )
        with self.assertRaisesRegex(verifier.SourceError, "checkpoint kind"):
            verifier.build_checkpoint_round_commands(
                Path("/ibcmd"), "unknown", Path("/evidence"), Path("/source"), Path("/export")
            )


class CheckpointExecutionTests(unittest.TestCase):
    def fake_ibcmd(self, root: Path, mode: str) -> Path:
        install = root / "platform-install"
        install.mkdir(parents=True, exist_ok=True)
        (install / "runtime.dylib").write_bytes(b"synthetic-dynamic-library")
        (install / "messages.res").write_bytes(b"synthetic-resource")
        script = install / f"ibcmd-{mode}"
        script.write_text(
            "#!/usr/bin/env python3\n"
            + textwrap.dedent(
                f"""
                import shutil
                import sys
                from pathlib import Path
                from xml.etree import ElementTree as ET

                MODE = {mode!r}
                args = sys.argv[1:]
                if args == ["--version"]:
                    print("8.3.28.1" if MODE == "wrong-version" else "8.3.27.2074")
                    if MODE == "swap-after-version":
                        with Path(sys.argv[0]).open("a", encoding="utf-8") as stream:
                            stream.write("\\n# swapped after version probe\\n")
                    if MODE == "mutate-install-after-version":
                        Path(sys.argv[0]).with_name("messages.res").write_bytes(
                            b"mutated-resource"
                        )
                    raise SystemExit(0)

                db_arg = next(arg for arg in args if arg.startswith("--db-path="))
                state = Path(db_arg.split("=", 1)[1]).parent / "state"

                def replace_tree(source, destination):
                    if destination.exists():
                        shutil.rmtree(destination)
                    shutil.copytree(source, destination)

                command = args[0]
                if command == "infobase":
                    imported = next((arg.split("=", 1)[1] for arg in args if arg.startswith("--import=")), None)
                    if imported:
                        imported_path = Path(imported)
                        if MODE == "mutate-round1-input" and "round2" in state.parts:
                            candidates = sorted(
                                path for path in imported_path.rglob("*.xml")
                                if path.name != "ConfigDumpInfo.xml"
                            )
                            tree = ET.parse(candidates[0])
                            document = tree.getroot()
                            for child in list(document):
                                if child.tag.rsplit("}}", 1)[-1] == "normalized":
                                    document.remove(child)
                            tree.write(candidates[0], encoding="utf-8", xml_declaration=True)
                        replace_tree(imported_path, state)
                        if MODE == "mutate-rejected":
                            (Path(imported) / "platform-wrote-here.bin").write_bytes(b"forbidden")
                    else:
                        state.mkdir(parents=True, exist_ok=True)
                    raise SystemExit(0)

                if command == "extension":
                    raise SystemExit(0)

                action = next(arg for arg in args if arg in {{"import", "check", "apply", "export"}})
                if (
                    MODE in {{"rejected", "mutate-rejected"}}
                    or (MODE == "reject-round2" and "round2" in state.parts)
                ) and action == "check":
                    print("synthetic rejection", file=sys.stderr)
                    raise SystemExit(9)
                if action == "import":
                    out = next((arg.split("=", 1)[1] for arg in args if arg.startswith("--out=")), None)
                    if out:
                        source = Path(args[-1])
                        if state.exists():
                            shutil.rmtree(state)
                        state.mkdir(parents=True)
                        if source.suffix.lower() == ".xml":
                            descriptor = source
                            content = source.with_suffix("")
                        else:
                            descriptors = sorted(source.glob("*.xml"))
                            descriptor = descriptors[0]
                            content = descriptor.with_suffix("")
                        shutil.copy2(descriptor, state / "descriptor.xml")
                        if content.is_dir():
                            replace_tree(content, state / "content")
                        Path(out).write_bytes(b"artifact")
                    else:
                        replace_tree(Path(args[-1]), state)
                    raise SystemExit(0)
                if action in {{"check", "apply"}}:
                    raise SystemExit(0)

                destination = Path(args[-1])
                if (
                    MODE in {{
                        "mutate-retained-round1-input",
                        "mutate-retained-round1-nonxml",
                        "mutate-retained-round1-empty-dir",
                    }}
                    and "round2" in state.parts
                ):
                    retained_input = Path.cwd().parent / "round1/input/source"
                    if MODE == "mutate-retained-round1-empty-dir":
                        (retained_input / "late-empty").mkdir()
                    elif MODE == "mutate-retained-round1-nonxml":
                        (retained_input / "late-mutation.bin").write_bytes(b"late")
                    else:
                        candidates = sorted(
                            path for path in retained_input.rglob("*.xml")
                            if path.name != "ConfigDumpInfo.xml"
                        )
                        tree = ET.parse(candidates[0])
                        ET.SubElement(tree.getroot(), "lateMutation")
                        tree.write(candidates[0], encoding="utf-8", xml_declaration=True)
                artifact = next(
                    (arg for arg in args if arg.startswith("--file=")), None
                )
                if artifact:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(state / "descriptor.xml", destination.with_suffix(".xml"))
                    if (state / "content").is_dir():
                        replace_tree(state / "content", destination)
                    else:
                        destination.mkdir()
                    raise SystemExit(0)
                replace_tree(state, destination)
                if MODE == "directory-normalized-remove":
                    empty = destination / "declared-empty"
                    if empty.is_dir():
                        empty.rmdir()
                (destination / "ConfigDumpInfo.xml").write_text(
                    f"<dump>{{destination.parent.name}}</dump>", encoding="utf-8"
                )
                if MODE == "nonxml-normalized-add":
                    (destination / "platform-added.bin").write_bytes(b"stable")
                if MODE == "nonxml-normalized-remove":
                    removed = destination / "Ext/Module.bsl"
                    if removed.exists():
                        removed.unlink()
                if MODE == "nonxml-unstable-change":
                    changed = destination / "Ext/Module.bsl"
                    changed.parent.mkdir(parents=True, exist_ok=True)
                    changed.write_bytes(destination.parent.name.encode("utf-8"))
                candidates = sorted(
                    path for path in destination.rglob("*.xml") if path.name != "ConfigDumpInfo.xml"
                )
                if MODE in {{"normalized", "unstable"}} or (
                    MODE == "mutate-round1-input" and destination.parent.name == "round1"
                ):
                    tree = ET.parse(candidates[0])
                    document = tree.getroot()
                    if MODE in {{"normalized", "mutate-round1-input"}}:
                        if not any(child.tag == "normalized" for child in document):
                            ET.SubElement(document, "normalized")
                    else:
                        ET.SubElement(document, "unstable").text = destination.parent.name
                    tree.write(candidates[0], encoding="utf-8", xml_declaration=True)
                raise SystemExit(0)
                """
            ),
            encoding="utf-8",
        )
        script.chmod(script.stat().st_mode | stat.S_IXUSR)
        return script

    def checkpoint_item(self, source: Path, *, kind: str = "configuration", base: Path | None = None):
        owner_name = "Artifact.xml" if kind in {"epf", "erf"} else "Configuration.xml"
        def expected_hashes(root: Path | None, *, xml: bool):
            if root is None:
                return None
            return {
                path.relative_to(root).as_posix(): sha256(path.read_bytes())
                for path in sorted(root.rglob("*"))
                if path.is_file() and (path.suffix.lower() == ".xml") == xml
            }
        def expected_empty_directories(root: Path | None):
            if root is None:
                return None
            return sorted(
                path.relative_to(root).as_posix()
                for path in root.rglob("*")
                if path.is_dir() and not any(path.iterdir())
            )
        return {
            "id": f"synthetic-{kind}",
            "toolId": "unica.synthetic",
            "checkpoint": {"kind": kind, "coveredCaseIds": [f"synthetic-{kind}"]},
            "source": source,
            "baseSource": base,
            "sourceOwnerRelativePaths": [owner_name],
            "ownerVersions": {str(source / owner_name): "2.20"},
            "sourceExpectedXmlHashes": expected_hashes(source, xml=True),
            "sourceExpectedNonXmlHashes": expected_hashes(source, xml=False),
            "sourceExpectedEmptyDirectoryPaths": expected_empty_directories(source),
            "baseExpectedXmlHashes": expected_hashes(base, xml=True),
            "baseExpectedNonXmlHashes": expected_hashes(base, xml=False),
            "baseExpectedEmptyDirectoryPaths": expected_empty_directories(base),
        }

    def test_pass_normalized_unstable_and_rejected_verdicts(self):
        verifier = load_verifier()
        expected = {
            "pass": "pass",
            "normalized": "accepted-normalized",
            "unstable": "unstable-roundtrip",
            "rejected": "rejected",
        }
        for mode, verdict in expected.items():
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / "source"
                write(source / "Configuration.xml", CONFIG_XML)
                ibcmd = self.fake_ibcmd(root, mode)
                evidence = root / "evidence"
                runner = verifier.CommandRunner(timeout_seconds=15)

                result = verifier.run_checkpoint(
                    self.checkpoint_item(source), ibcmd, runner, evidence, root
                )

                self.assertEqual(result["verdict"], verdict)
                self.assertEqual(result["commandCount"], 2 if verdict == "rejected" else 6)
                self.assertNotIn(str(root), json.dumps(result))
                if verdict == "rejected":
                    self.assertEqual(result["failedStage"], "check")
                else:
                    self.assertTrue(result["roundtripComparison"] is not None)

    def test_non_xml_add_remove_and_change_affect_platform_verdicts(self):
        verifier = load_verifier()
        for mode, verdict in (
            ("nonxml-normalized-add", "accepted-normalized"),
            ("nonxml-normalized-remove", "accepted-normalized"),
            ("nonxml-unstable-change", "unstable-roundtrip"),
        ):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / "source"
                write(source / "Configuration.xml", CONFIG_XML)
                module = source / "Ext/Module.bsl"
                module.parent.mkdir(parents=True)
                module.write_bytes(b"source")

                result = verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, mode),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

                self.assertEqual(result["verdict"], verdict, result)
                self.assertFalse(
                    result["sourceComparison"]["nonXml"]["equal"], result
                )
                if verdict == "unstable-roundtrip":
                    self.assertFalse(
                        result["roundtripComparison"]["nonXml"]["equal"], result
                    )
                else:
                    self.assertTrue(
                        result["roundtripComparison"]["nonXml"]["equal"], result
                    )

    def test_platform_removal_of_empty_directory_is_accepted_normalized(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)
            (source / "declared-empty").mkdir()

            result = verifier.run_checkpoint(
                self.checkpoint_item(source),
                self.fake_ibcmd(root, "directory-normalized-remove"),
                verifier.CommandRunner(timeout_seconds=15),
                root / "evidence",
                root,
            )

            self.assertEqual(result["verdict"], "accepted-normalized", result)
            self.assertFalse(result["sourceComparison"]["directories"]["equal"])
            self.assertEqual(
                result["sourceComparison"]["directories"]["removed"],
                ["declared-empty"],
            )
            self.assertTrue(result["roundtripComparison"]["directories"]["equal"])

    def test_manifest_bound_input_hashes_reject_mutation_before_private_copy(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)
            module = source / "Ext/Module.bsl"
            module.parent.mkdir(parents=True)
            module.write_bytes(b"declared")
            item = self.checkpoint_item(source)
            module.write_bytes(b"changed-after-corpus-validation")

            with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                verifier.run_checkpoint(
                    item,
                    self.fake_ibcmd(root, "pass"),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

            result = raised.exception.checkpoint
            self.assertEqual(result["failedStage"], "private-input-snapshot", result)
            self.assertEqual(result["commandCount"], 0, result)

    def test_manifest_bound_empty_directories_reject_topology_mutation_before_private_copy(self):
        verifier = load_verifier()
        for mutation in ("added", "removed"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / "source"
                write(source / "Configuration.xml", CONFIG_XML)
                declared_empty = source / "declared-empty"
                declared_empty.mkdir()
                item = self.checkpoint_item(source)
                if mutation == "added":
                    (source / "late-empty").mkdir()
                else:
                    declared_empty.rmdir()

                with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                    verifier.run_checkpoint(
                        item,
                        self.fake_ibcmd(root, "pass"),
                        verifier.CommandRunner(timeout_seconds=15),
                        root / "evidence",
                        root,
                    )

                result = raised.exception.checkpoint
                self.assertEqual(result["failedStage"], "private-input-snapshot", result)
                self.assertEqual(result["commandCount"], 0, result)

    def test_round_two_cannot_mutate_away_round_one_evidence(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)

            with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "mutate-round1-input"),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

            result = raised.exception.checkpoint
            self.assertEqual(result["verdict"], "source-error", result)
            self.assertIn("input-integrity", result["failedStage"])
            self.assertEqual(
                set(result["evidenceSha256"]),
                {
                    "sourceXml",
                    "sourceNonXml",
                    "sourceEmptyDirectoryPaths",
                    "round1InputXml",
                    "round1InputNonXml",
                    "round1InputEmptyDirectoryPaths",
                    "export1Xml",
                    "export1NonXml",
                    "export1EmptyDirectoryPaths",
                    "round2InputXml",
                    "round2InputNonXml",
                    "round2InputEmptyDirectoryPaths",
                },
            )
            round_two_import = next(
                command
                for command in result["commands"]
                if command["round"] == 2 and command["stage"] == "import-apply"
            )
            self.assertIn(
                "--import=$EVIDENCE/round2/input/source", round_two_import["argv"]
            )

    def test_late_mutation_of_retained_round_one_input_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)

            with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "mutate-retained-round1-input"),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

            result = raised.exception.checkpoint
            self.assertEqual(result["verdict"], "source-error", result)
            self.assertEqual(result["failedRound"], 2, result)
            self.assertEqual(result["failedStage"], "retained-evidence", result)
            self.assertEqual(result["commandCount"], 6, result)
            self.assertEqual(
                set(result["evidenceSha256"]),
                {
                    "sourceXml",
                    "sourceNonXml",
                    "sourceEmptyDirectoryPaths",
                    "round1InputXml",
                    "round1InputNonXml",
                    "round1InputEmptyDirectoryPaths",
                    "export1Xml",
                    "export1NonXml",
                    "export1EmptyDirectoryPaths",
                    "round2InputXml",
                    "round2InputNonXml",
                    "round2InputEmptyDirectoryPaths",
                    "export2Xml",
                    "export2NonXml",
                    "export2EmptyDirectoryPaths",
                },
            )

    def test_late_non_xml_mutation_of_retained_input_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)

            with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "mutate-retained-round1-nonxml"),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

            result = raised.exception.checkpoint
            self.assertEqual(result["verdict"], "source-error", result)
            self.assertEqual(result["failedStage"], "retained-evidence", result)
            self.assertIn("export2NonXml", result["evidenceSha256"])

    def test_late_empty_directory_mutation_of_retained_input_is_rejected(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)

            with self.assertRaises(verifier.CheckpointExecutionError) as raised:
                verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "mutate-retained-round1-empty-dir"),
                    verifier.CommandRunner(timeout_seconds=15),
                    root / "evidence",
                    root,
                )

            result = raised.exception.checkpoint
            self.assertEqual(result["verdict"], "source-error", result)
            self.assertEqual(result["failedStage"], "retained-evidence", result)
            self.assertIn(
                "round1InputEmptyDirectoryPaths", result["evidenceSha256"]
            )

    def test_round_two_rejection_retains_all_completed_input_and_export_hashes(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)

            result = verifier.run_checkpoint(
                self.checkpoint_item(source),
                self.fake_ibcmd(root, "reject-round2"),
                verifier.CommandRunner(timeout_seconds=15),
                root / "evidence",
                root,
            )

            self.assertEqual(result["verdict"], "rejected", result)
            self.assertEqual(result["failedRound"], 2)
            self.assertEqual(
                set(result["evidenceSha256"]),
                {
                    "sourceXml",
                    "sourceNonXml",
                    "sourceEmptyDirectoryPaths",
                    "round1InputXml",
                    "round1InputNonXml",
                    "round1InputEmptyDirectoryPaths",
                    "export1Xml",
                    "export1NonXml",
                    "export1EmptyDirectoryPaths",
                    "round2InputXml",
                    "round2InputNonXml",
                    "round2InputEmptyDirectoryPaths",
                },
            )

    def test_verdict_and_hashes_use_the_same_immutable_xml_payloads(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)
            evidence = root / "evidence"
            original_compare = verifier.compare_xml_directories
            mutated = False

            def mutate_before_live_compare(left, right):
                nonlocal mutated
                if not mutated:
                    write(evidence / "round1/export/Configuration.xml", CONFIG_XML)
                    write(evidence / "round2/export/Configuration.xml", CONFIG_XML)
                    mutated = True
                return original_compare(left, right)

            with mock.patch.object(
                verifier, "compare_xml_directories", side_effect=mutate_before_live_compare
            ):
                result = verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "normalized"),
                    verifier.CommandRunner(timeout_seconds=15),
                    evidence,
                    root,
                )

            self.assertEqual(result["verdict"], "accepted-normalized", result)
            self.assertFalse(result["sourceComparison"]["equal"], result)
            self.assertNotEqual(
                result["evidenceSha256"]["sourceXml"],
                result["evidenceSha256"]["export1Xml"],
            )

    def test_extension_and_artifact_kinds_execute_both_fresh_rounds(self):
        verifier = load_verifier()
        expected_counts = {"extension": 12, "epf": 6, "erf": 6}
        for kind, command_count in expected_counts.items():
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / "source"
                if kind in {"epf", "erf"}:
                    write(source / "Artifact.xml", CONFIG_XML)
                    write(source / "Artifact/Forms/Form.xml", "<Form/>")
                else:
                    write(source / "Configuration.xml", CONFIG_XML)
                base = None
                if kind == "extension":
                    base = root / "base"
                    write(base / "Configuration.xml", CONFIG_XML)
                evidence = root / "evidence"
                result = verifier.run_checkpoint(
                    self.checkpoint_item(source, kind=kind, base=base),
                    self.fake_ibcmd(root, "pass"),
                    verifier.CommandRunner(timeout_seconds=15),
                    evidence,
                    root,
                )

                self.assertEqual(result["verdict"], "pass")
                self.assertEqual(result["commandCount"], command_count)
                round1_db = "$EVIDENCE/round1/ib/db"
                round2_db = "$EVIDENCE/round2/ib/db"
                argv_text = json.dumps(result["commands"])
                self.assertIn(round1_db, argv_text)
                self.assertIn(round2_db, argv_text)
                round1_import = next(
                    command
                    for command in result["commands"]
                    if command["round"] == 1
                    and command["stage"]
                    in {"base-import-apply", "empty-infobase-create"}
                )
                if kind == "extension":
                    self.assertIn(
                        "--import=$EVIDENCE/round1/input/base", round1_import["argv"]
                    )
                    extension_import = next(
                        command
                        for command in result["commands"]
                        if command["round"] == 1
                        and command["stage"] == "extension-import"
                    )
                    self.assertEqual(
                        extension_import["argv"][-1], "$EVIDENCE/round1/input/source"
                    )
                if kind in {"epf", "erf"}:
                    artifact_import = next(
                        command
                        for command in result["commands"]
                        if command["round"] == 1
                        and command["stage"] == "artifact-import"
                    )
                    self.assertEqual(
                        artifact_import["argv"][-1], "$EVIDENCE/round1/input/source"
                    )
                    round2_import = next(
                        command
                        for command in result["commands"]
                        if command["round"] == 2
                        and command["stage"] == "artifact-import"
                    )
                    self.assertEqual(
                        round2_import["argv"][-1], "$EVIDENCE/round2/input/source"
                    )
                    self.assertTrue(
                        (evidence / "round2/input/source/export.xml").is_file()
                    )
                    self.assertEqual(
                        sorted(result["evidenceSha256"]["export1Xml"]),
                        ["content/Forms/Form.xml", "descriptor.xml"],
                    )

    def test_checkpoint_refuses_nonempty_evidence_target(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            write(source / "Configuration.xml", CONFIG_XML)
            evidence = root / "evidence"
            evidence.mkdir()
            write(evidence / "sentinel", "keep")
            with self.assertRaisesRegex(verifier.SourceError, "empty|exists"):
                verifier.run_checkpoint(
                    self.checkpoint_item(source),
                    self.fake_ibcmd(root, "pass"),
                    verifier.CommandRunner(timeout_seconds=15),
                    evidence,
                    root,
                )


class GateAndReportTests(unittest.TestCase):
    def synthetic_gate(self, root: Path, *, fake_mode: str = "pass"):
        verifier = load_verifier()
        corpus_root = root / "corpus"
        corpus_root.mkdir()
        case = write_platform_case(corpus_root)
        manifest = write_manifest(corpus_root, [case])
        evidence = root / "evidence"
        evidence.mkdir()
        report = root / "platform-report.json"
        fake = CheckpointExecutionTests().fake_ibcmd(root, fake_mode)
        for name in (
            "EXPECTED_IBCMD_SHA256",
            "EXPECTED_PLATFORM_INSTALL_SHA256",
            "EXPECTED_PLATFORM_INSTALL_FILE_COUNT",
        ):
            self.addCleanup(setattr, verifier, name, getattr(verifier, name))
        verifier.EXPECTED_IBCMD_SHA256 = sha256(fake.read_bytes())
        inventory = verifier.capture_platform_install_inventory(fake)
        verifier.EXPECTED_PLATFORM_INSTALL_SHA256 = inventory["sha256"]
        verifier.EXPECTED_PLATFORM_INSTALL_FILE_COUNT = inventory["fileCount"]
        return verifier, manifest, evidence, report, fake

    def test_full_tree_snapshot_covers_non_xml_and_rejects_symlinks(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "a.xml", "<a/>")
            (root / "binary.bin").write_bytes(b"one")
            first = verifier.snapshot_regular_tree(root)
            (root / "binary.bin").write_bytes(b"two")
            second = verifier.snapshot_regular_tree(root)
            self.assertNotEqual(first, second)
            self.assertEqual(sorted(first["files"]), ["a.xml", "binary.bin"])
            self.assertEqual(first["emptyDirectoryPaths"], [])

            (root / "empty").mkdir()
            with_empty = verifier.snapshot_regular_tree(root)
            self.assertEqual(with_empty["files"], second["files"])
            self.assertEqual(with_empty["emptyDirectoryPaths"], ["empty"])
            self.assertNotEqual(with_empty, second)

            (root / "link").symlink_to(root / "a.xml")
            with self.assertRaisesRegex(verifier.SourceError, "symlink"):
                verifier.snapshot_regular_tree(root)

    def test_gate_binds_parsed_manifest_and_refuses_post_validation_swap(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            original_load = verifier.load_corpus
            loaded = original_load(
                manifest,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )
            self.assertEqual(
                loaded["manifestSha256"], sha256(manifest.read_bytes())
            )
            self.assertEqual(
                loaded["snapshot"]["files"]["corpus-manifest.json"],
                loaded["manifestSha256"],
            )

            def load_then_swap(*args, **kwargs):
                corpus = original_load(*args, **kwargs)
                owner = manifest.parent / "cases/cf-init-default/workspace/src/Configuration.xml"
                owner.write_text(
                    CONFIG_XML.replace("11111111", "aaaaaaaa"),
                    encoding="utf-8",
                )
                return corpus

            with mock.patch.object(
                verifier, "load_corpus", side_effect=load_then_swap
            ), mock.patch.object(
                verifier,
                "verify_platform_version",
                side_effect=AssertionError("platform command must not run"),
            ) as version_probe:
                with self.assertRaisesRegex(
                    verifier.SourceError,
                    "changed after validation and before platform execution",
                ):
                    verifier.execute_gate(
                        ibcmd=ibcmd,
                        corpus_manifest=manifest,
                        report_path=report_path,
                        evidence_dir=evidence,
                        timeout_seconds=15,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"cf-init-default"},
                    )
                version_probe.assert_not_called()

    def test_gate_refuses_empty_directory_swap_after_corpus_validation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            original_load = verifier.load_corpus

            def load_then_add_empty_directory(*args, **kwargs):
                corpus = original_load(*args, **kwargs)
                (
                    manifest.parent
                    / "cases/cf-init-default/workspace/late-empty"
                ).mkdir()
                return corpus

            with mock.patch.object(
                verifier, "load_corpus", side_effect=load_then_add_empty_directory
            ), mock.patch.object(
                verifier,
                "verify_platform_version",
                side_effect=AssertionError("platform command must not run"),
            ) as version_probe:
                with self.assertRaisesRegex(
                    verifier.SourceError,
                    "changed after validation and before platform execution",
                ):
                    verifier.execute_gate(
                        ibcmd=ibcmd,
                        corpus_manifest=manifest,
                        report_path=report_path,
                        evidence_dir=evidence,
                        timeout_seconds=15,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"cf-init-default"},
                    )
                version_probe.assert_not_called()

    def test_evidence_and_report_paths_are_fail_closed(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            home = root / "home"
            corpus = root / "corpus"
            safe = root / "safe"
            for directory in (repo, home, corpus, safe):
                directory.mkdir()

            self.assertEqual(
                verifier.validate_evidence_directory(safe, repo, home, corpus), safe.resolve()
            )
            for unsafe in (Path("relative"), Path("/"), repo, home, corpus):
                with self.subTest(unsafe=unsafe):
                    with self.assertRaisesRegex(verifier.SourceError, "evidence"):
                        verifier.validate_evidence_directory(unsafe, repo, home, corpus)
            write(safe / "sentinel", "keep")
            with self.assertRaisesRegex(verifier.SourceError, "empty"):
                verifier.validate_evidence_directory(safe, repo, home, corpus)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            home = root / "home"
            corpus = root / "corpus"
            output = root / "out"
            for directory in (repo, home, corpus, output):
                directory.mkdir()
            safe_report = output / "report.json"
            self.assertEqual(
                verifier.validate_report_path(safe_report, repo, home, corpus),
                safe_report.resolve(strict=False),
            )
            for unsafe in (Path("relative.json"), repo / "report.json", corpus / "report.json"):
                with self.subTest(unsafe=unsafe):
                    with self.assertRaisesRegex(verifier.SourceError, "report"):
                        verifier.validate_report_path(unsafe, repo, home, corpus)

    def test_gate_writes_deterministic_shape_and_exit_zero_for_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            before = verifier.snapshot_regular_tree(manifest.parent)

            with mock.patch.object(
                verifier,
                "capture_platform_install_inventory",
                wraps=verifier.capture_platform_install_inventory,
            ) as capture_install:
                exit_code, report = verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=evidence,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertEqual(exit_code, 0)
            self.assertEqual(report["status"], "pass")
            self.assertEqual(report["expectedPlatformVersion"], "8.3.27.2074")
            self.assertEqual(report["observedPlatformVersion"], "8.3.27.2074")
            self.assertEqual(report["platformVersion"], "8.3.27.2074")
            self.assertEqual(report["summary"]["pass"], 1)
            self.assertEqual(report["summary"]["commandCount"], 6)
            self.assertEqual(report["coverage"]["processedCaseIds"], ["cf-init-default"])
            self.assertEqual(
                report["comparisonPolicy"]["nonXml"],
                "exact logical path and byte equality",
            )
            self.assertEqual(
                report["comparisonPolicy"]["directories"],
                "exact empty-directory logical path equality",
            )
            self.assertEqual(capture_install.call_count, 2)
            platform_install = report["provenance"]["platformInstall"]
            self.assertEqual(
                platform_install,
                {
                    "expectedSha256": verifier.EXPECTED_PLATFORM_INSTALL_SHA256,
                    "expectedFileCount": verifier.EXPECTED_PLATFORM_INSTALL_FILE_COUNT,
                    "beforeSha256": verifier.EXPECTED_PLATFORM_INSTALL_SHA256,
                    "beforeFileCount": verifier.EXPECTED_PLATFORM_INSTALL_FILE_COUNT,
                    "beforeDirectoryCount": 1,
                    "afterSha256": verifier.EXPECTED_PLATFORM_INSTALL_SHA256,
                    "afterFileCount": verifier.EXPECTED_PLATFORM_INSTALL_FILE_COUNT,
                    "afterDirectoryCount": 1,
                    "unchanged": True,
                    "verified": True,
                },
            )
            self.assertEqual(verifier.snapshot_regular_tree(manifest.parent), before)
            on_disk = json.loads(report_path.read_text(encoding="utf-8"))
            self.assertEqual(on_disk, report)
            self.assertTrue(report_path.read_bytes().endswith(b"\n"))
            self.assertNotIn(str(root), json.dumps(report))

    def test_gate_rejects_an_unpinned_executable_before_running_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            verifier.EXPECTED_IBCMD_SHA256 = "0" * 64

            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "platform-binary-mismatch")
            self.assertIsNone(report["observedPlatformVersion"])
            self.assertIsNone(report["provenance"]["versionCheck"])
            self.assertEqual(report["provenance"]["ibcmdSha256"], sha256(ibcmd.read_bytes()))
            self.assertEqual(report["summary"]["versionCommandCount"], 0)

    def test_gate_rejects_ibcmd_replacement_after_the_version_probe(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(
                root, fake_mode="swap-after-version"
            )
            pinned_hash = verifier.EXPECTED_IBCMD_SHA256

            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "platform-source-error")
            self.assertIn("ibcmd changed", report["sourceError"]["message"])
            self.assertEqual(report["provenance"]["expectedIbcmdSha256"], pinned_hash)
            self.assertNotEqual(sha256(ibcmd.read_bytes()), pinned_hash)
            self.assertEqual(report["coverage"]["processedCaseIds"], [])

    def test_gate_rejects_install_inventory_mismatch_before_version_command(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            (ibcmd.parent / "runtime.dylib").write_bytes(b"substituted-dynamic-library")

            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(
                report["sourceError"]["code"], "platform-install-mismatch"
            )
            self.assertIsNone(report["provenance"]["versionCheck"])
            platform_install = report["provenance"]["platformInstall"]
            self.assertNotEqual(
                platform_install["beforeSha256"],
                platform_install["expectedSha256"],
            )
            self.assertEqual(
                platform_install["beforeFileCount"],
                platform_install["expectedFileCount"],
            )
            self.assertEqual(report["summary"]["versionCommandCount"], 0)

    def test_report_target_inside_platform_install_is_rejected_before_commands(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, _report_path, ibcmd = self.synthetic_gate(root)
            unsafe_report = ibcmd.parent / "platform-report.json"

            with self.assertRaisesRegex(
                verifier.SourceError, "report.*platform install"
            ):
                verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=unsafe_report,
                    evidence_dir=evidence,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertFalse(unsafe_report.exists())
            self.assertFalse((evidence / "control").exists())

    def test_evidence_target_inside_platform_install_is_rejected_before_commands(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, _evidence, report_path, ibcmd = self.synthetic_gate(root)
            unsafe_evidence = ibcmd.parent / "evidence"
            unsafe_evidence.mkdir()

            with self.assertRaisesRegex(
                verifier.SourceError, "evidence.*platform install"
            ):
                verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=unsafe_evidence,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertFalse((unsafe_evidence / "control").exists())

    def test_gate_rejects_platform_install_mutation_between_snapshots(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(
                root, fake_mode="mutate-install-after-version"
            )

            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(
                report["sourceError"]["code"], "platform-install-mutated"
            )
            platform_install = report["provenance"]["platformInstall"]
            self.assertNotEqual(
                platform_install["beforeSha256"],
                platform_install["afterSha256"],
            )
            self.assertFalse(platform_install["unchanged"])
            self.assertFalse(platform_install["verified"])

    def test_implicit_evidence_directory_is_validated_and_cleaned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, _evidence, report_path, ibcmd = self.synthetic_gate(root)
            implicit = root / "implicit-evidence"

            class ControlledTemporaryDirectory:
                def __init__(self, *args, **kwargs):
                    implicit.mkdir()
                    self.name = str(implicit)
                    self.cleaned = False

                def cleanup(self):
                    self.cleaned = True
                    if implicit.exists():
                        import shutil

                        shutil.rmtree(implicit)

            controlled = ControlledTemporaryDirectory.__new__(ControlledTemporaryDirectory)

            def factory(*args, **kwargs):
                ControlledTemporaryDirectory.__init__(controlled, *args, **kwargs)
                return controlled

            with mock.patch.object(
                verifier.tempfile, "TemporaryDirectory", side_effect=factory
            ), mock.patch.object(
                verifier,
                "validate_evidence_directory",
                wraps=verifier.validate_evidence_directory,
            ) as validate:
                exit_code, report = verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=None,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertEqual(exit_code, 0)
            self.assertEqual(report["status"], "pass")
            validate.assert_called_once_with(
                implicit.resolve(), ROOT.resolve(), Path.home().resolve(), manifest.parent.resolve()
            )
            self.assertTrue(controlled.cleaned)
            self.assertFalse(implicit.exists())

    def test_evidence_setup_failure_becomes_a_reported_source_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            original_mkdir = Path.mkdir

            def failing_control_mkdir(path, *args, **kwargs):
                if path.name == "control" and path.parent.resolve() == evidence.resolve():
                    raise OSError("synthetic permission failure")
                return original_mkdir(path, *args, **kwargs)

            with mock.patch.object(Path, "mkdir", new=failing_control_mkdir):
                exit_code, report = verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=evidence,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "platform-source-error")
            self.assertIn("control directory", report["sourceError"]["message"])
            self.assertTrue(report_path.is_file())

    def test_normalized_rejected_and_unstable_are_exit_one(self):
        for mode, verdict in (
            ("normalized", "accepted-normalized"),
            ("rejected", "rejected"),
            ("unstable", "unstable-roundtrip"),
        ):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(
                    root, fake_mode=mode
                )
                exit_code, report = verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=evidence,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )
                self.assertEqual(exit_code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertEqual(report["checkpoints"][0]["verdict"], verdict)

    def test_platform_receives_a_private_copy_and_cannot_mutate_the_corpus(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(
                root, fake_mode="mutate-rejected"
            )

            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["checkpoints"][0]["verdict"], "source-error")
            self.assertIn(
                "input-integrity", report["checkpoints"][0]["failedStage"]
            )
            self.assertTrue(report["corpusIntegrity"]["unchanged"])
            self.assertFalse(
                (
                    manifest.parent
                    / "cases/cf-init-default/workspace/src/platform-wrote-here.bin"
                ).exists()
            )
            self.assertTrue(
                (
                    evidence
                    / "cf-init-default/round1/input/source/platform-wrote-here.bin"
                ).is_file()
            )
            self.assertFalse(
                (
                    evidence
                    / "cf-init-default/input/source/platform-wrote-here.bin"
                ).exists()
            )
            self.assertEqual(report["coverage"]["processedCaseIds"], [])

    def test_partial_checkpoint_commands_survive_a_later_source_error(self):
        class PartialRunner:
            def __init__(self):
                self.calls = 0

            def run(self, argv, *, cwd, redactions=None):
                self.calls += 1
                if self.calls == 1:
                    return {
                        "argv": ["$IBCMD", "--version"],
                        "exitCode": 0,
                        "stdoutSha256": sha256(b"8.3.27.2074\n"),
                        "stderrSha256": sha256(b""),
                        "stdout": "8.3.27.2074\n",
                        "stderr": "",
                        "durationMs": 1,
                    }
                if self.calls == 2:
                    return {
                        "argv": ["$IBCMD", "infobase", "create"],
                        "exitCode": 0,
                        "stdoutSha256": sha256(b""),
                        "stderrSha256": sha256(b""),
                        "stdout": "",
                        "stderr": "",
                        "durationMs": 2,
                    }
                raise verifier.SourceError("synthetic command infrastructure failure")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
                runner=PartialRunner(),
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["summary"]["commandCount"], 1)
            self.assertEqual(report["summary"]["source-error"], 1)
            partial = report["checkpoints"][0]
            self.assertEqual(partial["verdict"], "source-error")
            self.assertEqual(partial["failedRound"], 1)
            self.assertEqual(partial["failedStage"], "check")
            self.assertEqual(partial["commandCount"], 1)
            self.assertEqual(report["coverage"]["processedCaseIds"], [])

    def test_source_error_still_checks_full_corpus_snapshot_and_marks_incomplete(self):
        verifier = load_verifier()

        class RaisingRunner:
            def __init__(self, corpus_root: Path):
                self.calls = 0
                self.corpus_root = corpus_root

            def run(self, argv, *, cwd, redactions=None):
                self.calls += 1
                if self.calls == 1:
                    empty_hash = sha256(b"")
                    return {
                        "argv": argv,
                        "exitCode": 0,
                        "stdoutSha256": sha256(b"8.3.27.2074\n"),
                        "stderrSha256": empty_hash,
                        "stdout": "8.3.27.2074\n",
                        "stderr": "",
                        "durationMs": 1,
                    }
                (self.corpus_root / "mutation.bin").write_bytes(b"bad")
                raise verifier.SourceError("synthetic infrastructure failure")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _module, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            runner = RaisingRunner(manifest.parent)
            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
                runner=runner,
            )
            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "corpus-mutated")
            self.assertEqual(report["coverage"]["processedCaseIds"], [])
            self.assertEqual(report["coverage"]["unprocessedCaseIds"], ["cf-init-default"])

    def test_uninspectable_final_corpus_never_reports_unchanged(self):
        verifier = load_verifier()

        class SymlinkMutatingRunner:
            def __init__(self, corpus_root: Path):
                self.calls = 0
                self.corpus_root = corpus_root

            def run(self, argv, *, cwd, redactions=None):
                self.calls += 1
                if self.calls == 1:
                    empty_hash = sha256(b"")
                    return {
                        "argv": ["$IBCMD", "--version"],
                        "exitCode": 0,
                        "stdoutSha256": sha256(b"8.3.27.2074\n"),
                        "stderrSha256": empty_hash,
                        "stdout": "8.3.27.2074\n",
                        "stderr": "",
                        "durationMs": 1,
                    }
                (self.corpus_root / "unsafe-link").symlink_to(
                    self.corpus_root / "corpus-manifest.json"
                )
                raise verifier.SourceError("synthetic infrastructure failure")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _module, manifest, evidence, report_path, ibcmd = self.synthetic_gate(root)
            exit_code, report = verifier.execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
                runner=SymlinkMutatingRunner(manifest.parent),
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["sourceError"]["code"], "corpus-unsafe")
            self.assertFalse(report["corpusIntegrity"]["unchanged"])
            self.assertIsNone(report["corpusIntegrity"]["afterSha256"])

    def test_temporary_evidence_creation_error_is_a_source_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, _evidence, report_path, ibcmd = self.synthetic_gate(root)
            with mock.patch.object(
                verifier.tempfile,
                "TemporaryDirectory",
                side_effect=OSError("synthetic temp failure"),
            ):
                with self.assertRaisesRegex(
                    verifier.SourceError, "temporary evidence directory"
                ):
                    verifier.execute_gate(
                        ibcmd=ibcmd,
                        corpus_manifest=manifest,
                        report_path=report_path,
                        evidence_dir=None,
                        timeout_seconds=15,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"cf-init-default"},
                    )

    def test_temporary_evidence_is_cleaned_when_report_write_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, _evidence, report_path, ibcmd = self.synthetic_gate(root)
            implicit = root / "implicit-evidence"

            class ControlledTemporaryDirectory:
                def __init__(self):
                    implicit.mkdir()
                    self.name = str(implicit)
                    self.cleaned = False

                def cleanup(self):
                    self.cleaned = True

            controlled = ControlledTemporaryDirectory()
            with mock.patch.object(
                verifier.tempfile, "TemporaryDirectory", return_value=controlled
            ), mock.patch.object(
                verifier,
                "_atomic_write_report",
                side_effect=verifier.SourceError("synthetic report failure"),
            ):
                with self.assertRaisesRegex(verifier.SourceError, "report failure"):
                    verifier.execute_gate(
                        ibcmd=ibcmd,
                        corpus_manifest=manifest,
                        report_path=report_path,
                        evidence_dir=None,
                        timeout_seconds=15,
                        repo_root=ROOT,
                        home_root=Path.home(),
                        mandatory_case_ids={"cf-init-default"},
                    )

            self.assertTrue(controlled.cleaned)

    def test_temporary_cleanup_failure_is_reported_and_never_leaves_pass_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, _evidence, report_path, ibcmd = self.synthetic_gate(root)
            implicit = root / "implicit-evidence"

            class FailingTemporaryDirectory:
                def __init__(self):
                    implicit.mkdir()
                    self.name = str(implicit)

                def cleanup(self):
                    raise OSError("synthetic cleanup failure")

            with mock.patch.object(
                verifier.tempfile,
                "TemporaryDirectory",
                return_value=FailingTemporaryDirectory(),
            ):
                exit_code, report = verifier.execute_gate(
                    ibcmd=ibcmd,
                    corpus_manifest=manifest,
                    report_path=report_path,
                    evidence_dir=None,
                    timeout_seconds=15,
                    repo_root=ROOT,
                    home_root=Path.home(),
                    mandatory_case_ids={"cf-init-default"},
                )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "evidence-cleanup-failed")
            self.assertIn("cleanup failure", report["sourceError"]["message"])
            self.assertEqual(json.loads(report_path.read_text(encoding="utf-8")), report)

    def test_version_mismatch_retains_the_completed_version_command_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _verifier, manifest, evidence, report_path, ibcmd = self.synthetic_gate(
                root, fake_mode="wrong-version"
            )

            exit_code, report = load_verifier().execute_gate(
                ibcmd=ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["status"], "source-error")
            self.assertEqual(report["sourceError"]["code"], "platform-version-mismatch")
            self.assertEqual(report["expectedPlatformVersion"], "8.3.27.2074")
            self.assertEqual(report["observedPlatformVersion"], "8.3.28.1")
            self.assertEqual(report["platformVersion"], "8.3.28.1")
            version = report["provenance"]["versionCheck"]
            self.assertEqual(version["argv"], ["$IBCMD", "--version"])
            self.assertEqual(version["exitCode"], 0)
            self.assertEqual(version["stdout"], "8.3.28.1\n")
            self.assertEqual(version["stdoutSha256"], sha256(b"8.3.28.1\n"))
            self.assertEqual(report["summary"]["versionCommandCount"], 1)
            self.assertEqual(report["provenance"]["ibcmdSha256"], sha256(ibcmd.read_bytes()))
            self.assertEqual(report["coverage"]["processedCaseIds"], [])

    def test_missing_ibcmd_path_is_redacted_in_the_source_error_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            verifier, manifest, evidence, report_path, _ibcmd = self.synthetic_gate(root)
            missing_ibcmd = root / "missing-ibcmd"

            exit_code, report = verifier.execute_gate(
                ibcmd=missing_ibcmd,
                corpus_manifest=manifest,
                report_path=report_path,
                evidence_dir=evidence,
                timeout_seconds=15,
                repo_root=ROOT,
                home_root=Path.home(),
                mandatory_case_ids={"cf-init-default"},
            )

            self.assertEqual(exit_code, 2)
            self.assertEqual(report["sourceError"]["code"], "platform-source-error")
            self.assertIn("$IBCMD", report["sourceError"]["message"])
            self.assertNotIn(str(root), json.dumps(report))

    def test_report_exit_semantics_are_total(self):
        verifier = load_verifier()
        self.assertEqual(verifier.report_exit_code({"status": "pass"}), 0)
        self.assertEqual(verifier.report_exit_code({"status": "failed"}), 1)
        self.assertEqual(verifier.report_exit_code({"status": "source-error"}), 2)
        with self.assertRaisesRegex(verifier.SourceError, "report status"):
            verifier.report_exit_code({"status": "inconclusive"})


class CliTests(unittest.TestCase):
    def test_main_uses_a_platform_safe_default_command_timeout(self):
        verifier = load_verifier()
        with mock.patch.object(
            verifier, "execute_gate", return_value=(0, {"status": "pass"})
        ) as execute:
            exit_code = verifier.main(
                [
                    "--ibcmd",
                    "/opt/1cv8/8.3.27.2074/ibcmd",
                    "--corpus",
                    "/tmp/corpus/corpus-manifest.json",
                    "--report",
                    "/tmp/report.json",
                ]
            )

        self.assertEqual(exit_code, 0)
        self.assertEqual(verifier.DEFAULT_COMMAND_TIMEOUT_SECONDS, 300.0)
        execute.assert_called_once_with(
            ibcmd=Path("/opt/1cv8/8.3.27.2074/ibcmd"),
            corpus_manifest=Path("/tmp/corpus/corpus-manifest.json"),
            report_path=Path("/tmp/report.json"),
            evidence_dir=None,
            timeout_seconds=300.0,
        )

    def test_main_forwards_the_documented_cli_contract(self):
        verifier = load_verifier()
        with mock.patch.object(
            verifier, "execute_gate", return_value=(1, {"status": "failed"})
        ) as execute:
            exit_code = verifier.main(
                [
                    "--ibcmd",
                    "/opt/1cv8/8.3.27.2074/ibcmd",
                    "--corpus",
                    "/tmp/corpus/corpus-manifest.json",
                    "--report",
                    "/tmp/report.json",
                    "--evidence-dir",
                    "/tmp/evidence",
                    "--timeout",
                    "17.5",
                ]
            )

        self.assertEqual(exit_code, 1)
        execute.assert_called_once_with(
            ibcmd=Path("/opt/1cv8/8.3.27.2074/ibcmd"),
            corpus_manifest=Path("/tmp/corpus/corpus-manifest.json"),
            report_path=Path("/tmp/report.json"),
            evidence_dir=Path("/tmp/evidence"),
            timeout_seconds=17.5,
        )

    def test_cli_entrypoint_exposes_the_documented_required_arguments(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--help"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        for option in ("--ibcmd", "--corpus", "--report", "--evidence-dir"):
            self.assertIn(option, result.stdout)

    def test_cli_preflight_source_error_is_exit_two_without_traceback(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            gate_fixture = GateAndReportTests()
            _verifier, manifest, evidence, report_path, _ibcmd = (
                gate_fixture.synthetic_gate(root)
            )
            gate_fixture.doCleanups()
            missing_ibcmd = root / "missing-ibcmd"
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--ibcmd",
                    str(missing_ibcmd),
                    "--corpus",
                    str(manifest),
                    "--report",
                    str(report_path),
                    "--evidence-dir",
                    str(evidence),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=10,
            )

            self.assertEqual(result.returncode, 2)
            self.assertNotIn("Traceback", result.stderr)
            self.assertIn("source error:", result.stderr)
            self.assertFalse(report_path.exists())

    def test_main_rejects_preflight_source_error_without_traceback(self):
        verifier = load_verifier()
        with mock.patch.object(
            verifier, "execute_gate", side_effect=verifier.SourceError("unsafe corpus")
        ), mock.patch("sys.stderr") as stderr:
            exit_code = verifier.main(
                [
                    "--ibcmd",
                    "/missing/ibcmd",
                    "--corpus",
                    "/unsafe/corpus.json",
                    "--report",
                    "/tmp/report.json",
                ]
            )

        self.assertEqual(exit_code, 2)
        rendered = "".join(call.args[0] for call in stderr.write.call_args_list)
        self.assertIn("source error: unsafe corpus", rendered)
        self.assertNotIn("Traceback", rendered)

    def test_timeout_must_be_finite_and_positive(self):
        verifier = load_verifier()
        for timeout in ("0", "inf", "nan"):
            with self.subTest(timeout=timeout):
                with self.assertRaises(SystemExit) as error, mock.patch("sys.stderr"):
                    verifier.main(
                        [
                            "--ibcmd",
                            "/tmp/ibcmd",
                            "--corpus",
                            "/tmp/corpus.json",
                            "--report",
                            "/tmp/report.json",
                            "--timeout",
                            timeout,
                        ]
                    )
                self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
