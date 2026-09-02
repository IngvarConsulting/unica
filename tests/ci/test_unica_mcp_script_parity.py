from __future__ import annotations

import argparse
import dataclasses
import hashlib
import io
import json
import os
import platform
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid
import xml.etree.ElementTree as ET
import zipfile
from collections.abc import Callable, Iterable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from unittest import mock
from xml.sax.saxutils import escape

MODULE_REPO_ROOT = Path(__file__).resolve().parents[2]
if str(MODULE_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(MODULE_REPO_ROOT))

from scripts.ci import donor_parity_contract as donor_contract


REPO_ROOT = MODULE_REPO_ROOT
PLUGIN_ROOT = REPO_ROOT / "plugins" / "unica"
SKILLS_ROOT = PLUGIN_ROOT / "skills"
FIXTURES_ROOT = REPO_ROOT / "tests" / "fixtures" / "unica_mcp_script_parity"
UNICA_REFERENCE_MODELS_ROOT = FIXTURES_ROOT / "unica_reference_models"
DONOR_SNAPSHOT_ROOT = Path(
    os.environ.get("UNICA_DONOR_SNAPSHOT_ROOT", FIXTURES_ROOT / "cc-1c-skills")
).resolve()
DONOR_SKILLS_ROOT = DONOR_SNAPSHOT_ROOT / "skills"
CC_1C_CASES_ROOT = DONOR_SNAPSHOT_ROOT / "cases"
DONOR_BASELINE_PATH = FIXTURES_ROOT / "donor-baseline.json"
DONOR_RELATIONS_PATH = FIXTURES_ROOT / "donor-relations.json"
BSP_DCS_QUERY_FIXTURE = (
    "bsp/dcs/Catalogs__ПравилаОбработкиЭлектроннойПочты__"
    "СхемаПравилаОбработкиЭлектроннойПочты/Template.xml"
)
BSP_DCS_UNION_FIXTURE = (
    "bsp/dcs/DataProcessors__ВыгрузкаЗагрузкаEnterpriseData__"
    "СхемаКомпоновкиДанных/Template.xml"
)
BSP_DCS_OBJECT_FIXTURE = (
    "bsp/dcs/DataProcessors__ЗаменаИОбъединениеЭлементов__"
    "ОсновнаяСхемаКомпоновкиДанных/Template.xml"
)
BSP_CF_CONFIGURATION_FIXTURE = "bsp/cf/Configuration.xml"
BSP_META_CATALOG_FIXTURE = "bsp/meta/Catalogs/Валюты.xml"
BSP_META_DOCUMENT_FIXTURE = "bsp/meta/Documents/АктОбУничтоженииПерсональныхДанных.xml"
BSP_META_REPORT_FIXTURE = "bsp/meta/Reports/АнализВерсийОбъектов.xml"
BSP_META_REPORT_TEMPLATE_FIXTURE = (
    "bsp/meta/Reports/АнализВерсийОбъектов/Templates/ОсновнаяСхемаКомпоновкиДанных.xml"
)
BSP_META_REPORT_TEMPLATE_CONTENT_FIXTURE = (
    "bsp/meta/Reports/АнализВерсийОбъектов/Templates/"
    "ОсновнаяСхемаКомпоновкиДанных/Ext/Template.xml"
)
BSP_META_COMMON_MODULE_FIXTURE = "bsp/meta/CommonModules/GoogleПереводчик.xml"
BSP_META_COMMON_MODULE_BSL_FIXTURE = "bsp/meta/CommonModules/GoogleПереводчик/Module.bsl"
BSP_META_ENUM_FIXTURE = "bsp/meta/Enums/ВажностьПроблемыУчета.xml"
BSP_META_INFORMATION_REGISTER_FIXTURE = "bsp/meta/InformationRegisters/АдминистративнаяИерархия.xml"
BSP_SUBSYSTEM_FIXTURE = "bsp/subsystems/Администрирование.xml"
BSP_SUBSYSTEM_COMMAND_INTERFACE_FIXTURE = "bsp/subsystems/Администрирование/Ext/CommandInterface.xml"
BSP_FORM_BUSINESS_PROCESS_FIXTURE = (
    "bsp/forms/BusinessProcesses__Задание__ФормаБизнесПроцесса/Form.xml"
)
BSP_ROLE_ADMIN_RIGHTS_FIXTURE = "bsp/roles/АдминистраторСистемы/Rights.xml"
BSP_ROLE_ADMINISTRATION_RIGHTS_FIXTURE = "bsp/roles/Администрирование/Rights.xml"
BSP_MXL_RECEIPT_FIXTURE = (
    "bsp/mxl/Catalogs__МашиночитаемыеДоверенности__"
    "ПФ_MXL_Квитанция/Template.xml"
)
BSP_MXL_POWER_OF_ATTORNEY_FIXTURE = (
    "bsp/mxl/Catalogs__МашиночитаемыеДоверенности__"
    "ПФ_MXL_Доверенность/Template.xml"
)
READER_STANDINS_ROOT = FIXTURES_ROOT / "reader-standins"
DOCUMENTED_READER_TOOL_NAMES = {
    "unica.cf.info",
    "unica.cfe.diff",
    "unica.cfe.validate",
    "unica.code.definition",
    "unica.code.graph",
    "unica.code.search",
    "unica.dcs.info",
    "unica.documentation.get",
    "unica.documentation.search",
    "unica.form.info",
    "unica.interface.validate",
    "unica.meta.info",
    "unica.mxl.decompile",
    "unica.mxl.info",
    "unica.role.info",
    "unica.subsystem.info",
    "unica.xdto.info",
}
TYPED_CONTRACT_TOOL_NAMES = {
    name
    for name, review in json.loads(
        (REPO_ROOT / "arch/tool-surface-review.json").read_text(encoding="utf-8")
    ).items()
    if review["result"]["contract"] == "typed"
}


@dataclasses.dataclass(frozen=True)
class SetupStep:
    skill: str
    script: str
    arguments: dict[str, Any]
    tool: str | None = None
    stdout_path: str | None = None


@dataclasses.dataclass(frozen=True)
class FileFixture:
    source: str
    target: str


@dataclasses.dataclass(frozen=True)
class ParityScenario:
    name: str
    tool: str
    skill: str
    script: str
    arguments: dict[str, Any]
    expect_ok: bool
    fixtures: tuple[FileFixture, ...] = ()
    setup_steps: tuple[SetupStep, ...] = ()
    compare_files: bool = False
    # A migrated tool selects its target logically while the reference model
    # still selects a file. Parity then compares the analysis, not the
    # selector: both sides must describe the same object identically.
    reference_arguments: dict[str, Any] | None = None

    @property
    def script_arguments(self) -> dict[str, Any]:
        return self.arguments if self.reference_arguments is None else self.reference_arguments


@dataclasses.dataclass(frozen=True)
class SkillMcpExample:
    skill: str
    document: str
    line: int
    payload: dict[str, Any]


@dataclasses.dataclass(frozen=True)
class CcSkillCase:
    case_id: str
    skill_dir: str
    case_path: Path
    skill_config: dict[str, Any]
    case_data: dict[str, Any]


