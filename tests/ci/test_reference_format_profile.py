from __future__ import annotations

import codecs
import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
import xml.parsers.expat
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
UNICA_REFERENCE_MODELS = ROOT / "tests/fixtures/unica_mcp_script_parity/unica_reference_models"
SUBSYSTEM_EDIT = UNICA_REFERENCE_MODELS / "subsystem-edit/scripts/subsystem-edit.py"
TEMPLATE_ADD = UNICA_REFERENCE_MODELS / "template-add/scripts/add-template.py"
META_VALIDATE = UNICA_REFERENCE_MODELS / "meta-validate/scripts/meta-validate.py"
MXL_COMPILE = UNICA_REFERENCE_MODELS / "mxl-compile/scripts/mxl-compile.py"
DCS_COMPILE = UNICA_REFERENCE_MODELS / "dcs-compile/scripts/dcs-compile.py"
VALIDATOR_SCRIPTS = tuple(
    UNICA_REFERENCE_MODELS / relative
    for relative in (
        "cf-validate/scripts/cf-validate.py",
        "cfe-validate/scripts/cfe-validate.py",
        "form-validate/scripts/form-validate.py",
        "meta-validate/scripts/meta-validate.py",
        "subsystem-validate/scripts/subsystem-validate.py",
    )
)
MD_NS = "http://v8.1c.ru/8.3/MDClasses"
MXL_NS = "http://v8.1c.ru/8.2/data/spreadsheet"
XDTO_NS = "http://v8.1c.ru/8.1/xdto"
XSI_NS = "http://www.w3.org/2001/XMLSchema-instance"
XDTO_SPEC = ROOT / "plugins/unica/references/specs/1c-xdto-spec.md"
XDTO_FIXTURE = ROOT / "tests/fixtures/xdto/enterprise-data-minimal"
XDTO_CONTRACT_START = "<!-- xdto-evidence-contract:start -->"
XDTO_CONTRACT_END = "<!-- xdto-evidence-contract:end -->"
XDTO_DONOR_START = "<!-- xdto-donor-evidence:start -->"
XDTO_DONOR_END = "<!-- xdto-donor-evidence:end -->"


class ReconfigurableStringIO(io.StringIO):
    def reconfigure(self, **_kwargs) -> None:
        pass


def run_script(script: Path, *arguments: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(script), *arguments],
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def load_script(path: Path, module_name: str):
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load script: {path}")
    module = importlib.util.module_from_spec(spec)
    previous = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        spec.loader.exec_module(module)
    finally:
        sys.dont_write_bytecode = previous
    return module


def subsystem_xml(version: str | None) -> str:
    version_attribute = "" if version is None else f' version="{version}"'
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        f'<MetaDataObject xmlns="{MD_NS}"{version_attribute}>'
        "<Subsystem><Properties><Name>Sales</Name></Properties>"
        "<ChildObjects/><Content/></Subsystem></MetaDataObject>\n"
    )


def xdto_contract_rows(specification: str) -> dict[tuple[str, str], str]:
    start = specification.index(XDTO_CONTRACT_START) + len(XDTO_CONTRACT_START)
    end = specification.index(XDTO_CONTRACT_END, start)
    rows: dict[tuple[str, str], str] = {}
    for line in specification[start:end].splitlines():
        cells = [cell.strip().strip("`") for cell in line.strip().strip("|").split("|")]
        if len(cells) != 3 or cells[0] in {"Status", "---"}:
            continue
        rows[(cells[0], cells[1])] = cells[2]
    return rows


def analyze_xdto_qnames(
    package_bytes: bytes,
) -> tuple[
    list[dict[str, object]],
    set[str],
    list[tuple[str, tuple[str, str]]],
]:
    parser = xml.parsers.expat.ParserCreate(namespace_separator="}")
    pending_namespaces: list[tuple[str, str]] = []
    namespace_scopes: list[dict[str, str]] = [{}]
    elements: list[tuple[int, str]] = []
    properties: list[dict[str, object]] = []
    property_by_element: dict[int, dict[str, object]] = {}
    qname_namespaces: set[str] = set()
    qname_references: list[tuple[str, tuple[str, str]]] = []
    next_element_id = 0

    def start_namespace(prefix: str | None, uri: str) -> None:
        pending_namespaces.append((prefix or "", uri))

    def resolve_qname(value: str, scope: dict[str, str]) -> tuple[str, str]:
        prefix, separator, local = value.partition(":")
        if not separator or not prefix or not local:
            raise AssertionError(f"QName must use a declared prefix: {value}")
        if prefix not in scope:
            raise AssertionError(f"QName prefix is out of scope: {value}")
        return scope[prefix], local

    def start_element(name: str, attrs: dict[str, str]) -> None:
        nonlocal next_element_id
        scope = namespace_scopes[-1].copy()
        scope.update(pending_namespaces)
        pending_namespaces.clear()
        namespace_scopes.append(scope)
        local = name.rsplit("}", 1)[-1]
        parent_id, parent_local = elements[-1] if elements else (-1, "")
        element_id = next_element_id
        next_element_id += 1

        unqualified = {key: value for key, value in attrs.items() if "}" not in key}
        resolved: dict[str, tuple[str, str]] = {}
        for attribute in ("type", "base", "ref"):
            if attribute in unqualified:
                resolved[attribute] = resolve_qname(unqualified[attribute], scope)
                qname_namespaces.add(resolved[attribute][0])
                qname_references.append((attribute, resolved[attribute]))

        if local == "property":
            record = {
                "parent_id": parent_id,
                "parent_local": parent_local,
                "name": unqualified.get("name"),
                "ref": unqualified.get("ref"),
                "resolved_ref": resolved.get("ref"),
                "type": unqualified.get("type"),
                "lower": unqualified.get("lowerBound"),
                "upper": unqualified.get("upperBound"),
                "nested_type_def": False,
            }
            properties.append(record)
            property_by_element[element_id] = record
        elif local == "typeDef" and parent_local == "property":
            property_by_element[parent_id]["nested_type_def"] = True
        elements.append((element_id, local))

    def end_element(_name: str) -> None:
        elements.pop()
        namespace_scopes.pop()

    parser.StartNamespaceDeclHandler = start_namespace
    parser.StartElementHandler = start_element
    parser.EndElementHandler = end_element
    parser.Parse(package_bytes, True)
    return properties, qname_namespaces, qname_references