SUCCESS_SCENARIOS = [
    ParityScenario(
        name="cfe-validate-detailed-outfile",
        tool="unica.cfe.validate",
        skill="cfe-validate",
        script="cfe-validate.py",
        arguments={
            "ExtensionPath": "src-cfe/Configuration.xml",
            "Detailed": True,
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                },
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="form-compile-simple",
        tool="unica.form.compile",
        skill="form-compile",
        script="form-compile.py",
        arguments={
            "JsonPath": "fixtures/form-simple.json",
            "OutputPath": "forms/Form.xml",
        },
        fixtures=(FileFixture("form-simple.json", "fixtures/form-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-form-compile-catalog-list-from-object",
        tool="unica.form.compile",
        skill="form-compile",
        script="form-compile.py",
        arguments={
            "FromObject": True,
            "ObjectPath": "src/Catalogs/Валюты.xml",
            "Purpose": "List",
            "OutputPath": "src/Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml",
        },
        fixtures=(
            FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-form-compile-catalog-item-from-object",
        tool="unica.form.compile",
        skill="form-compile",
        script="form-compile.py",
        arguments={
            "FromObject": True,
            "ObjectPath": "src/Catalogs/Валюты.xml",
            "Purpose": "Item",
            "OutputPath": "src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml",
        },
        fixtures=(
            FileFixture(
                BSP_META_CATALOG_FIXTURE,
                "src/Catalogs/Валюты.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-form-compile-document-list-from-object",
        tool="unica.form.compile",
        skill="form-compile",
        script="form-compile.py",
        arguments={
            "FromObject": True,
            "ObjectPath": "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            "Purpose": "List",
            "OutputPath": (
                "src/Documents/АктОбУничтоженииПерсональныхДанных/"
                "Forms/ФормаСписка/Ext/Form.xml"
            ),
        },
        fixtures=(
            FileFixture(
                BSP_META_DOCUMENT_FIXTURE,
                "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-form-compile-document-item-from-object",
        tool="unica.form.compile",
        skill="form-compile",
        script="form-compile.py",
        arguments={
            "FromObject": True,
            "ObjectPath": "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            "Purpose": "Item",
            "OutputPath": (
                "src/Documents/АктОбУничтоженииПерсональныхДанных/"
                "Forms/ФормаДокумента/Ext/Form.xml"
            ),
        },
        fixtures=(
            FileFixture(
                BSP_META_DOCUMENT_FIXTURE,
                "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="subsystem-compile-basic",
        tool="unica.subsystem.compile",
        skill="subsystem-compile",
        script="subsystem-compile.py",
        arguments={
            "DefinitionFile": "fixtures/subsystem-sales.json",
            "OutputDir": "src/Subsystems",
            "NoValidate": True,
        },
        fixtures=(FileFixture("subsystem-sales.json", "fixtures/subsystem-sales.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="interface-validate-detailed",
        tool="unica.interface.validate",
        skill="interface-validate",
        script="interface-validate.py",
        arguments={
            "CIPath": "src/Subsystems/Sales/Ext/CommandInterface.xml",
            "Detailed": True,
        },
        fixtures=(
            FileFixture(
                "interface-validate/Sales/Ext/CommandInterface.xml",
                "src/Subsystems/Sales/Ext/CommandInterface.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-interface-validate-real-command-interface",
        tool="unica.interface.validate",
        skill="interface-validate",
        script="interface-validate.py",
        arguments={
            "CIPath": "src/Subsystems/Администрирование/Ext/CommandInterface.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(
            FileFixture(
                BSP_SUBSYSTEM_FIXTURE,
                "src/Subsystems/Администрирование.xml",
            ),
            FileFixture(
                BSP_SUBSYSTEM_COMMAND_INTERFACE_FIXTURE,
                "src/Subsystems/Администрирование/Ext/CommandInterface.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="dcs-compile-simple",
        tool="unica.dcs.compile",
        skill="dcs-compile",
        script="dcs-compile.py",
        arguments={
            "DefinitionFile": "fixtures/dcs-simple.json",
            "OutputPath": "templates/DCS.xml",
        },
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-compile-bsp-data-usage",
        tool="unica.dcs.compile",
        skill="dcs-compile",
        script="dcs-compile.py",
        arguments={
            "DefinitionFile": "fixtures/dcs-bsp-data-usage.json",
            "OutputPath": "templates/DCS.xml",
        },
        fixtures=(FileFixture("dcs-bsp-data-usage.json", "fixtures/dcs-bsp-data-usage.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="mxl-compile-simple",
        tool="unica.mxl.compile",
        skill="mxl-compile",
        script="mxl-compile.py",
        arguments={
            "JsonPath": "fixtures/mxl-simple.json",
            "OutputPath": "templates/MXL.xml",
        },
        fixtures=(FileFixture("mxl-simple.json", "fixtures/mxl-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="mxl-decompile-simple-stdout",
        tool="unica.mxl.decompile",
        skill="mxl-decompile",
        script="mxl-decompile.py",
        arguments={
            "TemplatePath": "templates/MXL.xml",
        },
        setup_steps=(
            SetupStep(
                skill="mxl-compile",
                script="mxl-compile.py",
                arguments={
                    "JsonPath": "fixtures/mxl-simple.json",
                    "OutputPath": "templates/MXL.xml",
                },
            ),
        ),
        fixtures=(FileFixture("mxl-simple.json", "fixtures/mxl-simple.json"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-mxl-decompile-real-template-stdout",
        tool="unica.mxl.decompile",
        skill="mxl-decompile",
        script="mxl-decompile.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
        },
        fixtures=(
            FileFixture(
                BSP_MXL_RECEIPT_FIXTURE,
                "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-mxl-parity-roundtrip-real-template",
        tool="unica.mxl.compile",
        skill="mxl-compile",
        script="mxl-compile.py",
        arguments={
            "JsonPath": "mxl-bsp.json",
            "OutputPath": "roundtrip/Template.xml",
        },
        setup_steps=(
            SetupStep(
                skill="mxl-decompile",
                script="mxl-decompile.py",
                tool="unica.mxl.decompile",
                arguments={
                    "TemplatePath": "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
                },
                stdout_path="mxl-bsp.json",
            ),
        ),
        fixtures=(
            FileFixture(
                BSP_MXL_RECEIPT_FIXTURE,
                "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="role-compile-reader",
        tool="unica.role.compile",
        skill="role-compile",
        script="role-compile.py",
        arguments={"JsonPath": "fixtures/role-reader.json", "OutputDir": "src/Roles"},
        fixtures=(FileFixture("role-reader.json", "fixtures/role-reader.json"),),
        expect_ok=True,
        compare_files=True,
    ),
]


VALIDATION_FAILURE_SCENARIOS = [
]


MISSING_INPUT_SCENARIOS = [
    ParityScenario(
        "cfe-validate-missing-extension",
        "unica.cfe.validate",
        "cfe-validate",
        "cfe-validate.py",
        {"ExtensionPath": "missing-extension"},
        False,
    ),
    # `unica.meta.info` has no missing-input scenario: the reference model fails
    # on a missing file, the tool fails on an address it cannot prove, and those
    # are different contracts by construction. The typed refusal is covered by
    # `meta_info_reports_an_unknown_address_without_naming_a_path`.
    ParityScenario(
        "interface-validate-missing-command-interface",
        "unica.interface.validate",
        "interface-validate",
        "interface-validate.py",
        {"CIPath": "missing/CommandInterface.xml"},
        False,
    ),
    ParityScenario(
        "mxl-decompile-missing-template",
        "unica.mxl.decompile",
        "mxl-decompile",
        "mxl-decompile.py",
        {"TemplatePath": "missing/Template.xml"},
        False,
    ),
]

SCENARIOS = tuple(
    SUCCESS_SCENARIOS + VALIDATION_FAILURE_SCENARIOS + MISSING_INPUT_SCENARIOS
)
MIN_NATIVE_PARITY_COVERAGE = 1.0

NATIVE_PARITY_TOOLS = {
    "unica.cfe.validate",
    "unica.form.compile",
    "unica.subsystem.compile",
    "unica.interface.validate",
    "unica.dcs.compile",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.role.compile",
}

MUTATING_FORM_DCS_PARITY_TOOLS = {
    "unica.form.compile",
    "unica.dcs.compile",
}

# A tool that answers with typed data has no prose to compare against the
# reference model, so it leaves this stand as it migrates (ADR-0023). The stand
# itself is scheduled for redesign; until then this list records what left and
# why, instead of scenarios quietly disappearing.
TYPED_RESULT_TOOLS = {
    "unica.cf.edit",
    "unica.cf.info",
    "unica.cf.init",
    "unica.cfe.borrow",
    "unica.cfe.diff",
    "unica.cfe.init",
    "unica.cfe.patch_method",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.form.edit",
    "unica.form.info",
    "unica.form.remove",
    "unica.interface.edit",
    "unica.meta.edit",
    "unica.meta.info",
    "unica.meta.remove",
    "unica.meta.add",
    "unica.mxl.info",
    "unica.role.info",
    "unica.role.edit",
    "unica.subsystem.edit",
    "unica.subsystem.info",
}

EXPECTED_TOOLS = {
    "unica.cfe.validate",
    "unica.form.compile",
    "unica.interface.validate",
    "unica.subsystem.compile",
    "unica.dcs.compile",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.role.compile",
}

BSP_PARITY_REQUIRED_TOOLS = {
    "unica.mxl.decompile",
    "unica.mxl.compile",
    "unica.interface.validate",
}

BSP_MUTATING_REQUIRED_TOOLS = {
    "unica.mxl.compile",
}

DCS_EDIT_REQUIRED_OPS = {
    "add-field",
    "add-total",
    "add-calculated-field",
    "add-parameter",
    "add-filter",
    "add-dataParameter",
    "add-order",
    "add-selection",
    "add-dataSetLink",
    "add-dataSet",
    "add-variant",
    "add-conditionalAppearance",
    "add-drilldown",
    "set-outputParameter",
    "set-query",
    "patch-query",
    "set-structure",
    "modify-field",
    "modify-filter",
    "modify-dataParameter",
    "modify-parameter",
    "modify-structure",
    "set-field-role",
    "rename-parameter",
    "reorder-parameters",
    "clear-selection",
    "clear-order",
    "clear-filter",
    "clear-conditionalAppearance",
    "remove-field",
    "remove-total",
    "remove-calculated-field",
    "remove-parameter",
    "remove-filter",
}

UUID_RE = re.compile(
    r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"
)


MCP_HANDSHAKE_ID = "unica-ci-handshake"
MCP_HANDSHAKE = [
    {
        "jsonrpc": "2.0",
        "id": MCP_HANDSHAKE_ID,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "unica-ci", "version": "1"},
        },
    },
    {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
]


class UnicaMcpScriptParityTests(unittest.TestCase):
    unica_bin: Path
    execution_by_tool: dict[str, str] | None = None

    @classmethod
    def setUpClass(cls) -> None:
        subprocess.run(
            ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica"],
            cwd=REPO_ROOT,
            check=True,
        )
        target_root = Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))
        suffix = ".exe" if os.name == "nt" else ""
        cls.unica_bin = target_root / "debug" / f"unica{suffix}"
        if not cls.unica_bin.is_file():
            raise AssertionError(f"built unica binary not found: {cls.unica_bin}")

    def test_every_in_scope_tool_has_a_parity_scenario(self) -> None:
        covered = {scenario.tool for scenario in SCENARIOS}
        self.assertEqual(covered, EXPECTED_TOOLS)
        # A migrated tool must be gone from the stand, not merely unscheduled:
        # a leftover scenario would compare a stdout that no longer exists.
        self.assertEqual(covered & TYPED_RESULT_TOOLS, set())
        self.assertEqual(NATIVE_PARITY_TOOLS & TYPED_RESULT_TOOLS, set())
        covered_by_success_snapshot = {
            scenario.tool
            for scenario in SCENARIOS
            if scenario.expect_ok and scenario.compare_files
        }
        self.assertEqual(
            covered_by_success_snapshot & MUTATING_FORM_DCS_PARITY_TOOLS,
            MUTATING_FORM_DCS_PARITY_TOOLS,
        )

    def test_native_parity_coverage_stays_above_required_threshold(self) -> None:
        covered = {scenario.tool for scenario in SCENARIOS if scenario.tool in NATIVE_PARITY_TOOLS}
        coverage = len(covered) / len(NATIVE_PARITY_TOOLS)
        self.assertGreaterEqual(coverage, MIN_NATIVE_PARITY_COVERAGE)
        self.assertEqual(NATIVE_PARITY_TOOLS - covered, set())

    # The v1 interception contract (Before/After only, never a function) is
    # asserted directly against the tool by the Rust test
    # `cfe_patch_method_rejects_unsupported_v1_interception_shapes_atomically`.
    # This guard checked the retired parity scenarios' arguments instead, so it
    # left the stand with unica.cfe.patch_method (ADR-0023).

    def test_rust_registry_parity_list_matches_python_parity_harness(self) -> None:
        app_mod = (REPO_ROOT / "crates" / "unica-coder" / "src" / "application" / "mod.rs").read_text(
            encoding="utf-8"
        )
        match = re.search(
            r"const PARITY_COVERED_TOOLS: &\[&str\] = &\[(.*?)\];",
            app_mod,
            flags=re.S,
        )
        self.assertIsNotNone(match)
        rust_tools = set(re.findall(r'"(unica\.[^"]+)"', match.group(1)))
        self.assertEqual(rust_tools, NATIVE_PARITY_TOOLS)

    def test_bsp_manifest_fixtures_are_exercised_by_parity_scenarios(self) -> None:
        manifest = json.loads((FIXTURES_ROOT / "bsp" / "manifest.json").read_text(encoding="utf-8"))
        manifest_sources = {f"bsp/{entry['target']}" for entry in manifest["files"]}
        retired_meta_sources = {
            f"bsp/{entry['target']}"
            for entry in manifest["files"]
            if entry["category"] == "meta"
        }
        used_sources = {fixture.source for scenario in SCENARIOS for fixture in scenario.fixtures}
        # The retired v0.12 stand keeps its BSP fixtures until the stand itself
        # is retired; scenarios must not name a fixture the manifest lost.
        self.assertEqual(used_sources - manifest_sources - {s for s in used_sources if not s.startswith("bsp/")}, set())
        self.assertTrue(retired_meta_sources <= manifest_sources)

    def test_language_aware_fixture_proves_list_presentation_precedence(self) -> None:
        fixture = (
            FIXTURES_ROOT
            / "meta-validate-language-aware"
            / "Enums"
            / "LanguageAware.xml"
        )
        root = ET.parse(fixture).getroot()
        namespaces = {
            "md": "http://v8.1c.ru/8.3/MDClasses",
            "v8": "http://v8.1c.ru/8.1/data/core",
        }

        def russian_text(property_name: str) -> str:
            item = root.find(
                f".//md:{property_name}/v8:item[v8:lang='ru']/v8:content",
                namespaces,
            )
            self.assertIsNotNone(item, f"missing Russian {property_name}")
            return item.text or ""

        self.assertGreater(len(russian_text("Synonym")), 38)
        self.assertLessEqual(len(russian_text("ListPresentation")), 38)

    def test_bsp_fixture_parity_covers_real_world_read_and_edit_tools(self) -> None:
        for tool in sorted(BSP_PARITY_REQUIRED_TOOLS):
            with self.subTest(tool=tool):
                scenarios = [
                    scenario
                    for scenario in SCENARIOS
                    if scenario.name.startswith("bsp-")
                    and scenario.tool == tool
                    and scenario.expect_ok
                ]
                self.assertGreater(len(scenarios), 0)
                if tool in BSP_MUTATING_REQUIRED_TOOLS:
                    self.assertTrue(any(scenario.compare_files for scenario in scenarios))

    def test_every_documented_dcs_edit_operation_stays_under_test(self) -> None:
        # unica.dcs.edit left the scenario stand for typed data (ADR-0023), so
        # the "no documented operation goes untested" guard now points at the
        # tests that live with the tool instead of at retired scenarios.
        dcs_rs = (
            REPO_ROOT
            / "crates"
            / "unica-coder"
            / "src"
            / "infrastructure"
            / "native_operations"
            / "dcs.rs"
        ).read_text(encoding="utf-8")
        marker = "mod tests"
        self.assertIn(marker, dcs_rs)
        tests_source = dcs_rs[dcs_rs.index(marker) :]
        untested = {
            operation
            for operation in DCS_EDIT_REQUIRED_OPS
            if f'"{operation}"' not in tests_source
        }
        self.assertEqual(untested, set())

    def test_event_subscription_fixture_exports_its_declared_handler(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-event-handler-fixture-") as temp:
            source_root = Path(temp) / "src" / "cf"
            source_root.mkdir(parents=True)
            (source_root / "Configuration.xml").write_text(
                """<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<Configuration>
		<ChildObjects>
		</ChildObjects>
	</Configuration>
</MetaDataObject>
""",
                encoding="utf-8",
            )
            prepare_meta_add_skill_example(
                {"main": source_root},
                {"sourceSet": "main", "kind": "EventSubscription"},
            )
            subscription = source_root / "EventSubscriptions" / "Events.xml"
            subscription.parent.mkdir(parents=True)
            write_meta_event_subscription_fixture(subscription, "Events")

            namespace = {"md": "http://v8.1c.ru/8.3/MDClasses"}
            handler = ET.parse(subscription).findtext(".//md:Handler", namespaces=namespace)
            self.assertIsNotNone(handler)
            prefix, module_name, procedure_name = handler.split(".")
            self.assertEqual(prefix, "CommonModule")
            module = (
                source_root / "CommonModules" / module_name / "Ext" / "Module.bsl"
            ).read_text(encoding="utf-8")
            self.assertRegex(
                module,
                rf"(?im)^\s*(?:Procedure|Процедура)\s+{re.escape(procedure_name)}"
                rf"\s*\([^)]*\)\s+(?:Export|Экспорт)\s*$",
            )

    def test_every_donor_case_has_one_reviewed_relation(self) -> None:
        cases = {case.case_id for case in iter_cc_1c_skill_cases()}
        relations = load_donor_relations()
        active_relations = {
            case_id
            for case_id in relations
            if case_id.partition("/")[0] in CC_CASE_TOOLS
        }
        self.assertEqual(active_relations, cases)
        retired_meta_relations = {
            case_id for case_id in relations if case_id.startswith("meta-compile/")
        }
        self.assertTrue(retired_meta_relations)
        self.assertEqual(retired_meta_relations & cases, set())

    def test_retired_donor_cases_are_not_compared(self) -> None:
        # A retired case keeps its files in the snapshot but leaves the
        # comparison, so it must not come back through the case iterator.
        retired = set(load_donor_registry().get("retired", {}))
        self.assertTrue(retired, "the retirement list records what left the stand")
        cases = {case.case_id for case in iter_cc_1c_skill_cases()}
        self.assertEqual(retired & cases, set())

    def test_donor_snapshot_integrity_and_provenance(self) -> None:
        errors = donor_contract.validate_repository_contract(REPO_ROOT)
        self.assertEqual(errors, [])

    def test_category_only_expected_gap_allowlist_is_removed(self) -> None:
        legacy_name = "CC_1C_" + "EXPECTED_GAPS"
        self.assertNotIn(
            legacy_name,
            Path(__file__).read_text(encoding="utf-8"),
        )

    def test_donor_cases_match_reviewed_relations(self) -> None:
        for case in iter_cc_1c_skill_cases():
            with self.subTest(case=case.case_id, tool=cc_case_tool(case)):
                self.assert_cc_1c_case_parity(case)

    def test_donor_inventory_relations_preview_and_snapshot_are_closed(self) -> None:
        self.test_every_donor_case_has_one_reviewed_relation()
        self.test_retired_donor_cases_are_not_compared()
        self.test_donor_snapshot_integrity_and_provenance()
        self.test_donor_cases_match_reviewed_relations()

    def assert_parity(self, scenario: ParityScenario) -> None:
        with tempfile.TemporaryDirectory(prefix=f"unica-parity-{scenario.name}-") as temp:
            temp_root = Path(temp)
            direct_ws = temp_root / "direct"
            mcp_ws = temp_root / "mcp"
            direct_ws.mkdir()
            mcp_ws.mkdir()
            mcp_cache = temp_root / "mcp-cache"
            self.prepare_workspace(direct_ws, scenario, setup_mode="reference")
            self.prepare_workspace(mcp_ws, scenario, setup_mode="mcp", cache_dir=mcp_cache)

            direct = run_unica_reference_model(
                scenario.skill, scenario.script, scenario.script_arguments, direct_ws
            )
            mcp = self.call_mcp(scenario, mcp_ws, mcp_cache)

            direct_ok = direct.returncode == 0
            self.assertEqual(direct_ok, scenario.expect_ok, direct.stderr)
            self.assertEqual(mcp["ok"], scenario.expect_ok, json.dumps(mcp, ensure_ascii=False, indent=2))
            self.assertEqual(mcp["ok"], direct_ok)
            self.assertEqual(
                normalize_text(direct.stdout, direct_ws),
                normalize_text(mcp.get("stdout") or "", mcp_ws),
            )
            self.assertEqual(
                normalize_text(direct.stderr, direct_ws),
                normalize_text(mcp.get("stderr") or "", mcp_ws),
            )
            if mcp.get("command") is not None:
                self.assertEqual(
                    normalize_command(
                        command_for_script(
                            scenario.skill, scenario.script, scenario.script_arguments
                        ),
                        direct_ws,
                    ),
                    normalize_command(mcp["command"], mcp_ws),
                )
            if scenario.tool in NATIVE_PARITY_TOOLS:
                self.assertIsNone(mcp.get("command"), f"{scenario.tool} must not use script fallback")
            if not direct_ok:
                expected_error = normalize_text(direct.stderr.strip(), direct_ws)
                if expected_error:
                    actual_errors = [normalize_text(error, mcp_ws) for error in mcp.get("errors", [])]
                    self.assertIn(expected_error, actual_errors)
            if scenario.compare_files:
                self.assertEqual(snapshot_workspace(direct_ws), snapshot_workspace(mcp_ws))

    def assert_cc_1c_case_parity(self, case: CcSkillCase) -> None:
        observation, message = self.observe_cc_1c_case(case)
        relation = load_donor_relations()[case.case_id]
        errors = donor_contract.validate_relation_observation(
            relation=relation,
            content_digest=donor_contract.case_content_digest(
                DONOR_SNAPSHOT_ROOT, case.case_id
            ),
            observation=observation,
        )
        self.assertEqual(
            errors,
            [],
            f"{case.case_id}: {message}\n"
            + json.dumps(observation, ensure_ascii=False, indent=2),
        )

    def observe_cc_1c_case(
        self, case: CcSkillCase
    ) -> tuple[dict[str, Any], str]:
        with tempfile.TemporaryDirectory(prefix=f"unica-cc-parity-{case.skill_dir}-{case.case_path.stem}-") as temp:
            temp_root = Path(temp)
            direct_ws = temp_root / "direct"
            mcp_ws = temp_root / "mcp"
            direct_ws.mkdir()
            mcp_ws.mkdir()
            mcp_cache = temp_root / "mcp-cache"

            self.prepare_cc_1c_workspace(direct_ws, case)
            self.prepare_cc_1c_workspace(mcp_ws, case)

            direct_args, direct_input = cc_case_main_arguments(case, direct_ws)
            mcp_args, mcp_input = cc_case_main_arguments(case, mcp_ws)
            try:
                direct = run_cc_python_script(cc_case_skill(case), cc_case_script(case), direct_args, direct_ws)
                mcp = self.call_mcp_tool(cc_case_tool(case), mcp_args, mcp_ws, mcp_cache)
            finally:
                if direct_input is not None:
                    direct_input.unlink(missing_ok=True)
                if mcp_input is not None:
                    mcp_input.unlink(missing_ok=True)

            expect_error = bool(case.case_data.get("expectError"))
            return cc_case_observation(
                case,
                direct,
                mcp,
                direct_ws,
                mcp_ws,
                expect_error,
            )

    def prepare_workspace(
        self,
        workspace: Path,
        scenario: ParityScenario,
        *,
        setup_mode: str,
        cache_dir: Path | None = None,
    ) -> None:
        for fixture in scenario.fixtures:
            target = workspace / fixture.target
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(FIXTURES_ROOT / fixture.source, target)
        for step in scenario.setup_steps:
            if setup_mode == "mcp" and step.tool is not None:
                if cache_dir is None:
                    raise AssertionError("cache_dir is required for MCP setup steps")
                mcp = self.call_mcp_tool(step.tool, step.arguments, workspace, cache_dir)
                self.assertTrue(mcp["ok"], json.dumps(mcp, ensure_ascii=False, indent=2))
                if step.tool in NATIVE_PARITY_TOOLS:
                    self.assertIsNone(mcp.get("command"), f"{step.tool} setup must not use script fallback")
                if step.stdout_path is not None:
                    target = workspace / step.stdout_path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(mcp.get("stdout") or "", encoding="utf-8")
            else:
                result = run_unica_reference_model(step.skill, step.script, step.arguments, workspace)
                if result.returncode != 0:
                    raise AssertionError(
                        f"setup step {step.skill}/{step.script} failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                    )
                if step.stdout_path is not None:
                    target = workspace / step.stdout_path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(result.stdout, encoding="utf-8")

    def prepare_cc_1c_workspace(self, workspace: Path, case: CcSkillCase) -> None:
        setup_name = case.case_data.get("setup") or case.skill_config.get("setup") or "none"
        if setup_name == "empty-config":
            result = run_cc_python_script("cf-init", "cf-init.py", {"Name": "TestConfig", "OutputDir": "."}, workspace)
            if result.returncode != 0:
                raise AssertionError(f"cc setup empty-config failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}")
            project_empty_config_to_8_3_27(workspace)
        elif isinstance(setup_name, str) and setup_name.startswith("fixture:"):
            fixture = case.case_path.parent / "fixtures" / setup_name.removeprefix("fixture:")
            if not fixture.exists():
                raise AssertionError(f"cc fixture not found: {fixture}")
            copy_tree_contents(fixture, workspace)
        elif setup_name not in ("none", None):
            raise AssertionError(f"unsupported cc setup: {setup_name}")

        for index, step in enumerate(case.case_data.get("preRun") or []):
            if "writeFile" in step:
                write_file = step["writeFile"]
                target = workspace / project_cc_case_path(
                    case.skill_dir,
                    write_file["path"],
                )
                target.parent.mkdir(parents=True, exist_ok=True)
                content = write_file.get("content", "")
                if not isinstance(content, str):
                    content = json.dumps(content, ensure_ascii=False, indent=2)
                target.write_text(content, encoding="utf-8")
                continue

            script_rel = step["script"]
            pre_input = None
            if "input" in step:
                pre_input = workspace / f"__cc_pre_input_{index}.json"
                pre_input.write_text(json.dumps(step["input"], ensure_ascii=False, indent=2), encoding="utf-8")
            args = cc_step_raw_args(
                step.get("args") or {},
                workspace,
                pre_input,
                case.skill_dir,
            )
            try:
                result = run_donor_skill_raw(script_rel, args, workspace)
            finally:
                if pre_input is not None:
                    pre_input.unlink(missing_ok=True)
            if result.returncode != 0:
                raise AssertionError(
                    f"cc preRun step {script_rel} failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                )

    def call_mcp(self, scenario: ParityScenario, workspace: Path, cache_dir: Path) -> dict[str, Any]:
        return self.call_mcp_tool(scenario.tool, scenario.arguments, workspace, cache_dir)

    def call_mcp_tool(
        self,
        tool: str,
        arguments: dict[str, Any],
        workspace: Path,
        cache_dir: Path,
    ) -> dict[str, Any]:
        arguments = dict(arguments)
        arguments["cwd"] = str(workspace)
        if type(self).execution_by_tool is None:
            listed = self.call_mcp_messages(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/list",
                        "params": {},
                    }
                ],
                cache_dir,
                process_cwd=workspace,
            )
            type(self).execution_by_tool = {
                entry["name"]: (
                    "mutation"
                    if "dryRun" in entry["inputSchema"]["properties"]
                    else "read"
                )
                for entry in listed[1]["result"]["tools"]
            }
        if type(self).execution_by_tool[tool] == "mutation":
            arguments["dryRun"] = False
        else:
            arguments.pop("dryRun", None)
        message = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        }
        env = os.environ.copy()
        env["UNICA_PLUGIN_ROOT"] = str(PLUGIN_ROOT)
        env["UNICA_CACHE_DIR"] = str(cache_dir)
        responses = self.run_mcp_messages([message], env)
        self.assertEqual(len(responses), 1, responses)
        response = responses[0]
        if "error" in response:
            raise AssertionError(json.dumps(response["error"], ensure_ascii=False, indent=2))
        return json.loads(response["result"]["content"][0]["text"])

    def tool_input_schemas(self) -> dict[str, dict[str, Any]]:
        cached = type(self)._input_schemas
        if cached is not None:
            return cached
        with tempfile.TemporaryDirectory(prefix="unica-tool-schemas-") as temp:
            responses = self.call_mcp_messages(
                [
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/list",
                        "params": {},
                    }
                ],
                Path(temp) / "cache",
            )
        cached = {
            tool["name"]: tool["inputSchema"]
            for tool in responses[1]["result"]["tools"]
        }
        type(self)._input_schemas = cached
        return cached

    def run_mcp_messages(
        self,
        messages: list[dict[str, Any]],
        env: dict[str, str],
        process_cwd: Path = REPO_ROOT,
        setup: (
            Callable[
                [Callable[[dict[str, Any]], dict[str, Any]]],
                None,
            ]
            | None
        ) = None,
    ) -> list[dict[str, Any]]:
        process = subprocess.Popen(
            [str(self.unica_bin)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            cwd=process_cwd,
            env=env,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        lines: queue.Queue[str] = queue.Queue()

        def read_stdout() -> None:
            while True:
                line = process.stdout.readline()
                lines.put(line)
                if not line:
                    return

        threading.Thread(target=read_stdout, daemon=True).start()
        # A batch is consumed by one stdio server. Reader examples now execute
        # their real bounded work instead of abusing writer-only dryRun, so the
        # transport budget must cover every public five-second deadline in the
        # batch rather than treating 32 requests as one invocation.
        deadline = time.monotonic() + max(30, len(messages) * 5)
        def read_response() -> dict[str, Any]:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.fail("timed out waiting for MCP response")
            try:
                line = lines.get(timeout=remaining)
            except queue.Empty:
                self.fail("timed out waiting for MCP response")
            if not line:
                self.fail("MCP process exited before all responses arrived")
            return json.loads(line)

        try:
            # The rmcp-based server requires the MCP handshake before requests;
            # perform it unless the scenario drives initialize itself, and wait
            # for the initialize acknowledgement before sending anything else.
            if not messages or messages[0].get("method") != "initialize":
                process.stdin.write(
                    json.dumps(MCP_HANDSHAKE[0], ensure_ascii=False) + "\n"
                )
                process.stdin.flush()
                handshake_response = read_response()
                self.assertEqual(
                    handshake_response.get("id"), MCP_HANDSHAKE_ID, handshake_response
                )
                self.assertEqual(
                    handshake_response["result"]["serverInfo"]["name"], "unica"
                )
                process.stdin.write(
                    json.dumps(MCP_HANDSHAKE[1], ensure_ascii=False) + "\n"
                )
            if setup is not None:
                process.stdin.flush()

                def request_one(message: dict[str, Any]) -> dict[str, Any]:
                    process.stdin.write(
                        json.dumps(message, ensure_ascii=False) + "\n"
                    )
                    process.stdin.flush()
                    return read_response()

                setup(request_one)
            for message in messages:
                process.stdin.write(json.dumps(message, ensure_ascii=False) + "\n")
            process.stdin.flush()

            expected = sum("id" in message for message in messages)
            responses = [read_response() for _ in range(expected)]

            process.stdin.close()
            return_code = process.wait(timeout=max(0.1, deadline - time.monotonic()))
            stderr = process.stderr.read()
            self.assertEqual(return_code, 0, stderr)
            return responses
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            process.stdout.close()
            process.stderr.close()

    def call_mcp_messages(
        self,
        messages: list[dict[str, Any]],
        cache_dir: Path,
        process_cwd: Path = REPO_ROOT,
        extra_env: dict[str, str] | None = None,
    ) -> dict[int, dict[str, Any]]:
        env = os.environ.copy()
        env["UNICA_PLUGIN_ROOT"] = str(PLUGIN_ROOT)
        env["UNICA_CACHE_DIR"] = str(cache_dir)
        if extra_env is not None:
            env.update(extra_env)
        responses = []
        for start in range(0, len(messages), 32):
            batch = messages[start : start + 32]
            responses.extend(self.run_mcp_messages(batch, env, process_cwd=process_cwd))
        return {response["id"]: response for response in responses}


def run_unica_reference_model(
    skill: str,
    script: str,
    arguments: dict[str, Any],
    workspace: Path,
    *,
    skills_root: Path = UNICA_REFERENCE_MODELS_ROOT,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command_for_script(skill, script, arguments, skills_root=skills_root),
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return decoded_completed_process(result)


def run_cc_python_script(
    skill: str,
    script: str,
    arguments: dict[str, Any],
    workspace: Path,
) -> subprocess.CompletedProcess[str]:
    return run_unica_reference_model(
        skill,
        script,
        arguments,
        workspace,
        skills_root=DONOR_SKILLS_ROOT,
    )


# Donor cases compare tool stdout against the cc-1c reference scripts. A tool
# that migrated to typed data (ADR-0023) has no prose left to compare, so it
# leaves this stand the same way it leaves the scenario stand. `cfe-borrow`
# left with unica.cfe.borrow; the donor snapshot itself is untouched.
CC_CASE_TOOLS = {
    "skd-compile": "unica.dcs.compile",
    "form-compile": "unica.form.compile",
    "form-compile-from-object": "unica.form.compile",
}



def iter_cc_1c_skill_cases() -> list[CcSkillCase]:
    if not CC_1C_CASES_ROOT.exists():
        return []
    cases: list[CcSkillCase] = []
    for skill_dir in sorted(CC_CASE_TOOLS):
        skill_root = CC_1C_CASES_ROOT / skill_dir
        skill_config_path = skill_root / "_skill.json"
        if not skill_config_path.exists():
            continue
        skill_config = json.loads(skill_config_path.read_text(encoding="utf-8"))
        for case_path in sorted(skill_root.glob("*.json")):
            if case_path.name.startswith("_"):
                continue
            case_data = json.loads(case_path.read_text(encoding="utf-8"))
            cases.append(
                CcSkillCase(
                    case_id=f"{skill_dir}/{case_path.stem}",
                    skill_dir=skill_dir,
                    case_path=case_path,
                    skill_config=skill_config,
                    case_data=case_data,
                )
            )
    return cases


def load_donor_registry() -> dict[str, Any]:
    return donor_contract.load_json(DONOR_RELATIONS_PATH)


def load_donor_relations() -> dict[str, dict[str, Any]]:
    registry = load_donor_registry()
    relations = registry.get("relations")
    if not isinstance(relations, dict):
        raise AssertionError("donor relation registry must contain an object")
    return relations


def write_donor_observation_candidates(output_path: Path) -> None:
    UnicaMcpScriptParityTests.setUpClass()
    test_case = UnicaMcpScriptParityTests(methodName="runTest")
    observations = {}
    cases = iter_cc_1c_skill_cases()
    for index, case in enumerate(cases, start=1):
        print(
            f"[{index}/{len(cases)}] {case.case_id}",
            file=sys.stderr,
            flush=True,
        )
        observation, message = test_case.observe_cc_1c_case(case)
        observations[case.case_id] = {
            "contentDigest": donor_contract.case_content_digest(
                DONOR_SNAPSHOT_ROOT, case.case_id
            ),
            "observation": observation,
            "observationFingerprint": donor_contract.observation_fingerprint(
                observation
            ),
            "message": message,
        }
    payload = {
        "schemaVersion": 1,
        "snapshotRoot": str(DONOR_SNAPSHOT_ROOT),
        "observations": observations,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def cc_case_tool(case: CcSkillCase) -> str:
    return CC_CASE_TOOLS[case.skill_dir]


def cc_case_skill(case: CcSkillCase) -> str:
    return cc_script_skill_and_script(case.skill_config["script"])[0]


def cc_case_script(case: CcSkillCase) -> str:
    return cc_script_skill_and_script(case.skill_config["script"])[1]


def cc_script_skill_and_script(script_rel: str) -> tuple[str, str]:
    parts = script_rel.split("/")
    if len(parts) != 3 or parts[1] != "scripts":
        raise AssertionError(f"unsupported cc script path: {script_rel}")
    return parts[0], f"{parts[2]}.py"


def cc_case_main_arguments(case: CcSkillCase, workspace: Path) -> tuple[dict[str, Any], Path | None]:
    input_file = None
    if "input" in case.case_data:
        input_file = workspace / "__cc_input.json"
        input_file.write_text(json.dumps(case.case_data["input"], ensure_ascii=False, indent=2), encoding="utf-8")

    arguments: dict[str, Any] = {}
    for mapping in case.skill_config["args"]:
        key = mapping["flag"].lstrip("-")
        value = cc_mapping_value(
            mapping,
            case.case_data,
            workspace,
            input_file,
            case.skill_dir,
        )
        if value is CC_OMIT:
            continue
        arguments[key] = value

    for key, value in cc_args_extra(
        case.case_data.get("args_extra") or [],
        workspace,
        case.skill_dir,
    ).items():
        arguments[key] = value
    return arguments, input_file


CC_OMIT = object()


def cc_mapping_value(
    mapping: dict[str, Any],
    case_data: dict[str, Any],
    workspace: Path,
    input_file: Path | None,
    case_scope: str,
) -> Any:
    source = mapping["from"]
    if source == "inputFile":
        if input_file is None:
            return CC_OMIT
        return input_file.as_posix()
    if source == "workDir":
        return "."
    if source == "outputPath":
        raw = project_cc_case_path(
            case_scope,
            case_data.get("outputPath") or "",
        )
        return cc_workspace_path(workspace, raw)
    if source == "workPath":
        field = mapping.get("field") or "objectPath"
        raw = case_data.get("params", {}).get(field, case_data.get(field))
        if raw in (None, ""):
            return CC_OMIT if mapping.get("optional") else "."
        raw = project_cc_case_path(case_scope, raw)
        return cc_workspace_path(workspace, raw)
    if source == "switch":
        return case_data.get(mapping["flag"].lstrip("-"), True) is not False
    if source == "literal":
        return mapping.get("value") or ""
    if source.startswith("case."):
        field = source.removeprefix("case.")
        return case_data.get("params", {}).get(field, case_data.get(field, ""))
    raise AssertionError(f"unsupported cc arg source: {source}")


def cc_workspace_path(workspace: Path, raw: str) -> str:
    return (workspace / raw).as_posix()


def project_cc_case_path(case_scope: str, raw: str) -> str:
    projections = donor_contract.CASE_EXECUTION_PATH_PROJECTIONS.get(
        case_scope,
        {},
    )
    for source, target in projections.items():
        for prefix in (source, f"{{workDir}}/{source}"):
            if raw == prefix:
                return target if prefix == source else f"{{workDir}}/{target}"
            if raw.startswith(f"{prefix}/"):
                replacement = (
                    target
                    if prefix == source
                    else f"{{workDir}}/{target}"
                )
                return replacement + raw[len(prefix) :]
    return raw


def cc_args_extra(
    args_extra: list[Any],
    workspace: Path,
    case_scope: str,
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    index = 0
    while index < len(args_extra):
        raw_flag = args_extra[index]
        if not isinstance(raw_flag, str) or not raw_flag.startswith("-"):
            raise AssertionError(f"unsupported cc args_extra item: {raw_flag!r}")
        key = raw_flag.lstrip("-")
        next_index = index + 1
        if next_index >= len(args_extra) or (
            isinstance(args_extra[next_index], str) and args_extra[next_index].startswith("-")
        ):
            result[key] = True
            index += 1
            continue
        value = args_extra[next_index]
        if isinstance(value, str):
            value = project_cc_case_path(case_scope, value)
            value = value.replace("{workDir}", workspace.as_posix())
        result[key] = value
        index += 2
    return result


def cc_step_raw_args(
    args_map: dict[str, Any],
    workspace: Path,
    input_file: Path | None,
    case_scope: str,
) -> list[str]:
    args: list[str] = []
    for flag, raw_value in args_map.items():
        args.append(flag)
        if raw_value is True or raw_value == "":
            continue
        value = project_cc_case_path(case_scope, str(raw_value))
        value = value.replace("{workDir}", workspace.as_posix())
        if input_file is not None:
            value = value.replace("{inputFile}", input_file.as_posix())
        args.append(value)
    return args


def run_donor_skill_raw(
    script_rel: str,
    args: list[str],
    workspace: Path,
) -> subprocess.CompletedProcess[str]:
    skill, script = cc_script_skill_and_script(script_rel)
    script_path = DONOR_SKILLS_ROOT / skill / "scripts" / script
    result = subprocess.run(
        ["python3", str(script_path), *args],
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return decoded_completed_process(result)


def decoded_completed_process(
    result: subprocess.CompletedProcess[bytes],
) -> subprocess.CompletedProcess[str]:
    def decode(data: bytes) -> str:
        if os.name == "nt":
            data = data.replace(b"\r\r\n", b"\r\n")
        return data.decode("utf-8")

    return subprocess.CompletedProcess(
        result.args,
        result.returncode,
        stdout=decode(result.stdout),
        stderr=decode(result.stderr),
    )


def cc_case_observation(
    case: CcSkillCase,
    direct: subprocess.CompletedProcess[str],
    mcp: dict[str, Any],
    direct_ws: Path,
    mcp_ws: Path,
    expect_error: bool,
) -> tuple[dict[str, Any], str]:
    mismatch_kind, message = _cc_case_parity_gap(
        case,
        direct,
        mcp,
        direct_ws,
        mcp_ws,
        expect_error,
    )
    expected_files = cc_case_expected_files(case)
    donor_snapshot = snapshot_workspace(direct_ws)
    unica_snapshot = snapshot_workspace(mcp_ws)
    observation = {
        "donorOk": direct.returncode == 0,
        "unicaOk": bool(mcp.get("ok")),
        "mismatchKind": mismatch_kind,
        "donorStdoutSha256": donor_contract.sha256_json(
            normalize_text(direct.stdout, direct_ws)
        ),
        "unicaStdoutSha256": donor_contract.sha256_json(
            normalize_text(mcp.get("stdout") or "", mcp_ws)
        ),
        "donorStderrSha256": donor_contract.sha256_json(
            normalize_text(direct.stderr, direct_ws)
        ),
        "unicaStderrSha256": donor_contract.sha256_json(
            normalize_text(mcp.get("stderr") or "", mcp_ws)
        ),
        "donorWorkspaceSha256": donor_contract.sha256_json(donor_snapshot),
        "unicaWorkspaceSha256": donor_contract.sha256_json(unica_snapshot),
        "donorExpectedFiles": {
            path: (direct_ws / path).exists() for path in expected_files
        },
        "unicaExpectedFiles": {
            path: (mcp_ws / path).exists() for path in expected_files
        },
    }
    return observation, message


def _cc_case_parity_gap(
    case: CcSkillCase,
    direct: subprocess.CompletedProcess[str],
    mcp: dict[str, Any],
    direct_ws: Path,
    mcp_ws: Path,
    expect_error: bool,
) -> tuple[str | None, str]:
    direct_ok = direct.returncode == 0
    if direct_ok != (not expect_error):
        return "donor_expect_mismatch", direct.stderr or direct.stdout

    if mcp.get("ok") != direct_ok:
        errors = mcp.get("errors") or []
        first_error = str(errors[0]) if errors else ""
        if "Unsupported form element" in first_error:
            category = "unsupported_form_element"
        elif "Object type" in first_error and "not supported" in first_error:
            category = "unsupported_from_object_type"
        elif "native meta compiler currently supports one metadata object per call" in first_error:
            category = "meta_batch_unsupported"
        else:
            category = "ok_mismatch"
        return category, json.dumps(mcp, ensure_ascii=False, indent=2)

    if mcp.get("command") is not None:
        return "script_fallback", f"{cc_case_tool(case)} must not use script fallback"

    direct_stdout = normalize_text(direct.stdout, direct_ws)
    mcp_stdout = normalize_text(mcp.get("stdout") or "", mcp_ws)
    if direct_stdout != mcp_stdout:
        snapshot_equal = direct_ok and snapshot_workspace(direct_ws) == snapshot_workspace(mcp_ws)
        category = "stdout_mismatch_snapshot_equal" if snapshot_equal else "stdout_mismatch_snapshot_diff"
        return category, unified_text_message("stdout", direct_stdout, mcp_stdout)

    direct_stderr = normalize_text(direct.stderr, direct_ws)
    mcp_stderr = normalize_text(mcp.get("stderr") or "", mcp_ws)
    if direct_stderr != mcp_stderr:
        return "stderr_mismatch", unified_text_message("stderr", direct_stderr, mcp_stderr)

    if not direct_ok:
        expected_error = direct_stderr.strip()
        if expected_error:
            actual_errors = [normalize_text(error, mcp_ws) for error in mcp.get("errors", [])]
            if expected_error not in actual_errors:
                return "error_payload_mismatch", json.dumps(mcp, ensure_ascii=False, indent=2)
        return None, ""

    for rel_path in cc_case_expected_files(case):
        if not (direct_ws / rel_path).exists():
            return "missing_direct_expected_file", rel_path
        if not (mcp_ws / rel_path).exists():
            return "missing_mcp_expected_file", rel_path

    direct_snapshot = snapshot_workspace(direct_ws)
    mcp_snapshot = snapshot_workspace(mcp_ws)
    if direct_snapshot != mcp_snapshot:
        return "snapshot_diff", f"direct files: {len(direct_snapshot)}, mcp files: {len(mcp_snapshot)}"

    return None, ""


def unified_text_message(label: str, direct: str, mcp: str) -> str:
    return f"{label} differs\n--- direct\n{direct}\n--- mcp\n{mcp}"


def cc_case_expected_files(case: CcSkillCase) -> list[str]:
    files = case.case_data.get("expect", {}).get("files") or []
    return [str(path) for path in files]


def project_empty_config_to_8_3_27(workspace: Path) -> None:
    configuration = workspace / "Configuration.xml"
    data = configuration.read_bytes()
    marker = b'version="2.17"'
    if marker not in data:
        raise AssertionError(
            "donor empty-config fixture no longer uses the reviewed 2.17 format"
        )
    configuration.write_bytes(data.replace(marker, b'version="2.20"', 1))


def copy_tree_contents(source: Path, target: Path) -> None:
    for child in source.iterdir():
        destination = target / child.name
        if child.is_dir():
            if destination.exists():
                shutil.rmtree(destination)
            shutil.copytree(child, destination)
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(child, destination)


def command_for_script(
    skill: str,
    script: str,
    arguments: dict[str, Any],
    *,
    skills_root: Path = UNICA_REFERENCE_MODELS_ROOT,
) -> list[str]:
    script_path = skills_root / skill / "scripts" / script
    return ["python3", str(script_path), *script_args(arguments)]


def iter_documented_mcp_examples(documents: Iterable[Path]) -> list[SkillMcpExample]:
    examples: list[SkillMcpExample] = []
    for skill_doc in sorted(documents):
        text = skill_doc.read_text(encoding="utf-8")
        for match in re.finditer(r"```json\n(.*?)\n```", text, flags=re.S):
            block = match.group(1)
            if '"method": "tools/call"' not in block:
                continue
            payload = json.loads(block)
            if payload.get("method") != "tools/call":
                continue
            line = text.count("\n", 0, match.start()) + 1
            examples.append(
                SkillMcpExample(
                    skill=skill_doc.relative_to(SKILLS_ROOT).parts[0],
                    document=skill_doc.relative_to(REPO_ROOT).as_posix(),
                    line=line,
                    payload=payload,
                )
            )
    return examples


def iter_skill_mcp_examples() -> list[SkillMcpExample]:
    return iter_documented_mcp_examples(SKILLS_ROOT.glob("*/SKILL.md"))


def execution_message_for_example(
    example: SkillMcpExample,
    request_id: int,
    workspace: Path,
    execution_by_tool: dict[str, str],
) -> dict[str, Any]:
    message = json.loads(json.dumps(example.payload, ensure_ascii=False))
    message["id"] = request_id
    message["jsonrpc"] = "2.0"
    params = message.setdefault("params", {})
    arguments = params.setdefault("arguments", {})
    tool_name = params.get("name", "")
    if tool_name.startswith("unica.meta.") or tool_name == "unica.role.edit":
        arguments.pop("cwd", None)
    else:
        arguments["cwd"] = str(workspace)
    if execution_by_tool[tool_name] == "mutation":
        arguments["dryRun"] = True
    else:
        arguments.pop("dryRun", None)
    return message


def copy_reader_fixture(source: str, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(FIXTURES_ROOT / source, target)
    if target.suffix.lower() == ".xml":
        data = target.read_bytes()
        target.write_bytes(data.replace(b'version="2.17"', b'version="2.20"'))


def reader_content_path(raw_path: str, leaf: str) -> Path:
    path = Path(raw_path)
    return path if path.suffix.lower() == ".xml" else path / "Ext" / leaf


def replace_reader_placeholder(
    arguments: dict[str, Any],
    key: str,
    example: SkillMcpExample,
    replacement: str,
) -> str:
    value = str(arguments[key])
    if value.startswith("<") and value.endswith(">"):
        arguments[key] = replacement
        return replacement
    return value


def register_configuration_child(configuration: Path, kind: str, name: str) -> None:
    text = configuration.read_text(encoding="utf-8-sig")
    registration = f"\t\t\t<{kind}>{name}</{kind}>\n"
    if registration in text:
        return
    marker = "\t\t</ChildObjects>"
    if marker not in text:
        raise AssertionError(f"configuration fixture has no ChildObjects marker: {configuration}")
    configuration.write_text(
        text.replace(marker, registration + marker, 1),
        encoding="utf-8",
    )


def add_extension_internal_info(configuration: Path) -> None:
    text = configuration.read_text(encoding="utf-8-sig")
    if "<InternalInfo>" in text:
        return
    donor = (FIXTURES_ROOT / "cf-validate/Configuration.xml").read_text(encoding="utf-8")
    internal_info = re.search(r"\t\t<InternalInfo>.*?\t\t</InternalInfo>\n", donor, re.S)
    if internal_info is None:
        raise AssertionError("cf-validate fixture lost its InternalInfo donor block")
    marker = re.search(r"\t<Configuration[^>]*>\n", text)
    if marker is None:
        raise AssertionError(f"extension fixture has no Configuration root: {configuration}")
    configuration.write_text(
        text[: marker.end()] + internal_info.group(0) + text[marker.end() :],
        encoding="utf-8",
    )


def write_named_subsystem_fixture(target: Path, name: str, child: str | None = None) -> None:
    source = (FIXTURES_ROOT / BSP_SUBSYSTEM_FIXTURE).read_text(encoding="utf-8-sig")
    source = source.replace("Администрирование", name)
    if child is not None:
        source = source.replace("КонтрольРаботыПользователей", child)
    else:
        source = source.replace(
            "\t\t<ChildObjects>\n\t\t\t<Subsystem>КонтрольРаботыПользователей</Subsystem>\n\t\t</ChildObjects>\n",
            "\t\t<ChildObjects>\n\t\t</ChildObjects>\n",
        )
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(source, encoding="utf-8")
    copy_reader_fixture(
        BSP_SUBSYSTEM_COMMAND_INTERFACE_FIXTURE,
        target.with_suffix("") / "Ext" / "CommandInterface.xml",
    )


# ADR-0049: a bridged reader may select its target logically. Such an example
# carries no path to substitute, so the harness materialises a registered,
# addressable object instead and points the example at it.
LOGICAL_READER_TARGETS: dict[str, dict[str, Any]] = {
    "unica.role.info": {"address": "Role.ParityRole"},
    "unica.role.validate": {"address": "Role.ParityRole"},
    "unica.form.info": {"address": "Catalog.ParityCatalog.Form.ParityForm"},
    "unica.form.validate": {"address": "Catalog.ParityCatalog.Form.ParityForm"},
    "unica.dcs.info": {"address": "Report.ParityReport.Template.ParityDcs"},
    "unica.dcs.validate": {"address": "Report.ParityReport.Template.ParityDcs"},
    "unica.mxl.info": {"address": "Report.ParityReport.Template.ParityMxl"},
    "unica.mxl.validate": {"address": "Report.ParityReport.Template.ParityMxl"},
    "unica.mxl.decompile": {"address": "Report.ParityReport.Template.ParityMxl"},
    "unica.subsystem.info": {"address": "Subsystem.ParitySubsystem"},
    "unica.subsystem.validate": {"address": "Subsystem.ParitySubsystem"},
    "unica.cf.info": {"address": None},
    "unica.cf.validate": {"address": None},
}

# ADR-0054: path-form examples of support-aware readers must still resolve to
# registered logical owners. Keep the public selector form under test while
# routing the synthetic execution through the same objects as logical examples.
REGISTERED_SUPPORT_READER_PATHS: dict[str, tuple[str, str]] = {
    "unica.cf.info": ("ConfigPath", "src/cf/Configuration.xml"),
    "unica.dcs.info": (
        "TemplatePath",
        "src/cf/Reports/ParityReport/Templates/ParityDcs/Ext/Template.xml",
    ),
    "unica.form.info": (
        "FormPath",
        "src/cf/Catalogs/ParityCatalog/Forms/ParityForm/Ext/Form.xml",
    ),
    "unica.mxl.info": (
        "TemplatePath",
        "src/cf/Reports/ParityReport/Templates/ParityMxl/Ext/Template.xml",
    ),
    "unica.role.info": ("RightsPath", "src/cf/Roles/ParityRole/Ext/Rights.xml"),
}


def descriptor_image(kind: str, name: str, children: str = "") -> str:
    # A validator reads the descriptor, so the identity fields it checks — the
    # UUID and a non-empty synonym — have to be real, not omitted.
    # Derived, not random: `hash` is salted per process, and a fixture
    # that changes between runs is not a fixture.
    digest = hashlib.sha256(name.encode("utf-8")).hexdigest()
    uuid = f"{digest[:8]}-{digest[8:12]}-{digest[12:16]}-{digest[16:20]}-{digest[20:32]}"
    synonym = (
        "\t\t\t<Synonym>\n\t\t\t\t<v8:item>\n\t\t\t\t\t<v8:lang>ru</v8:lang>\n"
        f"\t\t\t\t\t<v8:content>{name}</v8:content>\n"
        "\t\t\t\t</v8:item>\n\t\t\t</Synonym>\n"
    )
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"'
        ' xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">\n'
        f'\t<{kind} uuid="{uuid}">\n\t\t<Properties>\n\t\t\t<Name>{name}</Name>\n'
        f"{synonym}\t\t</Properties>\n{children}\t</{kind}>\n</MetaDataObject>\n"
    )


def child_objects(kind: str, name: str) -> str:
    return f"\t\t<ChildObjects>\n\t\t\t<{kind}>{name}</{kind}>\n\t\t</ChildObjects>\n"


def materialise_logical_reader_target(source_root: Path) -> None:
    """Write the registered objects every logical reader example addresses."""
    configuration = source_root / "Configuration.xml"

    (source_root / "Roles").mkdir(parents=True, exist_ok=True)
    (source_root / "Roles" / "ParityRole.xml").write_text(
        descriptor_image("Role", "ParityRole"), encoding="utf-8"
    )
    copy_reader_fixture(
        BSP_ROLE_ADMIN_RIGHTS_FIXTURE,
        source_root / "Roles" / "ParityRole" / "Ext" / "Rights.xml",
    )
    register_configuration_child(configuration, "Role", "ParityRole")

    (source_root / "Catalogs").mkdir(parents=True, exist_ok=True)
    (source_root / "Catalogs" / "ParityCatalog.xml").write_text(
        descriptor_image("Catalog", "ParityCatalog", child_objects("Form", "ParityForm")),
        encoding="utf-8",
    )
    forms = source_root / "Catalogs" / "ParityCatalog" / "Forms"
    forms.mkdir(parents=True, exist_ok=True)
    (forms / "ParityForm.xml").write_text(
        descriptor_image("Form", "ParityForm"), encoding="utf-8"
    )
    copy_reader_fixture(
        BSP_FORM_BUSINESS_PROCESS_FIXTURE, forms / "ParityForm" / "Ext" / "Form.xml"
    )
    register_configuration_child(configuration, "Catalog", "ParityCatalog")

    (source_root / "Reports").mkdir(parents=True, exist_ok=True)
    (source_root / "Reports" / "ParityReport.xml").write_text(
        descriptor_image(
            "Report",
            "ParityReport",
            "\t\t<ChildObjects>\n\t\t\t<Template>ParityDcs</Template>\n"
            "\t\t\t<Template>ParityMxl</Template>\n\t\t</ChildObjects>\n",
        ),
        encoding="utf-8",
    )
    templates = source_root / "Reports" / "ParityReport" / "Templates"
    templates.mkdir(parents=True, exist_ok=True)
    for template, fixture in (
        ("ParityDcs", BSP_DCS_OBJECT_FIXTURE),
        ("ParityMxl", BSP_MXL_RECEIPT_FIXTURE),
    ):
        (templates / f"{template}.xml").write_text(
            descriptor_image("Template", template), encoding="utf-8"
        )
        copy_reader_fixture(fixture, templates / template / "Ext" / "Template.xml")
    register_configuration_child(configuration, "Report", "ParityReport")

    write_named_subsystem_fixture(
        source_root / "Subsystems" / "ParitySubsystem.xml", "ParitySubsystem"
    )
    register_configuration_child(configuration, "Subsystem", "ParitySubsystem")


def prepare_logical_reader_example(arguments: dict[str, Any], tool_name: str) -> None:
    arguments["sourceSet"] = "main"
    address = LOGICAL_READER_TARGETS[tool_name]["address"]
    if address is None:
        arguments.pop("metadataPath", None)
    else:
        arguments["metadataPath"] = address


def prepare_skill_reader_fixtures(
    examples: list[SkillMcpExample],
    execution_by_tool: dict[str, str],
    workspace: Path,
    source_roots: dict[str, Path],
) -> None:
    readers = {
        example.payload["params"]["name"]
        for example in examples
        if execution_by_tool[example.payload["params"]["name"]] == "read"
    }
    if readers != DOCUMENTED_READER_TOOL_NAMES:
        locations = {
            example.payload["params"]["name"]: f"{example.document}:{example.line}"
            for example in examples
            if execution_by_tool[example.payload["params"]["name"]] == "read"
        }
        raise AssertionError(
            f"documented reader fixture routing changed: readers={sorted(readers)}; "
            f"locations={locations}"
        )

    for relative in [
        "src/Configuration.xml",
        "test-tmp/cf/Configuration.xml",
        "upload/cfempty/Configuration.xml",
    ]:
        copy_reader_fixture("cf-validate/Configuration.xml", workspace / relative)
    for relative in ["src/cfe/Configuration.xml", "src/extensions/MyExtension/Configuration.xml"]:
        configuration = workspace / relative
        copy_reader_fixture("cfe-diff/mode-b/src-cfe/Configuration.xml", configuration)
        add_extension_internal_info(configuration)
    code_module = source_roots["main"] / "CommonModules" / "ParitySearch" / "Ext" / "Module.bsl"
    code_module.parent.mkdir(parents=True, exist_ok=True)
    code_module.write_text(
        """Procedure ОбработкаПроведения() Export
    СведенияОВнешнейОбработке = True;
    Запрос = "ВЫБРАТЬ 1";
    ВыполнитьОбменСКонтрагентом();
EndProcedure
""",
        encoding="utf-8",
    )
    (source_roots["main"] / "CommonModules" / "ParitySearch.xml").write_text(
        """<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <CommonModule><Properties><Name>ParitySearch</Name></Properties></CommonModule>
</MetaDataObject>
""",
        encoding="utf-8",
    )
    register_configuration_child(source_roots["main"] / "Configuration.xml", "CommonModule", "ParitySearch")

    interface_fixture = "interface-validate/Sales/Ext/CommandInterface.xml"
    role_rights_fixture = BSP_ROLE_ADMIN_RIGHTS_FIXTURE
    materialise_logical_reader_target(source_roots["main"])

    handled: set[str] = set()
    for example in examples:
        tool_name = example.payload["params"]["name"]
        if execution_by_tool[tool_name] != "read":
            continue
        handled.add(tool_name)
        arguments = example.payload["params"]["arguments"]

        # ADR-0049: a logical example carries a selector, not a path, so the
        # path substitution below has nothing to work on.
        if "sourceSet" in arguments and tool_name in LOGICAL_READER_TARGETS:
            prepare_logical_reader_example(arguments, tool_name)
            continue

        if tool_name in REGISTERED_SUPPORT_READER_PATHS:
            path_argument, registered_path = REGISTERED_SUPPORT_READER_PATHS[tool_name]
            arguments[path_argument] = registered_path
            continue

        if tool_name in {"unica.cf.info", "unica.cf.validate", "unica.cfe.diff"}:
            continue
        if tool_name == "unica.cfe.validate":
            extension_path = str(arguments["ExtensionPath"])
            if extension_path == "src":
                arguments["ExtensionPath"] = "src/cfe"
            elif extension_path == "src/Configuration.xml":
                arguments["ExtensionPath"] = "src/cfe/Configuration.xml"
            continue
        if tool_name == "unica.code.search":
            replace_reader_placeholder(arguments, "sourceSet", example, "main")
            continue
        if tool_name in {"unica.code.graph", "unica.code.definition"}:
            continue
        if tool_name in {"unica.meta.info", "unica.xdto.info"}:
            continue
        if tool_name in {"unica.dcs.info", "unica.dcs.validate"}:
            raw = replace_reader_placeholder(
                arguments,
                "TemplatePath",
                example,
                f"reader-fixtures/dcs/{example.line}",
            )
            copy_reader_fixture(
                BSP_DCS_OBJECT_FIXTURE,
                workspace / reader_content_path(raw, "Template.xml"),
            )
            continue
        if tool_name in {"unica.form.info", "unica.form.validate"}:
            raw = replace_reader_placeholder(
                arguments,
                "FormPath",
                example,
                f"reader-fixtures/forms/{example.line}",
            )
            copy_reader_fixture(
                BSP_FORM_BUSINESS_PROCESS_FIXTURE,
                workspace / reader_content_path(raw, "Form.xml"),
            )
            continue
        if tool_name == "unica.interface.validate":
            raw = replace_reader_placeholder(
                arguments,
                "CIPath",
                example,
                f"reader-fixtures/interfaces/{example.line}",
            )
            copy_reader_fixture(
                interface_fixture,
                workspace / reader_content_path(raw, "CommandInterface.xml"),
            )
            continue
        if tool_name in {"unica.mxl.info", "unica.mxl.decompile"}:
            if "SrcDir" in arguments:
                arguments.pop("SrcDir")
                raw = f"reader-fixtures/mxl/{example.line}/Ext/Template.xml"
                arguments["TemplatePath"] = raw
            else:
                raw = replace_reader_placeholder(
                    arguments,
                    "TemplatePath",
                    example,
                    f"reader-fixtures/mxl/{example.line}/Ext/Template.xml",
                )
                path = Path(raw)
                if path.suffix.lower() != ".xml":
                    raw = (path / "Ext" / "Template.xml").as_posix()
                    arguments["TemplatePath"] = raw
            target = workspace / raw
            copy_reader_fixture(BSP_MXL_RECEIPT_FIXTURE, target)
            continue
        if tool_name == "unica.mxl.validate":
            raw = replace_reader_placeholder(
                arguments,
                "TemplatePath",
                example,
                f"reader-fixtures/mxl/{example.line}",
            )
            target = workspace / reader_content_path(raw, "Template.xml")
            copy_reader_fixture(BSP_MXL_RECEIPT_FIXTURE, target)
            continue
        if tool_name in {"unica.role.info", "unica.role.validate"}:
            raw = replace_reader_placeholder(
                arguments,
                "RightsPath",
                example,
                f"src/Roles/ReaderParity{example.line}/Ext/Rights.xml",
            )
            rights = workspace / raw
            if rights.suffix.lower() != ".xml":
                rights = rights / "Ext" / "Rights.xml"
                arguments["RightsPath"] = str(rights.relative_to(workspace))
            copy_reader_fixture(role_rights_fixture, rights)
            role_dir = rights.parent.parent
            copy_reader_fixture("role-info/SalesReader.xml", role_dir.with_suffix(".xml"))
            if "MetadataPath" in arguments:
                arguments["MetadataPath"] = "src/Configuration.xml"
            continue
        if tool_name in {"unica.subsystem.info", "unica.subsystem.validate"}:
            raw = str(arguments["SubsystemPath"])
            if not raw.startswith("src/cf/"):
                raw = raw.removeprefix("src/")
                arguments["SubsystemPath"] = f"src/cf/{raw}"
            continue
        if tool_name in {"unica.documentation.search", "unica.documentation.get"}:
            continue
        raise AssertionError(f"unhandled reader fixture: {example.document}:{example.line} {tool_name}")

    if handled != readers:
        raise AssertionError(f"reader fixture helpers missed tools: {sorted(readers - handled)}")

    for root in [workspace, source_roots["main"]]:
        subsystems = root / "Subsystems"
        write_named_subsystem_fixture(subsystems / "Sales.xml", "Sales")
        write_named_subsystem_fixture(subsystems / "Администрирование.xml", "Администрирование")
        write_named_subsystem_fixture(
            subsystems / "Продажи.xml",
            "Продажи",
            child="ОптовыеПродажи",
        )
        write_named_subsystem_fixture(
            subsystems / "Продажи" / "Subsystems" / "ОптовыеПродажи.xml",
            "ОптовыеПродажи",
        )
    register_configuration_child(source_roots["main"] / "Configuration.xml", "Subsystem", "Продажи")
    register_configuration_child(source_roots["main"] / "Configuration.xml", "Subsystem", "Администрирование")


def current_reader_standin_target() -> tuple[str, str, str]:
    machine = platform.machine().lower()
    if sys.platform == "darwin" and machine in {"arm64", "aarch64"}:
        return "darwin-arm64", "aarch64-apple-darwin", ""
    if sys.platform.startswith("linux") and machine in {"x86_64", "amd64"}:
        return "linux-x64", "x86_64-unknown-linux-gnu", ""
    raise AssertionError(f"reader stand-ins do not support {sys.platform}/{machine}")


def prepare_reader_standins(temp_root: Path) -> tuple[Path, dict[str, str], Path]:
    plugin_root = temp_root / "plugin" / "unica"
    (plugin_root / "skills").mkdir(parents=True)
    shutil.copytree(PLUGIN_ROOT / "references", plugin_root / "references")
    for manifest_name in [
        ".mcp.json",
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
    ]:
        source = PLUGIN_ROOT / manifest_name
        target = plugin_root / manifest_name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    target_id, target_triple, suffix = current_reader_standin_target()
    binary_dir = plugin_root / "bin" / target_id
    binary_dir.mkdir(parents=True)
    binaries = {
        f"bsl-analyzer{suffix}": READER_STANDINS_ROOT / "bsl_mcp.py",
        f"rlm-bsl-mcp{suffix}": READER_STANDINS_ROOT / "bsl_mcp.py",
        f"rlm-bsl-index{suffix}": READER_STANDINS_ROOT / "rlm_index.py",
        # Mutation examples are previews, but a preview may only promise a
        # runnable command when the declared runner actually exists. The
        # stand-in is never executed in dry-run mode; its bytes make the
        # successful-fixture precondition explicit.
        f"v8-runner{suffix}": READER_STANDINS_ROOT / "bsl_mcp.py",
    }
    manifest_tools = []
    for binary_name, source in binaries.items():
        target = binary_dir / binary_name
        shutil.copyfile(source, target)
        target.chmod(0o755)
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
        tool_name = binary_name.removesuffix(suffix)
        manifest_tools.append(
            {
                "name": tool_name,
                "binaries": {
                    target_id: {
                        "targetTriple": target_triple,
                        "binaryPath": target.relative_to(plugin_root).as_posix(),
                        "sha256": digest,
                    }
                },
            }
        )
    third_party = plugin_root / "third-party"
    third_party.mkdir()
    (third_party / "manifest.json").write_text(
        json.dumps({"schemaVersion": 2, "tools": manifest_tools}, indent=2) + "\n",
        encoding="utf-8",
    )
    lock = json.loads((PLUGIN_ROOT / "third-party/tools.lock.json").read_text(encoding="utf-8"))
    digest_by_name = {
        entry["name"]: entry["binaries"][target_id]["sha256"] for entry in manifest_tools
    }
    for tool in lock["tools"]:
        if tool["name"] in digest_by_name:
            tool["assets"][target_id]["sha256"] = digest_by_name[tool["name"]]
    (third_party / "tools.lock.json").write_text(
        json.dumps(lock, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    call_log = temp_root / "reader-standin-calls.jsonl"
    return plugin_root, {
        "UNICA_PLUGIN_ROOT": str(plugin_root),
        "UNICA_READER_STANDIN_LOG": str(call_log),
    }, call_log


def v8_block(data: bytes, next_block: int = 0x7FFF_FFFF) -> bytes:
    return (
        b"\r\n"
        + f"{len(data):08x} {len(data):08x} {next_block:08x} ".encode()
        + b"\r\n"
        + data
    )


def v8_entry_header(name: str) -> bytes:
    return b"\0" * 20 + name.encode("utf-16-le") + b"\0\0\0\0"


def v8_container(entries: list[tuple[str, bytes]]) -> bytes:
    toc_len = 12 * len(entries)
    cursor = 16 + 31 + toc_len
    addresses: list[tuple[int, int]] = []
    body = bytearray()
    for name, data in entries:
        header = v8_block(v8_entry_header(name))
        payload = v8_block(data)
        addresses.append((cursor, cursor + len(header)))
        cursor += len(header) + len(payload)
        body.extend(header)
        body.extend(payload)
    toc = bytearray()
    for header_address, data_address in addresses:
        for field in [header_address, data_address, 0x7FFF_FFFF]:
            toc.extend(field.to_bytes(4, "little"))
    return (
        cursor.to_bytes(4, "little")
        + (512).to_bytes(4, "little")
        + b"\0" * 8
        + v8_block(bytes(toc))
        + bytes(body)
    )


def hbk_bytes(pages: dict[str, str]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, html in pages.items():
            archive.writestr(name, html)
    return v8_container([("FileStorage", buffer.getvalue())])


def prepare_platform_help_installation(temp_root: Path, workspace: Path) -> Path:
    installation = temp_root / "platform" / "8.3.27.2074"
    installation.mkdir(parents=True)
    locator = "objects/catalog238/ValueTable/methods/GroupBy1290.html"
    pages = {
        "global/StringFind.html": (
            "<html><body><h1>СтрНайти</h1><p>СтрНайти выполняет поиск строки.</p></body></html>"
        ),
        locator: (
            "<html><body><h1>ТаблицаЗначений.Свернуть</h1>"
            "<p>ТаблицаЗначений Свернуть GroupBy удаляет дубли группировкой.</p></body></html>"
        ),
        "collections/ArrayDelete.html": (
            "<html><body><h1>Массив.Удалить</h1><p>Как удалить элемент массива.</p></body></html>"
        ),
    }
    (installation / "shcntx_ru.hbk").write_bytes(hbk_bytes(pages))
    (installation / "1cv8_ru.hbk").write_bytes(
        hbk_bytes({"guides/local.html": "<html><body><h1>Локальная справка</h1></body></html>"})
    )
    with (workspace / "v8project.yaml").open("a", encoding="utf-8") as config:
        config.write(
            "tools:\n"
            "  platform:\n"
            "    version: '8.3.27.2074'\n"
            f"    path: '{installation.as_posix()}'\n"
        )
    return installation


class V8StdStandin:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.server: ThreadingHTTPServer | None = None
        self.thread: threading.Thread | None = None

    def start(self) -> str:
        fixture = json.loads(
            (READER_STANDINS_ROOT / "v8std_response.json").read_text(encoding="utf-8")
        )
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                length = int(self.headers.get("Content-Length", "0"))
                payload = json.loads(self.rfile.read(length))
                owner.calls.append(
                    {"host": self.headers.get("Host"), "path": self.path, "payload": payload}
                )
                response = json.loads(json.dumps(fixture))
                response["id"] = payload.get("id")
                body = json.dumps(response).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, _format: str, *_args: object) -> None:
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        host, port = self.server.server_address
        return f"http://{host}:{port}/mcp"

    def stop(self) -> None:
        if self.server is not None:
            self.server.shutdown()
            self.server.server_close()
        if self.thread is not None:
            self.thread.join(timeout=5)


META_INFO_SKILL_EXAMPLE_DIRECTORIES = {
    "Catalog": "Catalogs",
    "Document": "Documents",
    "InformationRegister": "InformationRegisters",
    "CommonModule": "CommonModules",
    "HTTPService": "HTTPServices",
    "WebService": "WebServices",
    "EventSubscription": "EventSubscriptions",
    "ScheduledJob": "ScheduledJobs",
    "DefinedType": "DefinedTypes",
}


def prepare_meta_add_skill_example(
    source_roots: dict[str, Path], arguments: dict[str, Any]
) -> None:
    """Materialize prerequisites selected by a documented Meta add template."""
    if arguments["kind"] != "EventSubscription":
        return
    source_root = source_roots[arguments["sourceSet"]]
    operations = arguments.get("operations", ())
    if not operations:
        write_meta_event_handler_fixture(
            source_root,
            "EventHandlers",
            "OnBeforeWrite",
            ("Source", "Cancel"),
        )
        return
    prepare_event_subscription_binding_skill_example(
        source_root,
        operations,
    )


def prepare_meta_info_skill_example(
    source_roots: dict[str, Path],
    arguments: dict[str, Any],
) -> None:
    """Materialize the object a documented `unica.meta.info` example addresses.

    The example names an object logically, so the fixture is derived from that
    address instead of from a path spelled out in the document.
    """
    kind, _, name = arguments["metadataPath"].partition(".")
    directory = META_INFO_SKILL_EXAMPLE_DIRECTORIES.get(kind)
    if directory is None:
        raise AssertionError(f"unsupported meta.info example kind: {kind}")
    source_root = source_roots[arguments["sourceSet"]]
    descriptor = source_root / directory / f"{name}.xml"
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    if kind == "EventSubscription":
        write_meta_event_subscription_fixture(descriptor, name)
        register_meta_skill_object(source_root, kind, name)
        return
    descriptor_uuid = uuid.uuid5(uuid.NAMESPACE_URL, f"unica-skill-example:{kind}.{name}")
    drill = arguments.get("Name")
    children = ""
    if drill and kind == "HTTPService":
        children = (
            f"<URLTemplate><Properties><Name>{drill}</Name><Template>/{drill}</Template>"
            f"</Properties><ChildObjects><Method><Properties><Name>Get</Name>"
            f"<HTTPMethod>GET</HTTPMethod><Handler>Обработчик</Handler></Properties>"
            f"</Method></ChildObjects></URLTemplate>"
        )
    elif drill and kind == "WebService":
        children = (
            f"<Operation><Properties><Name>{drill}</Name><XDTOReturningValueType>"
            f"{{http://www.w3.org/2001/XMLSchema}}string</XDTOReturningValueType>"
            f"<ProcedureName>Обработчик</ProcedureName></Properties></Operation>"
        )
    elif drill:
        children = (
            f"<Attribute><Properties><Name>{drill}</Name>"
            f"<Type><v8:Type xmlns:v8=\"http://v8.1c.ru/8.1/data/core\">"
            f"xs:string</v8:Type></Type></Properties></Attribute>"
        )
    descriptor.write_text(
        '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">'
        f'<{kind} uuid="{descriptor_uuid}"><Properties><Name>{name}</Name></Properties>'
        f"<ChildObjects>{children}</ChildObjects></{kind}></MetaDataObject>\n",
        encoding="utf-8",
    )
    register_meta_skill_object(source_root, kind, name)


def register_meta_skill_object(source_root: Path, kind: str, name: str) -> None:
    configuration = source_root / "Configuration.xml"
    text = configuration.read_text(encoding="utf-8")
    registration = f"\t\t\t<{kind}>{name}</{kind}>\n"
    if registration in text:
        return
    closing = "\t\t</ChildObjects>"
    if closing not in text:
        raise AssertionError("typed Meta fixture has no Configuration ChildObjects")
    configuration.write_text(
        text.replace(closing, registration + closing, 1),
        encoding="utf-8",
    )


def prepare_role_edit_skill_example(
    source_roots: dict[str, Path], arguments: dict[str, Any]
) -> None:
    """Create one registered exact-profile role for the documented logical call."""
    kind, separator, name = arguments["metadataPath"].partition(".")
    if (kind, bool(separator and name)) != ("Role", True):
        raise AssertionError("role.edit skill example must use Role.<name>")
    source_root = source_roots[arguments["sourceSet"]]
    descriptor = source_root / "Roles" / f"{name}.xml"
    rights = source_root / "Roles" / name / "Ext/Rights.xml"
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    rights.parent.mkdir(parents=True, exist_ok=True)
    descriptor.write_text(
        '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">'
        f'<Role uuid="{stable_meta_skill_uuid(f"role:{name}")}">'
        f"<Properties><Name>{name}</Name></Properties></Role></MetaDataObject>\n",
        encoding="utf-8",
    )
    operations = arguments.get("operations")
    if not isinstance(operations, list) or not operations:
        raise AssertionError("role.edit skill example must declare operations")
    targets: dict[str, dict[str, bool]] = {}
    for index, operation in enumerate(operations):
        if not isinstance(operation, dict) or operation.get("op") != "setRight":
            raise AssertionError(
                f"role.edit skill example operation {index} must be setRight"
            )
        object_name = operation.get("objectName")
        right = operation.get("right")
        value = operation.get("value")
        if (
            not isinstance(object_name, str)
            or not object_name
            or not isinstance(right, str)
            or not right
            or not isinstance(value, bool)
        ):
            raise AssertionError(
                f"role.edit skill example operation {index} has invalid fields"
            )
        targets.setdefault(object_name, {}).setdefault(right, not value)
    objects = "".join(
        "<object>"
        f"<name>{escape(object_name)}</name>"
        + "".join(
            "<right>"
            f"<name>{escape(right)}</name>"
            f"<value>{str(initial_value).lower()}</value>"
            "</right>"
            for right, initial_value in rights_by_name.items()
        )
        + "</object>"
        for object_name, rights_by_name in targets.items()
    )
    rights.write_text(
        '<Rights xmlns="http://v8.1c.ru/8.2/roles" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" '
        'xsi:type="Rights" version="2.20">'
        "<setForNewObjects>false</setForNewObjects>"
        "<setForAttributesByDefault>true</setForAttributesByDefault>"
        "<independentRightsOfChildObjects>false</independentRightsOfChildObjects>"
        f"{objects}</Rights>\n",
        encoding="utf-8",
    )
    register_meta_skill_object(source_root, "Role", name)


def prepare_meta_edit_skill_example(
    source_roots: dict[str, Path],
    example: SkillMcpExample,
    arguments: dict[str, Any],
) -> None:
    """Create a registered object for one typed Meta edit example."""
    kind, separator, name = arguments["metadataPath"].partition(".")
    if not separator or not name:
        raise AssertionError(
            f"invalid typed metadataPath at {example.document}:{example.line}"
        )
    directory = META_INFO_SKILL_EXAMPLE_DIRECTORIES.get(kind)
    if directory is None:
        raise AssertionError(f"unsupported meta.edit example kind: {kind}")
    source_root = source_roots[arguments["sourceSet"]]
    object_path = source_root / directory / f"{name}.xml"
    object_path.parent.mkdir(parents=True, exist_ok=True)

    if kind == "EventSubscription":
        write_meta_event_subscription_fixture(object_path, name)
        register_meta_skill_object(source_root, kind, name)
        prepare_event_subscription_binding_skill_example(
            source_root,
            arguments.get("operations", ()),
        )
        return

    if not object_path.exists():
        is_document = kind == "Document"
        source = FIXTURES_ROOT / (
            BSP_META_DOCUMENT_FIXTURE if is_document else BSP_META_CATALOG_FIXTURE
        )
        xml = source.read_bytes().decode("utf-8-sig")
        if not is_document:
            xml = xml.replace("Catalog.Валюты", f"Catalog.{name}")
            xml = re.sub(
                r"<(Default(?:Object|Folder|List|Choice|FolderChoice)Form)>.*?</\1>",
                r"<\1/>",
                xml,
            )
            child_start = xml.index("\n\t\t<ChildObjects>")
            child_end = xml.rindex("\n\t\t</ChildObjects>")
            child_end += len("\n\t\t</ChildObjects>")
            xml = xml[:child_start] + "\n\t\t<ChildObjects/>" + xml[child_end:]
        xml, replacements = re.subn(
            r"(<Properties>\s*<Name>)[^<]+",
            rf"\g<1>{name}",
            xml,
            count=1,
        )
        if replacements != 1:
            raise AssertionError(f"cannot rename metadata fixture for {object_path}")
        object_path.write_bytes(xml.encode("utf-8"))

    register_meta_skill_object(source_root, kind, name)

    introduced_predefined_ids: set[str] = set()
    required_predefined_ids: list[str] = []
    for operation in arguments.get("operations", ()):
        if operation.get("collection") == "predefinedItems":
            if operation["op"] == "add":
                introduced_predefined_ids.update(
                    element["id"] for element in operation["elements"]
                )
            elif operation["op"] == "update":
                required_predefined_ids.extend(
                    element["id"]
                    for element in operation["elements"]
                    if element["id"] not in introduced_predefined_ids
                )
            elif operation["op"] == "remove":
                required_predefined_ids.extend(
                    item_id
                    for item_id in operation["ids"]
                    if item_id not in introduced_predefined_ids
                )
            continue
        if operation["op"] not in {"update", "remove"}:
            continue
        if operation["collection"] != "attributes":
            raise AssertionError(
                "typed Meta skill fixture can materialize update/remove targets "
                f"only for attributes, got {operation['collection']}"
            )
        scope = operation.get("scope", {}).get("tabularSection")
        target_names = (
            operation["names"]
            if operation["op"] == "remove"
            else [element["name"] for element in operation["elements"]]
        )
        for target_name in target_names:
            ensure_meta_edit_skill_attribute(object_path, target_name, scope)
    if required_predefined_ids:
        ensure_meta_edit_skill_predefined_items(
            object_path, kind, required_predefined_ids
        )


def write_meta_event_subscription_fixture(descriptor: Path, name: str) -> None:
    descriptor.write_text(
        f'''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <EventSubscription uuid="11111111-1111-4111-8111-111111111111">
    <InternalInfo/>
    <Properties>
      <Name>{name}</Name>
      <Source><v8:Type>xs:boolean</v8:Type></Source>
      <Event>BeforeWrite</Event>
      <Handler>CommonModule.EventHandlers.OnBeforeWrite</Handler>
    </Properties>
  </EventSubscription>
</MetaDataObject>
''',
        encoding="utf-8",
    )


def write_meta_information_register_fixture(source_root: Path, name: str) -> None:
    descriptor = source_root / "InformationRegisters" / f"{name}.xml"
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    descriptor.write_text(
        f'''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <InformationRegister uuid="22222222-2222-4222-8222-222222222222">
    <InternalInfo>
      <xr:GeneratedType name="InformationRegisterRecordSet.{name}" category="RecordSet">
        <xr:TypeId>33333333-3333-4333-8333-333333333333</xr:TypeId>
        <xr:ValueId>44444444-4444-4444-8444-444444444444</xr:ValueId>
      </xr:GeneratedType>
    </InternalInfo>
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </InformationRegister>
</MetaDataObject>
''',
        encoding="utf-8",
    )
    register_meta_skill_object(source_root, "InformationRegister", name)


def prepare_event_subscription_binding_skill_example(
    source_root: Path,
    operations: Iterable[dict[str, Any]],
) -> None:
    """Materialize every target named by a documented subscription binding."""
    sources: list[dict[str, Any]] = []
    properties: dict[str, Any] = {}
    for operation in operations:
        if operation.get("relation") == "source":
            sources.extend(operation.get("targets", ()))
        if operation.get("op") == "setProperties":
            properties.update(operation.get("values", {}))

    source_signatures: set[tuple[str, ...]] = set()
    for source in sources:
        source_kind = source.get("kind")
        metadata_path = source.get("metadataPath")
        if source_kind == "object" and isinstance(metadata_path, str):
            target_kind, separator, target_name = metadata_path.partition(".")
            if not separator or target_kind != "Catalog":
                raise AssertionError(
                    "typed Meta skill object source fixture supports a Catalog target"
                )
            write_meta_catalog_object_fixture(source_root, target_name)
            source_signatures.add(("Source", "Cancel"))
            continue
        if source_kind == "recordSet" and isinstance(metadata_path, str):
            target_kind, separator, target_name = metadata_path.partition(".")
            if not separator or target_kind != "InformationRegister":
                raise AssertionError(
                    "typed Meta skill recordSet source fixture supports an "
                    "InformationRegister target"
                )
            write_meta_information_register_fixture(source_root, target_name)
            source_signatures.add(("Source", "Cancel", "Replacing"))
            continue
        raise AssertionError(
            f"unsupported documented EventSubscription source fixture: {source!r}"
        )

    event = properties.get("Event")
    if event != "BeforeWrite":
        raise AssertionError(
            f"unsupported documented EventSubscription event fixture: {event!r}"
        )
    if len(source_signatures) != 1:
        raise AssertionError(
            "documented EventSubscription sources must have one common signature"
        )
    handler = properties.get("Handler")
    if not isinstance(handler, str):
        raise AssertionError("documented EventSubscription binding has no Handler")
    prefix, module_name, procedure_name, *tail = handler.split(".")
    if prefix != "CommonModule" or tail:
        raise AssertionError(f"invalid documented EventSubscription Handler: {handler}")
    write_meta_event_handler_fixture(
        source_root,
        module_name,
        procedure_name,
        next(iter(source_signatures)),
    )


def write_meta_catalog_object_fixture(source_root: Path, name: str) -> None:
    descriptor = source_root / "Catalogs" / f"{name}.xml"
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    descriptor_uuid = stable_meta_skill_uuid(f"Catalog.{name}")
    type_uuid = stable_meta_skill_v4_uuid(f"CatalogObject.{name}.type")
    value_uuid = stable_meta_skill_v4_uuid(f"CatalogObject.{name}.value")
    descriptor.write_text(
        f'''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <Catalog uuid="{descriptor_uuid}">
    <InternalInfo>
      <xr:GeneratedType name="CatalogObject.{name}" category="Object">
        <xr:TypeId>{type_uuid}</xr:TypeId>
        <xr:ValueId>{value_uuid}</xr:ValueId>
      </xr:GeneratedType>
    </InternalInfo>
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
''',
        encoding="utf-8",
    )
    register_meta_skill_object(source_root, "Catalog", name)


def write_meta_event_handler_fixture(
    source_root: Path,
    module_name: str,
    procedure_name: str,
    parameters: tuple[str, ...],
) -> None:
    descriptor = source_root / "CommonModules" / f"{module_name}.xml"
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    descriptor_uuid = stable_meta_skill_uuid(f"CommonModule.{module_name}")
    descriptor.write_text(
        f'''<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <CommonModule uuid="{descriptor_uuid}">
    <Properties>
      <Name>{module_name}</Name>
      <Global>false</Global>
      <ClientManagedApplication>false</ClientManagedApplication>
      <Server>true</Server>
      <ExternalConnection>true</ExternalConnection>
      <ClientOrdinaryApplication>false</ClientOrdinaryApplication>
      <ServerCall>false</ServerCall>
      <Privileged>false</Privileged>
      <ReturnValuesReuse>DontUse</ReturnValuesReuse>
    </Properties>
  </CommonModule>
</MetaDataObject>
''',
        encoding="utf-8",
    )
    module = source_root / "CommonModules" / module_name / "Ext" / "Module.bsl"
    module.parent.mkdir(parents=True, exist_ok=True)
    module.write_text(
        f"Procedure {procedure_name}({', '.join(parameters)}) Export\nEndProcedure\n",
        encoding="utf-8",
    )
    register_meta_skill_object(source_root, "CommonModule", module_name)


def stable_meta_skill_uuid(identity: str) -> str:
    digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:32]
    return (
        f"{digest[:8]}-{digest[8:12]}-{digest[12:16]}-"
        f"{digest[16:20]}-{digest[20:32]}"
    )


def ensure_meta_edit_skill_predefined_items(
    object_path: Path, kind: str, item_ids: list[str]
) -> None:
    """Materialize initial UUID targets used by documented update/remove calls."""
    root_types = {
        "Catalog": "CatalogPredefinedItems",
        "ChartOfAccounts": "ChartOfAccountsPredefinedItems",
        "ChartOfCharacteristicTypes": "PlanOfCharacteristicKindPredefinedItems",
        "ChartOfCalculationTypes": "CalculationTypePredefinedItems",
    }
    root_type = root_types.get(kind)
    if root_type is None:
        raise AssertionError(f"unsupported predefinedItems owner in skill fixture: {kind}")
    unique_ids = list(dict.fromkeys(item_ids))
    items = "\n".join(
        f'\t<Item id="{item_id}"><Name>Fixture{index}</Name></Item>'
        for index, item_id in enumerate(unique_ids, start=1)
    )
    predefined = object_path.with_suffix("") / "Ext/Predefined.xml"
    predefined.parent.mkdir(parents=True, exist_ok=True)
    predefined.write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" '
        f'xsi:type="{root_type}" version="2.20">\n'
        f"{items}\n"
        "</PredefinedData>\n",
        encoding="utf-8",
    )


def stable_meta_skill_v4_uuid(identity: str) -> str:
    digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:32]
    return str(uuid.UUID(hex=digest, version=4))


def ensure_meta_edit_skill_tabular_section(
    object_path: Path, section_name: str
) -> None:
    """Clone a valid section when a documented scoped operation needs it."""
    xml = object_path.read_text(encoding="utf-8")
    section_pattern = re.compile(
        r"(?ms)^\t\t\t<TabularSection\b.*?^\t\t\t</TabularSection>"
    )
    sections = list(section_pattern.finditer(xml))
    for match in sections:
        name = re.search(r"<Name>([^<]+)</Name>", match.group(0))
        if name is not None and name.group(1) == section_name:
            return
    if not sections:
        raise AssertionError(f"no reusable TabularSection fixture in {object_path}")

    section = sections[0].group(0)
    source_name = re.search(r"<Name>([^<]+)</Name>", section)
    if source_name is None:
        raise AssertionError(f"reusable TabularSection has no Name in {object_path}")
    section = section.replace(source_name.group(1), section_name)
    uuid_index = 0

    def replace_uuid(match: re.Match[str]) -> str:
        nonlocal uuid_index
        uuid_index += 1
        return stable_meta_skill_uuid(
            f"tabularSection:{section_name}:{uuid_index}:{match.group(0)}"
        )

    section = re.sub(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-"
        r"[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        replace_uuid,
        section,
    )
    root_close = re.search(r"(?m)^\t\t</ChildObjects>\s*$", xml)
    if root_close is None:
        raise AssertionError(f"no root ChildObjects closing tag in {object_path}")
    object_path.write_text(
        f"{xml[:root_close.start()]}{section}\n{xml[root_close.start():]}",
        encoding="utf-8",
    )


def ensure_meta_edit_skill_attribute(
    object_path: Path, attribute_name: str, tabular_section: str | None = None
) -> None:
    """Clone a valid attribute so remove/modify examples have a real target."""
    if tabular_section is not None:
        ensure_meta_edit_skill_tabular_section(object_path, tabular_section)
    xml = object_path.read_text(encoding="utf-8")
    if tabular_section is None:
        container_start = 0
        container_end = len(xml)
        close_pattern = r"(?m)^\t\t</ChildObjects>\s*$"
        attribute_pattern = r"(?ms)^\t\t\t<Attribute\b.*?^\t\t\t</Attribute>"
        indent = "\t\t\t"
    else:
        section_match = next(
            (
                match
                for match in re.finditer(
                    r"(?ms)^\t\t\t<TabularSection\b.*?^\t\t\t</TabularSection>",
                    xml,
                )
                if (
                    (name := re.search(r"<Name>([^<]+)</Name>", match.group(0)))
                    is not None
                    and name.group(1) == tabular_section
                )
            ),
            None,
        )
        if section_match is None:
            raise AssertionError(
                f"cannot materialize TabularSection {tabular_section} in {object_path}"
            )
        container_start, container_end = section_match.span()
        close_pattern = r"(?m)^\t\t\t\t</ChildObjects>\s*$"
        attribute_pattern = (
            r"(?ms)^\t\t\t\t\t<Attribute\b.*?^\t\t\t\t\t</Attribute>"
        )
        indent = "\t\t\t\t\t"

    container = xml[container_start:container_end]
    attributes = list(re.finditer(attribute_pattern, container))
    for match in attributes:
        name = re.search(r"<Name>([^<]+)</Name>", match.group(0))
        if name is not None and name.group(1) == attribute_name:
            return
    if not attributes:
        raise AssertionError(f"no reusable Attribute fixture in {object_path}")
    match = attributes[0]
    attribute = match.group(0)
    attribute = re.sub(
        r"(<Name>)[^<]+(</Name>)",
        rf"\g<1>{attribute_name}\g<2>",
        attribute,
        count=1,
    )
    fixture_uuid = stable_meta_skill_uuid(
        f"attribute:{tabular_section or '<root>'}:{attribute_name}"
    )
    attribute = re.sub(
        r'uuid="[^"]+"',
        f'uuid="{fixture_uuid}"',
        attribute,
        count=1,
    )
    close = re.search(close_pattern, container)
    if close is None:
        raise AssertionError(f"no target ChildObjects closing tag in {object_path}")
    insert_at = container_start + close.start()
    if not attribute.startswith(indent):
        raise AssertionError(f"unexpected Attribute indentation in {object_path}")
    xml = f"{xml[:insert_at]}{attribute}\n{xml[insert_at:]}"
    object_path.write_text(xml, encoding="utf-8")


def script_args(arguments: dict[str, Any]) -> list[str]:
    result: list[str] = []
    for key in sorted(arguments):
        if key in {"dryRun", "cwd", "confirm", "args"}:
            continue
        value = arguments[key]
        flag = f"-{pascal_case_key(key)}"
        if value is True:
            result.append(flag)
        elif value is False or value is None:
            continue
        elif isinstance(value, list):
            result.append(flag)
            result.append(" ;; ".join(value_to_cli_string(item) for item in value))
        else:
            result.append(flag)
            result.append(value_to_cli_string(value))
    return result


def pascal_case_key(key: str) -> str:
    return key[:1].upper() + key[1:]


def value_to_cli_string(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    return json.dumps(value, ensure_ascii=False)


def normalize_command(command: list[str], workspace: Path) -> list[str]:
    return [normalize_text(part, workspace) for part in command]


def normalize_text(text: str, workspace: Path) -> str:
    normalized = text.replace("\r\r\n", "\r\n").replace("\r\n", "\n").replace("\r", "\n")
    normalized = normalized.replace(str(workspace.resolve()), "<WORKSPACE>")
    normalized = normalized.replace(str(workspace), "<WORKSPACE>")
    normalized = normalized.replace(str(REPO_ROOT), "<REPO>")
    if os.name == "nt":
        normalized = normalized.replace(str(workspace.resolve()).replace("\\", "/"), "<WORKSPACE>")
        normalized = normalized.replace(str(workspace).replace("\\", "/"), "<WORKSPACE>")
        normalized = normalized.replace(str(REPO_ROOT).replace("\\", "/"), "<REPO>")
        normalized = normalized.replace(r"\\?\<WORKSPACE>", "<WORKSPACE>")
        normalized = normalized.replace(r"\\?\<REPO>", "<REPO>")
        normalized = re.sub(
            r"<(?:WORKSPACE|REPO)>[^\s\"']*",
            lambda match: match.group(0).replace("\\", "/"),
            normalized,
        )
        normalized = re.sub(
            r"(?<![\w.-])(?:src(?:-cfe)?|exts|[.]build)\\[^\s\"'<>]+",
            lambda match: match.group(0).replace("\\", "/"),
            normalized,
        )
        normalized = re.sub(
            r"(?m)^(?P<label>[ \t]*(?:File|Module|Output|Path|Config|Configuration):[ \t]+)(?P<path>[^\r\n]+)$",
            lambda match: match.group("label") + match.group("path").replace("\\", "/"),
            normalized,
        )
    normalized = re.sub(
        r"<REPO>/tests/fixtures/unica_mcp_script_parity/unica_reference_models/([^/\s\"']+)/scripts/([^/\s\"']+)",
        r"<REPO>/<SKILL_SCRIPT>/\1/\2",
        normalized,
    )
    normalized = re.sub(
        r"<REPO>/tests/fixtures/unica_mcp_script_parity/cc-1c-skills/skills/([^/\s\"']+)/scripts/([^/\s\"']+)",
        r"<REPO>/<CC_1C_SKILL_SCRIPT>/\1/\2",
        normalized,
    )
    normalized = UUID_RE.sub("<UUID>", normalized)
    return normalized


def normalize_snapshot_text(text: str, workspace: Path) -> str:
    normalized = normalize_text(
        text.replace("&#13;\r\n", "\r\n").replace("&#13;\n", "\n"),
        workspace,
    )
    normalized = re.sub(
        r'(<\?xml\s+version="1\.0"\s+encoding=")utf-8(")',
        r"\1UTF-8\2",
        normalized,
        count=1,
    )
    return normalized.removesuffix("\n")


class ReaderStandinFixtureTests(unittest.TestCase):
    def test_windows_target_is_not_claimed_without_native_launchers(self) -> None:
        with (
            mock.patch.object(sys, "platform", "win32"),
            mock.patch.object(os, "name", "nt"),
            mock.patch.object(platform, "machine", return_value="AMD64"),
        ):
            with self.assertRaisesRegex(AssertionError, "do not support"):
                current_reader_standin_target()

    def test_rlm_index_standin_reports_missing_index_root(self) -> None:
        env = os.environ.copy()
        env.pop("RLM_INDEX_DIR", None)
        result = subprocess.run(
            [
                sys.executable,
                str(READER_STANDINS_ROOT / "rlm_index.py"),
                "index",
                "info",
            ],
            cwd=REPO_ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 2, result)
        self.assertIn("RLM_INDEX_DIR must be set", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


class WindowsParityNormalizationTests(unittest.TestCase):
    def test_cfe_borrow_execution_separates_case_colliding_extension_root(
        self,
    ) -> None:
        self.assertEqual(
            project_cc_case_path("cfe-borrow", "ext"),
            "extension",
        )
        self.assertEqual(
            project_cc_case_path(
                "cfe-borrow",
                "{workDir}/ext/Catalogs/Товары.xml",
            ),
            "{workDir}/extension/Catalogs/Товары.xml",
        )
        self.assertEqual(
            project_cc_case_path("meta-compile", "ext"),
            "ext",
        )

    def test_empty_donor_config_is_projected_to_bound_8_3_27_profile(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            configuration = workspace / "Configuration.xml"
            configuration.write_text(
                '<MetaDataObject version="2.17"><Configuration/></MetaDataObject>',
                encoding="utf-8",
            )

            project_empty_config_to_8_3_27(workspace)

            self.assertIn(
                'version="2.20"',
                configuration.read_text(encoding="utf-8"),
            )

    def test_snapshot_ignores_one_optional_terminal_newline(self) -> None:
        workspace = Path("/parity-workspace")

        self.assertEqual(
            normalize_snapshot_text("first\n", workspace),
            normalize_snapshot_text("first", workspace),
        )
        self.assertNotEqual(
            normalize_snapshot_text("first\n\n", workspace),
            normalize_snapshot_text("first", workspace),
        )

    def test_exact_byte_snapshot_detects_xdto_bom_and_eol_drift(self) -> None:
        fixture = (
            REPO_ROOT
            / "tests"
            / "fixtures"
            / "xdto"
            / "enterprise-data-minimal"
            / "XDTOPackages"
            / "EnterpriseData_1_17_3"
            / "Ext"
            / "Package.bin"
        )
        original = fixture.read_bytes()
        self.assertTrue(original.startswith(b"\xef\xbb\xbf"))
        self.assertIn(b"\r\n", original)

        with tempfile.TemporaryDirectory(prefix="unica-exact-byte-snapshot-") as temp:
            workspace = Path(temp)
            target = workspace / "XDTOPackages/P/Ext/Package.bin"
            target.parent.mkdir(parents=True)
            target.write_bytes(original)
            normalized_before = snapshot_workspace(workspace)
            exact_before = snapshot_workspace_bytes(workspace)

            mutated = original.removeprefix(b"\xef\xbb\xbf").replace(b"\r\n", b"\n")
            self.assertNotEqual(mutated, original)
            target.write_bytes(mutated)

            self.assertEqual(snapshot_workspace(workspace), normalized_before)
            self.assertNotEqual(snapshot_workspace_bytes(workspace), exact_before)

    def test_non_path_backslashes_remain_significant(self) -> None:
        workspace = Path("C:/parity-workspace")

        self.assertNotEqual(normalize_text(r"a\b", workspace), normalize_text("a/b", workspace))

    def test_blank_lines_remain_significant(self) -> None:
        workspace = Path("C:/parity-workspace")

        self.assertNotEqual(normalize_text("first\n\nsecond\n", workspace), normalize_text("first\nsecond\n", workspace))

    @unittest.skipUnless(os.name == "nt", "Windows text-mode newline artifact")
    def test_subprocess_decode_removes_only_doubled_carriage_return(self) -> None:
        doubled = subprocess.CompletedProcess([], 0, stdout=b"first\r\r\nsecond", stderr=b"")
        real_blank = subprocess.CompletedProcess([], 0, stdout=b"first\r\n\r\nsecond", stderr=b"")

        self.assertEqual(decoded_completed_process(doubled).stdout, "first\r\nsecond")
        self.assertEqual(decoded_completed_process(real_blank).stdout, "first\r\n\r\nsecond")

    @unittest.skipUnless(os.name == "nt", "Windows path separator equivalence")
    def test_known_workspace_paths_normalize_separators(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-normalize-path-") as tmp:
            workspace = Path(tmp)
            windows_path = f"output={workspace}\\src\\Template.xml"
            slash_path = f"output={workspace.as_posix()}/src/Template.xml"

            self.assertEqual(normalize_text(windows_path, workspace), normalize_text(slash_path, workspace))

    @unittest.skipUnless(os.name == "nt", "Windows path field equivalence")
    def test_documented_path_fields_normalize_separators(self) -> None:
        workspace = Path("C:/parity-workspace")

        self.assertEqual(
            normalize_text("     File: .\\Catalogs\\Item.xml\n", workspace),
            normalize_text("     File: ./Catalogs/Item.xml\n", workspace),
        )


def snapshot_workspace(workspace: Path) -> dict[str, str]:
    snapshot: dict[str, str] = {}
    for path in sorted(workspace.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(workspace).as_posix()
        if rel.startswith(".build/") or rel.startswith(".unica-cache/"):
            continue
        data = path.read_bytes()
        try:
            text = data.decode("utf-8-sig")
        except UnicodeDecodeError:
            snapshot[rel] = "sha256:" + hashlib.sha256(data).hexdigest()
            continue
        snapshot[rel] = normalize_snapshot_text(text, workspace)
    return snapshot


def snapshot_workspace_bytes(workspace: Path) -> dict[str, bytes]:
    snapshot: dict[str, bytes] = {}
    for path in sorted(workspace.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(workspace).as_posix()
        if rel.startswith(".build/") or rel.startswith(".unica-cache/"):
            continue
        snapshot[rel] = path.read_bytes()
    return snapshot


for _retired_test in {
    "test_donor_cases_match_reviewed_relations",
    "test_donor_inventory_relations_preview_and_snapshot_are_closed",
}:
    setattr(
        UnicaMcpScriptParityTests,
        _retired_test,
        unittest.skip(
            "v0.12 skill parity is intentionally outside the v0.13 surface-first cutover"
        )(getattr(UnicaMcpScriptParityTests, _retired_test)),
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(add_help=False)
    cli.add_argument("--write-donor-observations", type=Path)
    cli_args, unittest_args = cli.parse_known_args()
    if cli_args.write_donor_observations is not None:
        if unittest_args:
            cli.error(
                "unittest arguments cannot be combined with "
                "--write-donor-observations"
            )
        write_donor_observation_candidates(cli_args.write_donor_observations)
    else:
        unittest.main(argv=[sys.argv[0], *unittest_args])