class ReferenceFormatProfileTests(unittest.TestCase):
    def test_xdto_spec_names_the_real_pinned_donor_evidence(self) -> None:
        specification = XDTO_SPEC.read_text(encoding="utf-8")
        expected_paths = {
            "docs/xdto-guide.md",
            "docs/xdto-dsl-spec.md",
            ".claude/skills/xdto-compile/SKILL.md",
            ".claude/skills/xdto-decompile/SKILL.md",
            ".claude/skills/xdto-edit/SKILL.md",
            ".claude/skills/xdto-info/SKILL.md",
            ".claude/skills/xdto-validate/SKILL.md",
            "tests/skills/cases/xdto-compile/",
            "tests/skills/cases/xdto-decompile/",
            "tests/skills/cases/xdto-edit/",
            "tests/skills/cases/xdto-info/",
            "tests/skills/cases/xdto-validate/",
        }

        start = specification.index(XDTO_DONOR_START) + len(XDTO_DONOR_START)
        end = specification.index(XDTO_DONOR_END, start)
        evidence = {
            line.strip()
            for line in specification[start:end].splitlines()
            if line.strip() and not line.strip().startswith("```")
        }

        self.assertEqual(evidence, expected_paths)
        self.assertNotIn("docs/1c-xdto-spec.md", evidence)

    def test_xdto_spec_is_indexed_and_declares_exact_evidence_contract(self) -> None:
        specification = XDTO_SPEC.read_text(encoding="utf-8")
        readme = (XDTO_SPEC.parent / "README.md").read_text(encoding="utf-8")
        index = (XDTO_SPEC.parent / "format-index.md").read_text(encoding="utf-8")
        configuration = (XDTO_SPEC.parent / "1c-configuration-spec.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("1c-xdto-spec.md", readme)
        self.assertIn("1c-xdto-spec.md", index)
        self.assertIn("1c-xdto-spec.md", configuration)
        self.assertIn("2067778ba3bad527bd1e5850304d1c82acb81fc8", specification)
        self.assertEqual(
            xdto_contract_rows(specification),
            {
                ("supported", "package/import"): "direct-child",
                ("supported", "package/property"): "direct-child",
                ("supported", "package/valueType"): "direct-child",
                ("supported", "package/objectType"): "direct-child",
                ("supported", "objectType/property"): "direct-child",
                ("supported", "property/typeDef:ObjectType"): "direct-child",
                ("supported", "typeDef:ObjectType/property"): "direct-child",
                ("supported", "property-identity"): "exactly-one(name,ref)",
                ("supported", "ref-target"): "global-property",
                ("supported", "owned-type"): "zero-or-one(type,typeDef:ObjectType)",
                ("supported", "lowerBound"): "0-or-1;default=1",
                ("supported", "upperBound"): "-1-or-integer>=1;default=1",
                ("supported", "finite-bounds"): "lower<=upper",
                ("unsupported", "valueType/enumeration"): "writer-contract",
                ("unsupported", "valueType/pattern"): "writer-contract",
                ("unsupported", "valueType/typeDef:ValueType"): "writer-contract",
                ("unsupported", "property/typeDef:ValueType"): "writer-contract",
                ("unsupported", "valueType/memberTypes"): "writer-contract",
            },
        )

    def test_xdto_fixture_proves_profile_target_and_supported_package_shape(self) -> None:
        package_name = "EnterpriseData_1_17_3"
        target_namespace = "http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3"
        descriptor_path = XDTO_FIXTURE / "XDTOPackages" / f"{package_name}.xml"
        package_path = (
            XDTO_FIXTURE / "XDTOPackages" / package_name / "Ext" / "Package.bin"
        )

        for platform_xml_path in (
            XDTO_FIXTURE / "Configuration.xml",
            descriptor_path,
        ):
            with self.subTest(platform_xml_path=platform_xml_path):
                platform_xml_bytes = platform_xml_path.read_bytes()
                self.assertTrue(platform_xml_bytes.startswith(codecs.BOM_UTF8))
                platform_xml_body = platform_xml_bytes[len(codecs.BOM_UTF8) :]
                self.assertIn(b"\r\n", platform_xml_body)
                self.assertNotIn(b"\n", platform_xml_body.replace(b"\r\n", b""))
                self.assertNotIn(b"\r", platform_xml_body.replace(b"\r\n", b""))
                self.assertTrue(platform_xml_body.endswith(b"\r\n"))

        configuration_root = ET.parse(XDTO_FIXTURE / "Configuration.xml").getroot()
        self.assertEqual(configuration_root.tag, f"{{{MD_NS}}}MetaDataObject")
        self.assertEqual(configuration_root.attrib.get("version"), "2.20")
        configuration_children = list(configuration_root)
        self.assertEqual(len(configuration_children), 1)
        configuration = configuration_children[0]
        self.assertEqual(configuration.tag, f"{{{MD_NS}}}Configuration")
        child_objects = configuration.find(f"{{{MD_NS}}}ChildObjects")
        self.assertIsNotNone(child_objects)
        registrations = child_objects.findall(f"{{{MD_NS}}}XDTOPackage")
        self.assertEqual([registration.text for registration in registrations], [package_name])

        descriptor_root = ET.parse(descriptor_path).getroot()
        self.assertEqual(descriptor_root.tag, f"{{{MD_NS}}}MetaDataObject")
        self.assertEqual(descriptor_root.attrib.get("version"), "2.20")
        descriptor_children = list(descriptor_root)
        self.assertEqual(len(descriptor_children), 1)
        descriptor = descriptor_children[0]
        self.assertEqual(descriptor.tag, f"{{{MD_NS}}}XDTOPackage")
        properties = descriptor.find(f"{{{MD_NS}}}Properties")
        self.assertIsNotNone(properties)
        self.assertEqual(
            properties.findtext(f"{{{MD_NS}}}Name"),
            package_name,
        )
        self.assertEqual(
            properties.findtext(f"{{{MD_NS}}}Namespace"),
            target_namespace,
        )

        package_bytes = package_path.read_bytes()
        self.assertTrue(package_bytes.startswith(codecs.BOM_UTF8))
        body = package_bytes[len(codecs.BOM_UTF8) :]
        self.assertIn(b"\r\n", body)
        self.assertNotIn(b"\n", body.replace(b"\r\n", b""))
        self.assertNotIn(b"\r", body.replace(b"\r\n", b""))
        self.assertTrue(body.endswith(b"\r\n"))
        self.assertIn(
            '<objectType name="СоставнойЛюбойОбъект"/>\r\n'.encode(), body
        )

        package = ET.fromstring(package_bytes)
        self.assertEqual(package.tag, f"{{{XDTO_NS}}}package")
        self.assertEqual(package.attrib.get("targetNamespace"), target_namespace)
        child_kinds = [
            child.tag.removeprefix(f"{{{XDTO_NS}}}") for child in package
        ]
        rank = {
            name: position
            for position, name in enumerate(
                ("import", "property", "valueType", "objectType")
            )
        }
        self.assertEqual(child_kinds, sorted(child_kinds, key=rank.__getitem__))
        self.assertTrue(
            {"import", "property", "valueType", "objectType"} <= set(child_kinds)
        )

        contract = xdto_contract_rows(XDTO_SPEC.read_text(encoding="utf-8"))
        supported_edges = {
            construct
            for (status, construct), rule in contract.items()
            if status == "supported" and rule == "direct-child"
        }
        observed_edges: set[str] = set()
        for parent in package.iter():
            parent_kind = parent.tag.removeprefix(f"{{{XDTO_NS}}}")
            if parent_kind == "typeDef":
                self.assertEqual(
                    parent.attrib.get(f"{{{XSI_NS}}}type"), "ObjectType"
                )
                parent_kind = "typeDef:ObjectType"
            for child in parent:
                child_kind = child.tag.removeprefix(f"{{{XDTO_NS}}}")
                if child_kind == "typeDef":
                    self.assertEqual(
                        child.attrib.get(f"{{{XSI_NS}}}type"), "ObjectType"
                    )
                    child_kind = "typeDef:ObjectType"
                observed_edges.add(f"{parent_kind}/{child_kind}")
        self.assertEqual(observed_edges, supported_edges)

        imports = {
            child.attrib["namespace"]
            for child in package.findall(f"{{{XDTO_NS}}}import")
        }
        self.assertIn("http://v8.1c.ru/8.1/data/core", imports)
        self.assertIn("http://v8.1c.ru/8.1/data/enterprise/current-config", imports)

        named_types = [
            child.attrib["name"]
            for child in package
            if child.tag
            in {f"{{{XDTO_NS}}}valueType", f"{{{XDTO_NS}}}objectType"}
        ]
        self.assertEqual(len(named_types), len(set(named_types)))

        package_properties, qname_namespaces, qname_references = (
            analyze_xdto_qnames(package_bytes)
        )
        self.assertEqual(
            qname_namespaces,
            {
                "http://www.w3.org/2001/XMLSchema",
                target_namespace,
                "http://v8.1c.ru/8.1/data/core",
                "http://v8.1c.ru/8.1/data/enterprise/current-config",
            },
        )
        self.assertTrue(
            qname_namespaces
            - {target_namespace, "http://www.w3.org/2001/XMLSchema"}
            <= imports
        )
        for attribute, (namespace, local) in qname_references:
            if attribute == "ref":
                continue
            if namespace == target_namespace:
                self.assertIn(local, named_types)
            elif namespace != "http://www.w3.org/2001/XMLSchema":
                self.assertIn(namespace, imports)

        global_properties = {
            (target_namespace, record["name"])
            for record in package_properties
            if record["parent_local"] == "package" and record["name"] is not None
        }
        property_groups: dict[int, list[tuple[str, object]]] = {}
        effective_bounds: dict[str, tuple[int, int]] = {}
        for record in package_properties:
            has_name = record["name"] is not None
            has_ref = record["ref"] is not None
            self.assertNotEqual(has_name, has_ref)
            if has_ref:
                self.assertIsNone(record["type"])
                self.assertFalse(record["nested_type_def"])
                self.assertIn(record["resolved_ref"], global_properties)
                identity = record["resolved_ref"]
                label = f"ref:{record['ref']}"
            else:
                self.assertFalse(record["type"] and record["nested_type_def"])
                identity = (target_namespace, record["name"])
                label = str(record["name"])
            property_groups.setdefault(int(record["parent_id"]), []).append(identity)

            lower = int(record["lower"] or 1)
            upper = int(record["upper"] or 1)
            self.assertIn(lower, {0, 1})
            self.assertTrue(upper == -1 or upper >= 1)
            if upper != -1:
                self.assertLessEqual(lower, upper)
            effective_bounds[label] = (lower, upper)

        for identities in property_groups.values():
            self.assertEqual(len(identities), len(set(identities)))
        self.assertEqual(effective_bounds["EnterpriseData"], (1, 1))
        self.assertEqual(effective_bounds["Версия"], (0, 1))
        self.assertEqual(effective_bounds["Идентификаторы"], (1, 3))
        self.assertEqual(effective_bounds["Документ_ЗаказКлиента"], (0, -1))
        self.assertEqual(effective_bounds["ref:tns:EnterpriseData"], (1, 1))

        global_property = package.find(f"{{{XDTO_NS}}}property")
        self.assertIsNotNone(global_property)
        self.assertEqual(global_property.attrib.get("type"), "tns:EnterpriseData")

        any_reference = package.find(
            f"{{{XDTO_NS}}}objectType[@name='ЛюбаяСсылка']"
        )
        self.assertIsNotNone(any_reference)
        reference_property = any_reference.find(
            f"{{{XDTO_NS}}}property[@name='СсылкаНаОбъект']"
        )
        self.assertIsNotNone(reference_property)
        nested = reference_property.find(f"{{{XDTO_NS}}}typeDef")
        self.assertIsNotNone(nested)
        self.assertEqual(nested.attrib.get(f"{{{XSI_NS}}}type"), "ObjectType")

    def test_dcs_compile_validates_before_printing_success(self) -> None:
        source = DCS_COMPILE.read_text(encoding="utf-8")

        validation = source.rindex("run_post_validation(output_path)")
        success = source.rindex('print(f"OK  {args.OutputPath}")')

        self.assertLess(validation, success)

    def test_subsystem_edit_rejects_nonexact_owner_before_write(self) -> None:
        for version in (None, "2.19", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                subsystem = root / "Sales.xml"
                before = subsystem_xml(version).encode()
                subsystem.write_bytes(before)

                result = run_script(
                    SUBSYSTEM_EDIT,
                    "-SubsystemPath",
                    str(subsystem),
                    "-Operation",
                    "add-content",
                    "-Value",
                    "Catalog.Item",
                    "-NoValidate",
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("expected exact '2.20'", result.stderr)
                self.assertEqual(subsystem.read_bytes(), before)

    def test_subsystem_edit_restores_parent_if_child_stub_creation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            subsystem = root / "Sales.xml"
            before = subsystem_xml("2.20").encode()
            subsystem.write_bytes(before)
            conflict = root / "Sales" / "Subsystems"
            conflict.parent.mkdir()
            conflict.write_text("not a directory", encoding="utf-8")

            result = run_script(
                SUBSYSTEM_EDIT,
                "-SubsystemPath",
                str(subsystem),
                "-Operation",
                "add-child",
                "-Value",
                "Broken",
                "-NoValidate",
                cwd=root,
            )

            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertIn("Failed to publish subsystem edit", result.stderr)
            self.assertEqual(subsystem.read_bytes(), before)

    def test_subsystem_edit_removes_created_directory_chain_on_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            subsystem = root / "Sales.xml"
            before = subsystem_xml("2.20").encode()
            subsystem.write_bytes(before)
            script = load_script(SUBSYSTEM_EDIT, "reference_subsystem_edit")

            with (
                mock.patch.object(
                    sys,
                    "argv",
                    [
                        str(SUBSYSTEM_EDIT),
                        "-SubsystemPath",
                        str(subsystem),
                        "-Operation",
                        "add-child",
                        "-Value",
                        "Broken",
                        "-NoValidate",
                    ],
                ),
                mock.patch.object(
                    script,
                    "write_child_subsystem_stub",
                    side_effect=OSError("forced child write failure"),
                ),
                contextlib.redirect_stdout(ReconfigurableStringIO()),
                contextlib.redirect_stderr(ReconfigurableStringIO()),
                self.assertRaises(SystemExit) as raised,
            ):
                script.main()

            self.assertEqual(raised.exception.code, 1)
            self.assertEqual(subsystem.read_bytes(), before)
            self.assertFalse((root / "Sales").exists())

    def test_template_add_uses_object_owner_and_rejects_nonexact_version(self) -> None:
        for version in (None, "2.19", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                reports = root / "src" / "Reports"
                reports.mkdir(parents=True)
                owner = reports / "Sales.xml"
                owner.write_text(
                    subsystem_xml(version).replace("Subsystem", "Report"),
                    encoding="utf-8",
                )

                result = run_script(
                    TEMPLATE_ADD,
                    "-ObjectName",
                    "Sales",
                    "-TemplateName",
                    "Main",
                    "-TemplateType",
                    "Text",
                    "-SrcDir",
                    "src",
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("expected exact '2.20'", result.stderr)
                self.assertFalse((reports / "Sales").exists())

    def test_reference_validators_reject_malformed_and_numeric_equivalent_versions(self) -> None:
        for script in VALIDATOR_SCRIPTS:
            with self.subTest(script=script):
                source = script.read_text(encoding="utf-8")
                self.assertIn("re.fullmatch", source)
                self.assertRegex(source, r"actual == ['\"]2\.20['\"]")

        for version in ("-1.0", "+2.20", "2.20.0"):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                owner = root / "Catalog.xml"
                owner.write_text(
                    '<?xml version="1.0" encoding="UTF-8"?>\n'
                    f'<MetaDataObject xmlns="{MD_NS}" version="{version}">'
                    "<Catalog><Properties><Name>Item</Name></Properties></Catalog>"
                    "</MetaDataObject>\n",
                    encoding="utf-8",
                )

                result = run_script(
                    META_VALIDATE,
                    "-ObjectPath",
                    str(owner),
                    cwd=root,
                )

                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn(f"invalid export format version '{version}'", result.stdout)

    def test_reference_mxl_writer_uses_span_for_implicit_next_column(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            definition = root / "mxl.json"
            output = root / "Template.xml"
            definition.write_text(
                json.dumps(
                    {
                        "columns": 5,
                        "areas": [
                            {
                                "name": "A",
                                "rows": [
                                    {
                                        "cells": [
                                            {"col": 1, "span": 2, "text": "spanned"},
                                            {"col": 3, "text": "adjacent"},
                                            {"col": 5, "text": "after gap"},
                                        ]
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            result = run_script(
                MXL_COMPILE,
                "-JsonPath",
                str(definition),
                "-OutputPath",
                str(output),
                cwd=root,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            row = ET.parse(output).find(f".//{{{MXL_NS}}}row")
            self.assertIsNotNone(row)
            cells = row.findall(f"{{{MXL_NS}}}c")
            self.assertEqual(len(cells), 3)
            self.assertIsNone(cells[1].find(f"{{{MXL_NS}}}i"))
            self.assertEqual(cells[2].findtext(f"{{{MXL_NS}}}i"), "4")


if __name__ == "__main__":
    unittest.main()
