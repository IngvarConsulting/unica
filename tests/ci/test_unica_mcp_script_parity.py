from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

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


@dataclasses.dataclass(frozen=True)
class SetupStep:
    skill: str
    script: str
    arguments: dict[str, Any]
    tool: str | None = None


@dataclasses.dataclass(frozen=True)
class FileFixture:
    source: str
    target: str


META_VALIDATE_COMPILED_OWNER_FIXTURES = (
    FileFixture("meta-validate-parity-owner/Configuration.xml", "src/Configuration.xml"),
    FileFixture(
        "meta-validate-parity-owner/Languages/Русский.xml",
        "src/Languages/Русский.xml",
    ),
)

BSP_META_VALIDATE_OWNER_FIXTURES = (
    FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
    FileFixture("bsp/meta/Languages/Русский.xml", "src/Languages/Русский.xml"),
)


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


@dataclasses.dataclass(frozen=True)
class SkillMcpExample:
    skill: str
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
        name="cf-init-basic",
        tool="unica.cf.init",
        skill="cf-init",
        script="cf-init.py",
        arguments={
            "Name": "ParityConfiguration",
            "Synonym": "Parity configuration",
            "OutputDir": "src",
            "Version": "1.0.0.1",
            "Vendor": "Unica",
            "CompatibilityMode": "Version8_3_24",
        },
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-init-basic",
        tool="unica.cfe.init",
        skill="cfe-init",
        script="cfe-init.py",
        arguments={
            "Name": "ParityExtension",
            "Synonym": "Parity extension",
            "NamePrefix": "PE_",
            "OutputDir": "src-cfe",
            "Purpose": "Patch",
            "Version": "1.0.0.1",
            "Vendor": "Unica",
            "CompatibilityMode": "Version8_3_24",
            "NoRole": True,
        },
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-init-with-role",
        tool="unica.cfe.init",
        skill="cfe-init",
        script="cfe-init.py",
        arguments={
            "Name": "ParityExtensionRole",
            "Synonym": "Parity extension role",
            "NamePrefix": "PER_",
            "OutputDir": "src-cfe-role",
            "Purpose": "Customization",
            "Version": "2.0.0.0",
            "Vendor": "Unica",
            "CompatibilityMode": "Version8_3_24",
        },
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-validate-detailed-outfile",
        tool="unica.cfe.validate",
        skill="cfe-validate",
        script="cfe-validate.py",
        arguments={
            "ExtensionPath": "src-cfe/Configuration.xml",
            "Detailed": True,
            "OutFile": "cfe-validate.txt",
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
        name="cfe-patch-method-before-borrowed-common-module",
        tool="unica.cfe.patch_method",
        skill="cfe-patch-method",
        script="cfe-patch-method.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ModulePath": "CommonModule.GoogleПереводчик",
            "MethodName": "ОбновитьДанные",
            "InterceptorType": "Before",
            "Context": "НаСервере",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                arguments={
                    "Name": "ParityExtension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
            SetupStep(
                skill="cfe-borrow",
                script="cfe-borrow.py",
                tool="unica.cfe.borrow",
                arguments={
                    "ExtensionPath": "src-cfe",
                    "ConfigPath": "src",
                    "Object": "CommonModule.GoogleПереводчик",
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(
                BSP_META_COMMON_MODULE_FIXTURE,
                "src/CommonModules/GoogleПереводчик.xml",
            ),
            FileFixture(
                "cfe-patch-method/base-common-module.bsl",
                "src/CommonModules/GoogleПереводчик/Ext/Module.bsl",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-patch-method-after-borrowed-common-module",
        tool="unica.cfe.patch_method",
        skill="cfe-patch-method",
        script="cfe-patch-method.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ModulePath": "CommonModule.GoogleПереводчик",
            "MethodName": "ОбновитьДанные",
            "InterceptorType": "After",
            "Context": "НаСервере",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                arguments={
                    "Name": "ParityExtension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
            SetupStep(
                skill="cfe-borrow",
                script="cfe-borrow.py",
                tool="unica.cfe.borrow",
                arguments={
                    "ExtensionPath": "src-cfe",
                    "ConfigPath": "src",
                    "Object": "CommonModule.GoogleПереводчик",
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(
                BSP_META_COMMON_MODULE_FIXTURE,
                "src/CommonModules/GoogleПереводчик.xml",
            ),
            FileFixture(
                "cfe-patch-method/base-common-module.bsl",
                "src/CommonModules/GoogleПереводчик/Ext/Module.bsl",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-borrow-catalog-object",
        tool="unica.cfe.borrow",
        skill="cfe-borrow",
        script="cfe-borrow.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Object": "Catalog.ParityCatalog",
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
                    "NoRole": True,
                },
            ),
        ),
        fixtures=(
            FileFixture("cfe-borrow/Configuration.xml", "src/Configuration.xml"),
            FileFixture("cfe-borrow/Catalogs/ParityCatalog.xml", "src/Catalogs/ParityCatalog.xml"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-cfe-borrow-real-catalog-object",
        tool="unica.cfe.borrow",
        skill="cfe-borrow",
        script="cfe-borrow.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Object": "Catalog.Валюты",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                tool="unica.cfe.init",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-cfe-borrow-russian-types-batch",
        tool="unica.cfe.borrow",
        skill="cfe-borrow",
        script="cfe-borrow.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Object": "Справочник.Валюты;;Документ.АктОбУничтоженииПерсональныхДанных",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                tool="unica.cfe.init",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),
            FileFixture(
                BSP_META_DOCUMENT_FIXTURE,
                "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-cfe-borrow-real-document-object",
        tool="unica.cfe.borrow",
        skill="cfe-borrow",
        script="cfe-borrow.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Object": "Document.АктОбУничтоженииПерсональныхДанных",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                tool="unica.cfe.init",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(BSP_META_DOCUMENT_FIXTURE, "src/Documents/АктОбУничтоженииПерсональныхДанных.xml"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-cfe-borrow-business-process-form-main-attribute",
        tool="unica.cfe.borrow",
        skill="cfe-borrow",
        script="cfe-borrow.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Object": "BusinessProcess.Задание.Form.ФормаБизнесПроцесса",
            "BorrowMainAttribute": "Form",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                tool="unica.cfe.init",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(
                "cfe-borrow-bsp-form/BusinessProcesses/Задание.xml",
                "src/BusinessProcesses/Задание.xml",
            ),
            FileFixture(
                "cfe-borrow-bsp-form/BusinessProcesses/Задание/Forms/ФормаБизнесПроцесса.xml",
                "src/BusinessProcesses/Задание/Forms/ФормаБизнесПроцесса.xml",
            ),
            FileFixture(
                BSP_FORM_BUSINESS_PROCESS_FIXTURE,
                "src/BusinessProcesses/Задание/Forms/ФормаБизнесПроцесса/Ext/Form.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-diff-empty-extension-mode-a",
        tool="unica.cfe.diff",
        skill="cfe-diff",
        script="cfe-diff.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Mode": "A",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                arguments={
                    "Name": "ParityExtension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "NoRole": True,
                },
            ),
            SetupStep(
                skill="cf-init",
                script="cf-init.py",
                arguments={
                    "Name": "ParityConfiguration",
                    "OutputDir": "src",
                },
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-cfe-diff-borrowed-catalog-mode-a",
        tool="unica.cfe.diff",
        skill="cfe-diff",
        script="cfe-diff.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Mode": "A",
        },
        setup_steps=(
            SetupStep(
                skill="cfe-init",
                script="cfe-init.py",
                tool="unica.cfe.init",
                arguments={
                    "Name": "ParityExtension",
                    "Synonym": "Parity extension",
                    "NamePrefix": "PE_",
                    "OutputDir": "src-cfe",
                    "Purpose": "Customization",
                    "Version": "1.0.0.1",
                    "Vendor": "Unica",
                    "CompatibilityMode": "Version8_3_24",
                    "NoRole": True,
                },
            ),
            SetupStep(
                skill="cfe-borrow",
                script="cfe-borrow.py",
                tool="unica.cfe.borrow",
                arguments={
                    "ExtensionPath": "src-cfe",
                    "ConfigPath": "src",
                    "Object": "Catalog.Валюты",
                },
            ),
        ),
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-cfe-diff-transfer-check-mode-b",
        tool="unica.cfe.diff",
        skill="cfe-diff",
        script="cfe-diff.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ConfigPath": "src",
            "Mode": "B",
        },
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(
                "cfe-diff/mode-b/src/Catalogs/Валюты/Ext/ObjectModule.bsl",
                "src/Catalogs/Валюты/Ext/ObjectModule.bsl",
            ),
            FileFixture(
                "cfe-diff/mode-b/src-cfe/Configuration.xml",
                "src-cfe/Configuration.xml",
            ),
            FileFixture(
                "cfe-diff/mode-b/src-cfe/Catalogs/Валюты.xml",
                "src-cfe/Catalogs/Валюты.xml",
            ),
            FileFixture(
                "cfe-diff/mode-b/src-cfe/Catalogs/Валюты/Ext/ObjectModule.bsl",
                "src-cfe/Catalogs/Валюты/Ext/ObjectModule.bsl",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="cf-info-overview-outfile",
        tool="unica.cf.info",
        skill="cf-info",
        script="cf-info.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Mode": "overview",
            "OutFile": "cf-info.txt",
        },
        fixtures=(FileFixture("cf-info/Configuration.xml", "src/Configuration.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cf-info-full-with-interface-ext",
        tool="unica.cf.info",
        skill="cf-info",
        script="cf-info.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Mode": "full",
            "Limit": 120,
        },
        fixtures=(
            FileFixture("cf-info/Configuration.xml", "src/Configuration.xml"),
            FileFixture(
                "cf-info/Ext/ClientApplicationInterface.xml",
                "src/Ext/ClientApplicationInterface.xml",
            ),
            FileFixture(
                "cf-info/Ext/HomePageWorkArea.xml",
                "src/Ext/HomePageWorkArea.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="cf-info-home-page-section",
        tool="unica.cf.info",
        skill="cf-info",
        script="cf-info.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Section": "home-page",
            "Limit": 120,
        },
        fixtures=(
            FileFixture("cf-info/Configuration.xml", "src/Configuration.xml"),
            FileFixture(
                "cf-info/Ext/HomePageWorkArea.xml",
                "src/Ext/HomePageWorkArea.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="cf-validate-detailed-outfile",
        tool="unica.cf.validate",
        skill="cf-validate",
        script="cf-validate.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Detailed": True,
            "OutFile": "cf-validate.txt",
        },
        fixtures=(
            FileFixture("cf-validate/Configuration.xml", "src/Configuration.xml"),
            FileFixture("cf-validate/Languages/Русский.xml", "src/Languages/Русский.xml"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-cf-info-brief",
        tool="unica.cf.info",
        skill="cf-info",
        script="cf-info.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Mode": "brief",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-cf-info-full",
        tool="unica.cf.info",
        skill="cf-info",
        script="cf-info.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-cf-validate-detailed",
        tool="unica.cf.validate",
        skill="cf-validate",
        script="cf-validate.py",
        arguments={
            "ConfigPath": "src/Configuration.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="cf-edit-definition-file-all-ops",
        tool="unica.cf.edit",
        skill="cf-edit",
        script="cf-edit.py",
        arguments={
            "ConfigPath": "src",
            "DefinitionFile": "fixtures/cf-edit-ops.json",
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="cf-init",
                script="cf-init.py",
                arguments={"Name": "ParityConfiguration", "OutputDir": "src"},
            ),
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
            SetupStep(
                skill="form-add",
                script="form-add.py",
                arguments={
                    "ObjectPath": "src/Catalogs/ParityCatalog.xml",
                    "FormName": "ListForm",
                    "Purpose": "List",
                },
            ),
            SetupStep(
                skill="form-add",
                script="form-add.py",
                arguments={
                    "ObjectPath": "src/Catalogs/ParityCatalog.xml",
                    "FormName": "ObjectForm",
                    "Purpose": "Object",
                },
            ),
        ),
        fixtures=(
            FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),
            FileFixture("cf-edit/ops.json", "fixtures/cf-edit-ops.json"),
        ),
        expect_ok=True,
        # Native cf.edit preserves Configuration.xml text for child-object edits;
        # the donor script still rewrites the XML and is no longer a byte oracle.
        compare_files=False,
    ),
    ParityScenario(
        name="meta-compile-catalog",
        tool="unica.meta.compile",
        skill="meta-compile",
        script="meta-compile.py",
        arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
        fixtures=(FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-meta-edit-catalog-ops",
        tool="unica.meta.edit",
        skill="meta-edit",
        script="meta-edit.py",
        arguments={
            "ObjectPath": "src/Catalogs/Валюты.xml",
            "DefinitionFile": "fixtures/meta-edit-bsp-catalog-ops.json",
            "NoValidate": True,
        },
        fixtures=(
            FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),
            FileFixture(
                "meta-edit/bsp-catalog-ops.json",
                "fixtures/meta-edit-bsp-catalog-ops.json",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="meta-remove-catalog",
        tool="unica.meta.remove",
        skill="meta-remove",
        script="meta-remove.py",
        arguments={"ConfigDir": "src", "Object": "Catalog.ParityCatalog"},
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "Value": {
                        "name": "Sales",
                        "synonym": "Sales",
                        "content": [
                            "Catalog.ParityCatalog",
                            "Catalog.KeepCatalog",
                        ],
                    },
                    "OutputDir": "src",
                    "NoValidate": True,
                },
            ),
        ),
        fixtures=(
            FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),
            FileFixture("meta-remove/Configuration.xml", "src/Configuration.xml"),
            FileFixture(
                "cf-validate/Languages/Русский.xml",
                "src/Languages/Русский.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="meta-info-catalog-overview-outfile",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/Catalogs/ParityCatalog.xml",
            "Mode": "overview",
            "OutFile": "meta-info.txt",
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
        ),
        fixtures=(FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="meta-validate-catalog-detailed-outfile",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Catalogs/ParityCatalog.xml",
            "Detailed": True,
            "OutFile": "meta-validate.txt",
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
        ),
        fixtures=META_VALIDATE_COMPILED_OWNER_FIXTURES
        + (FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="meta-validate-language-aware",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Enums/LanguageAware.xml",
            "Detailed": True,
        },
        fixtures=(
            FileFixture(
                "meta-validate-language-aware/Configuration.xml",
                "src/Configuration.xml",
            ),
            FileFixture(
                "meta-validate-language-aware/Languages/Русский.xml",
                "src/Languages/Русский.xml",
            ),
            FileFixture(
                "meta-validate-language-aware/Languages/English.xml",
                "src/Languages/English.xml",
            ),
            FileFixture(
                "meta-validate-language-aware/Enums/LanguageAware.xml",
                "src/Enums/LanguageAware.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="help-add-catalog",
        tool="unica.help.add",
        skill="help-add",
        script="add-help.py",
        arguments={
            "ObjectName": "Catalogs/ParityCatalog",
            "SrcDir": "src",
            "Lang": "ru",
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
        ),
        fixtures=(FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-meta-info-catalog-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/Catalogs/Валюты.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-catalog-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Catalogs/Валюты.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-info-document-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(
                BSP_META_DOCUMENT_FIXTURE,
                "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-document-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (
            FileFixture(
                BSP_META_DOCUMENT_FIXTURE,
                "src/Documents/АктОбУничтоженииПерсональныхДанных.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-info-report-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/Reports/АнализВерсийОбъектов.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_META_REPORT_FIXTURE, "src/Reports/АнализВерсийОбъектов.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-report-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Reports/АнализВерсийОбъектов.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (FileFixture(BSP_META_REPORT_FIXTURE, "src/Reports/АнализВерсийОбъектов.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-info-common-module-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/CommonModules/GoogleПереводчик.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(BSP_META_COMMON_MODULE_FIXTURE, "src/CommonModules/GoogleПереводчик.xml"),
            FileFixture(
                BSP_META_COMMON_MODULE_BSL_FIXTURE,
                "src/CommonModules/GoogleПереводчик/Ext/Module.bsl",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-common-module-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/CommonModules/GoogleПереводчик.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (
            FileFixture(BSP_META_COMMON_MODULE_FIXTURE, "src/CommonModules/GoogleПереводчик.xml"),
            FileFixture(
                BSP_META_COMMON_MODULE_BSL_FIXTURE,
                "src/CommonModules/GoogleПереводчик/Ext/Module.bsl",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-info-enum-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/Enums/ВажностьПроблемыУчета.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_META_ENUM_FIXTURE, "src/Enums/ВажностьПроблемыУчета.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-enum-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Enums/ВажностьПроблемыУчета.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (FileFixture(BSP_META_ENUM_FIXTURE, "src/Enums/ВажностьПроблемыУчета.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-info-information-register-full",
        tool="unica.meta.info",
        skill="meta-info",
        script="meta-info.py",
        arguments={
            "ObjectPath": "src/InformationRegisters/АдминистративнаяИерархия.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(
                BSP_META_INFORMATION_REGISTER_FIXTURE,
                "src/InformationRegisters/АдминистративнаяИерархия.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-meta-validate-information-register-detailed",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/InformationRegisters/АдминистративнаяИерархия.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=BSP_META_VALIDATE_OWNER_FIXTURES
        + (
            FileFixture(
                BSP_META_INFORMATION_REGISTER_FIXTURE,
                "src/InformationRegisters/АдминистративнаяИерархия.xml",
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
        name="form-edit-additions",
        tool="unica.form.edit",
        skill="form-edit",
        script="form-edit.py",
        arguments={
            "FormPath": "forms/Form.xml",
            "JsonPath": "fixtures/form-edit-additions.json",
        },
        setup_steps=(
            SetupStep(
                skill="form-compile",
                script="form-compile.py",
                arguments={
                    "JsonPath": "fixtures/form-simple.json",
                    "OutputPath": "forms/Form.xml",
                },
            ),
        ),
        fixtures=(
            FileFixture("form-simple.json", "fixtures/form-simple.json"),
            FileFixture("form-edit/additions.json", "fixtures/form-edit-additions.json"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="form-edit-valuetable-attribute-columns",
        tool="unica.form.edit",
        skill="form-edit",
        script="form-edit.py",
        arguments={
            "FormPath": "forms/Form.xml",
            "JsonPath": "fixtures/form-edit-valuetable-columns.json",
        },
        setup_steps=(
            SetupStep(
                skill="form-compile",
                script="form-compile.py",
                arguments={
                    "JsonPath": "fixtures/form-simple.json",
                    "OutputPath": "forms/Form.xml",
                },
            ),
        ),
        fixtures=(
            FileFixture("form-simple.json", "fixtures/form-simple.json"),
            FileFixture(
                "form-edit/valuetable-columns.json",
                "fixtures/form-edit-valuetable-columns.json",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-form-info-real-form-full",
        tool="unica.form.info",
        skill="form-info",
        script="form-info.py",
        arguments={
            "FormPath": "src/Form.xml",
            "Expand": "*",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ФормаСписка/Form.xml",
                "src/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-form-validate-real-form-detailed",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Form.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ФормаСписка/Form.xml",
                "src/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-form-info-real-action-run-form",
        tool="unica.form.info",
        skill="form-info",
        script="form-info.py",
        arguments={
            "FormPath": "src/Form.xml",
            "Expand": "attributes,commands,events",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ДействиеВыполнить/Form.xml",
                "src/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-form-validate-real-action-check-form",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Form.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ДействиеПроверить/Form.xml",
                "src/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-form-info-real-business-process-form",
        tool="unica.form.info",
        skill="form-info",
        script="form-info.py",
        arguments={
            "FormPath": "src/Form.xml",
            "Expand": "*",
            "Limit": 200,
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ФормаБизнесПроцесса/Form.xml",
                "src/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-form-edit-add-attribute-command-element",
        tool="unica.form.edit",
        skill="form-edit",
        script="form-edit.py",
        arguments={
            "FormPath": "src/Form.xml",
            "JsonPath": "fixtures/form-edit-bsp-additions.json",
        },
        fixtures=(
            FileFixture(
                "bsp/forms/BusinessProcesses__Задание__ФормаСписка/Form.xml",
                "src/Form.xml",
            ),
            FileFixture(
                "form-edit/bsp-additions.json",
                "fixtures/form-edit-bsp-additions.json",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="form-info-main-form",
        tool="unica.form.info",
        skill="form-info",
        script="form-info.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
        },
        fixtures=(
            FileFixture(
                "form-remove/ParityReport/Forms/MainForm/Ext/Form.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="form-validate-detailed",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "Detailed": True,
        },
        fixtures=(
            FileFixture(
                "form-validate/Form.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="form-validate-valid-binding-paths",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "Detailed": True,
        },
        fixtures=(
            FileFixture(
                "form-validate/ValidBindings.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
        expect_ok=True,
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
        name="subsystem-info-full",
        tool="unica.subsystem.info",
        skill="subsystem-info",
        script="subsystem-info.py",
        arguments={
            "SubsystemPath": "src/Subsystems/Subsystems/ParitySubsystem.xml",
            "Mode": "full",
            "OutFile": "subsystem-info.txt",
            "Limit": 0,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/subsystem-sales.json",
                    "OutputDir": "src/Subsystems",
                    "NoValidate": True,
                },
            ),
        ),
        fixtures=(FileFixture("subsystem-sales.json", "fixtures/subsystem-sales.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="subsystem-validate-detailed",
        tool="unica.subsystem.validate",
        skill="subsystem-validate",
        script="subsystem-validate.py",
        arguments={
            "SubsystemPath": "src/Subsystems/Subsystems/ParitySubsystem.xml",
            "Detailed": True,
            "OutFile": "subsystem-validate.txt",
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/subsystem-sales.json",
                    "OutputDir": "src/Subsystems",
                    "NoValidate": True,
                },
            ),
        ),
        fixtures=(FileFixture("subsystem-sales.json", "fixtures/subsystem-sales.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-subsystem-info-full",
        tool="unica.subsystem.info",
        skill="subsystem-info",
        script="subsystem-info.py",
        arguments={
            "SubsystemPath": "src/Subsystems/Администрирование.xml",
            "Mode": "full",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_SUBSYSTEM_FIXTURE, "src/Subsystems/Администрирование.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-subsystem-validate-detailed",
        tool="unica.subsystem.validate",
        skill="subsystem-validate",
        script="subsystem-validate.py",
        arguments={
            "SubsystemPath": "src/Subsystems/Администрирование.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(FileFixture(BSP_SUBSYSTEM_FIXTURE, "src/Subsystems/Администрирование.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="subsystem-edit-definition-file-all-ops",
        tool="unica.subsystem.edit",
        skill="subsystem-edit",
        script="subsystem-edit.py",
        arguments={
            "SubsystemPath": "src/Subsystems/Subsystems/ParitySubsystem.xml",
            "DefinitionFile": "fixtures/subsystem-edit-ops.json",
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/subsystem-sales.json",
                    "OutputDir": "src/Subsystems",
                    "NoValidate": True,
                },
            ),
        ),
        fixtures=(
            FileFixture("subsystem-sales.json", "fixtures/subsystem-sales.json"),
            FileFixture("subsystem-edit/ops.json", "fixtures/subsystem-edit-ops.json"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="form-remove-main-form",
        tool="unica.form.remove",
        skill="form-remove",
        script="remove-form.py",
        arguments={
            "ObjectName": "ParityReport",
            "FormName": "MainForm",
            "SrcDir": "src/Reports",
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-report.json", "OutputDir": "src"},
            ),
            SetupStep(
                skill="form-add",
                script="form-add.py",
                arguments={
                    "ObjectPath": "src/Reports/ParityReport.xml",
                    "FormName": "MainForm",
                    "Purpose": "Object",
                    "Synonym": "Main form",
                    "SetDefault": True,
                },
            ),
        ),
        fixtures=(
            FileFixture("meta-report.json", "fixtures/meta-report.json"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="form-add-catalog-list-default",
        tool="unica.form.add",
        skill="form-add",
        script="form-add.py",
        arguments={
            "ObjectPath": "src/Catalogs/ParityCatalog.xml",
            "FormName": "ListForm",
            "Purpose": "List",
            "Synonym": "List form",
            "SetDefault": True,
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-catalog.json", "OutputDir": "src"},
            ),
        ),
        fixtures=(FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="template-add-main-schema",
        tool="unica.template.add",
        skill="template-add",
        script="add-template.py",
        arguments={
            "ObjectName": "ParityReport",
            "TemplateName": "NewSchema",
            "TemplateType": "DataCompositionSchema",
            "Synonym": "New schema",
            "SrcDir": "src/Reports",
            "SetMainSKD": True,
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-report.json", "OutputDir": "src"},
            ),
        ),
        fixtures=(FileFixture("meta-report.json", "fixtures/meta-report.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-template-add-real-report-copy",
        tool="unica.template.add",
        skill="template-add",
        script="add-template.py",
        arguments={
            "ObjectName": "АнализВерсийОбъектов",
            "TemplateName": "ParityBspTemplate",
            "TemplateType": "DataCompositionSchema",
            "Synonym": "Parity BSP template",
            "SrcDir": "src/Reports",
        },
        fixtures=(FileFixture(BSP_META_REPORT_FIXTURE, "src/Reports/АнализВерсийОбъектов.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-template-remove-real-template-from-report-copy",
        tool="unica.template.remove",
        skill="template-remove",
        script="remove-template.py",
        arguments={
            "ObjectName": "АнализВерсийОбъектов",
            "TemplateName": "ОсновнаяСхемаКомпоновкиДанных",
            "SrcDir": "src/Reports",
        },
        fixtures=(
            FileFixture(BSP_META_REPORT_FIXTURE, "src/Reports/АнализВерсийОбъектов.xml"),
            FileFixture(
                BSP_META_REPORT_TEMPLATE_FIXTURE,
                "src/Reports/АнализВерсийОбъектов/Templates/ОсновнаяСхемаКомпоновкиДанных.xml",
            ),
            FileFixture(
                BSP_META_REPORT_TEMPLATE_CONTENT_FIXTURE,
                "src/Reports/АнализВерсийОбъектов/Templates/"
                "ОсновнаяСхемаКомпоновкиДанных/Ext/Template.xml",
            ),
        ),
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
            "OutFile": "interface-validate.txt",
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
        name="bsp-interface-edit-show-real-command",
        tool="unica.interface.edit",
        skill="interface-edit",
        script="interface-edit.py",
        arguments={
            "CIPath": "src/Subsystems/Администрирование/Ext/CommandInterface.xml",
            "Operation": "show",
            "Value": "Catalog.Пользователи.StandardCommand.OpenList",
            "NoValidate": True,
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
        compare_files=True,
    ),
    ParityScenario(
        name="interface-edit-definition-file-all-ops",
        tool="unica.interface.edit",
        skill="interface-edit",
        script="interface-edit.py",
        arguments={
            "CIPath": "src/Subsystems/Sales/Ext/CommandInterface.xml",
            "DefinitionFile": "fixtures/interface-edit-ops.json",
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "Value": {"name": "Sales", "synonym": "Sales"},
                    "OutputDir": "src",
                    "NoValidate": True,
                },
            ),
        ),
        fixtures=(
            FileFixture(
                "interface-validate/Sales/Ext/CommandInterface.xml",
                "src/Subsystems/Sales/Ext/CommandInterface.xml",
            ),
            FileFixture("interface-edit/ops.json", "fixtures/interface-edit-ops.json"),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="interface-edit-create-if-missing",
        tool="unica.interface.edit",
        skill="interface-edit",
        script="interface-edit.py",
        arguments={
            "CIPath": "src/Subsystems/NewSales/Ext/CommandInterface.xml",
            "Operation": "subsystem-order",
            "Value": "[\"Subsystem.Sales.Subsystem.Retail\",\"Subsystem.Sales.Subsystem.Wholesale\"]",
            "CreateIfMissing": True,
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "Value": {"name": "NewSales", "synonym": "New sales"},
                    "OutputDir": "src",
                    "NoValidate": True,
                },
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="interface-edit-create-if-missing-hide",
        tool="unica.interface.edit",
        skill="interface-edit",
        script="interface-edit.py",
        arguments={
            "CIPath": "src/Subsystems/NewVisibility/Ext/CommandInterface.xml",
            "Operation": "hide",
            "Value": "Catalog.Products.StandardCommand.OpenList",
            "CreateIfMissing": True,
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "Value": {
                        "name": "NewVisibility",
                        "synonym": "New visibility",
                    },
                    "OutputDir": "src",
                    "NoValidate": True,
                },
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="interface-edit-create-if-missing-show",
        tool="unica.interface.edit",
        skill="interface-edit",
        script="interface-edit.py",
        arguments={
            "CIPath": "src/Subsystems/NewVisible/Ext/CommandInterface.xml",
            "Operation": "show",
            "Value": "Catalog.Products.StandardCommand.OpenList",
            "CreateIfMissing": True,
            "NoValidate": True,
        },
        setup_steps=(
            SetupStep(
                skill="subsystem-compile",
                script="subsystem-compile.py",
                arguments={
                    "Value": {"name": "NewVisible", "synonym": "New visible"},
                    "OutputDir": "src",
                    "NoValidate": True,
                },
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="template-remove-main-schema",
        tool="unica.template.remove",
        skill="template-remove",
        script="remove-template.py",
        arguments={
            "ObjectName": "ParityReport",
            "TemplateName": "MainSchema",
            "SrcDir": "src/Reports",
        },
        setup_steps=(
            SetupStep(
                skill="meta-compile",
                script="meta-compile.py",
                arguments={"JsonPath": "fixtures/meta-report.json", "OutputDir": "src"},
            ),
            SetupStep(
                skill="template-add",
                script="add-template.py",
                arguments={
                    "ObjectName": "ParityReport",
                    "TemplateName": "MainSchema",
                    "TemplateType": "DataCompositionSchema",
                    "Synonym": "Main schema",
                    "SrcDir": "src/Reports",
                    "SetMainSKD": True,
                },
            ),
        ),
        fixtures=(
            FileFixture("meta-report.json", "fixtures/meta-report.json"),
        ),
        expect_ok=True,
        compare_files=True,
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
        name="dcs-info-overview-outfile",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Mode": "overview",
            "OutFile": "dcs-info.txt",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-overview",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "overview", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-query",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОсновнойНаборДанных",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_DCS_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-query-named-nested-union",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОпределениеПолей",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-fields",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "fields", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-links",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "links", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-calculated",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "calculated", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-resources",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "resources", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-params",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "params", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-variant",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "variant", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-trace",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "trace",
            "Name": "КоличествоДанных",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-templates",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "templates", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-info-full",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "full", "Limit": 200},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-dcs-validate-real-template-detailed",
        tool="unica.dcs.validate",
        skill="dcs-validate",
        script="dcs-validate.py",
        arguments={"TemplatePath": "src/Template.xml", "Detailed": True, "MaxErrors": 80},
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="dcs-validate-detailed-outfile",
        tool="unica.dcs.validate",
        skill="dcs-validate",
        script="dcs-validate.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
            "Detailed": True,
            "OutFile": "dcs-validate.txt",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-modify-structure",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "modify-structure",
            "Value": "Price @name=G2",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "set-structure",
                    "Value": "Code @name=G1 > Quantity @name=G2 > details",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-selection-in-named-variant",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-selection",
            "Value": "Code",
            "Variant": "Alt",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "add-variant",
                    "Value": "Alt [Alt presentation]",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-selection-folder",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-selection",
            "Value": "Folder(Parity folder: Code, Quantity)",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-selection-to-named-structure-group",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-selection",
            "Value": "Quantity @group=G1",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "set-structure",
                    "Value": "Code @name=G1 > details",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-query",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "patch-query",
            "Value": "1 => 2 @once",
            "DataSet": "ОсновнойНаборДанных",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-query",
                    "Value": "ВЫБРАТЬ\n\t1 КАК Ссылка",
                    "DataSet": "ОсновнойНаборДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-set-query-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-query",
            "Value": "ВЫБРАТЬ\n\t2 КАК Ссылка",
            "DataSet": "ОсновнойНаборДанных",
        },
        fixtures=(FileFixture(BSP_DCS_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-variant-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-variant",
            "Value": "ParityVariantFinal [Parity variant final]",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-calculated-field-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-calculated-field",
            "Value": "ParityCalcFinal: decimal(10,2) = КоличествоДанных + 1",
            "NoSelection": True,
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-modify-field-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "modify-field",
            "Value": "ПредставлениеДанных [Представление parity final]: string",
            "DataSet": "МестаИспользования",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-set-field-role-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-field-role",
            "Value": "ПредставлениеДанных @dimension",
            "DataSet": "МестаИспользования",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-order-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-order",
            "Value": "КоличествоДанных desc",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-clear-order-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-order",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-order",
                    "Value": "КоличествоДанных desc",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-clear-selection-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-selection",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-selection",
                    "Value": "КоличествоДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-clear-filter-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-filter",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-remove-filter-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-filter",
            "Value": "КоличествоДанных",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-data-parameter-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataParameter",
            "Value": "ДатаФормирования = LastMonth",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-data-set-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataSet",
            "Value": "ParityDataSetFinal: ВЫБРАТЬ 1 КАК КоличествоДанных",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-data-set-link-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataSetLink",
            "Value": "МестаИспользования > ParityDataSetFinal on КоличествоДанных = КоличествоДанных [param ParityLinkFinal]",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataSet",
                    "Value": "ParityDataSetFinal: ВЫБРАТЬ 1 КАК КоличествоДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-set-output-parameter-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-outputParameter",
            "Value": "Заголовок = ParityTitleFinal",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-set-structure-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-structure",
            "Value": "Ссылка @name=ParityRootFinal > details",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-rename-parameter-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "rename-parameter",
            "Value": "ParityRenameParam => ParityRenameParamFinal",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ParityRenameParam",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-remove-field-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-field",
            "Value": "ParityRemoveField",
            "DataSet": "МестаИспользования",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityRemoveField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-remove-field-keeps_group_items",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-field",
            "Value": "ParityGroupedField",
            "DataSet": "МестаИспользования",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityGroupedField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-structure",
                    "Value": "ParityGroupedField @name=ParityGroupedRoot > details",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-conditional-appearance-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-conditionalAppearance",
            "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-clear-conditional-appearance-final",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-conditionalAppearance",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-conditionalAppearance",
                    "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-field-preserves_schema_order_and_role_markers",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-field",
            "Value": "Amount: decimal(10,2) @dimension #noFilter",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-total-aggregate-shorthand",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-total",
            "Value": "Amount: Сумма",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-parameter-typed-available-values",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-parameter",
            "Value": (
                "Period: StandardPeriod = LastMonth "
                "availableValue=LastMonth:Прошлый месяц, ThisMonth:Текущий месяц"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-add-parameter-quoted-value-list-and-available-values",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "add-parameter",
            "Value": (
                "Tags: string = \"one,two\", 'three:four' "
                "availableValue=\"one,two\":\"One, two\", 'three:four':'Three: four'"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-modify-parameter-preserves_typed_value_and_sets_available_values",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "modify-parameter",
            "Value": (
                "Period [Период] value=ThisMonth denyIncompleteValues=true use=Always "
                "availableValue=ThisMonth:Текущий месяц, LastMonth:Прошлый месяц"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "add-parameter",
                    "Value": "Period: StandardPeriod = LastMonth",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-modify-parameter-quoted-value-list-and-available-values",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "modify-parameter",
            "Value": (
                "Tags value=\"one,two\", 'three:four' "
                "availableValue=\"one,two\":\"One, two\", 'three:four':'Three: four'"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "add-parameter",
                    "Value": "Tags: string = initial",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-modify-filter-preserves_existing_disabled_state",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "modify-filter",
            "Value": "Code >= 2",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "add-filter",
                    "Value": "Code = 1 @off",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="dcs-edit-modify-data-parameter-preserves_existing_value",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "modify-dataParameter",
            "Value": "Period @off",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "add-dataParameter",
                    "Value": "Period = LastMonth @off",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-fields-and-resources",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-total",
            "Value": "ВременныйИтог",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-field",
                    "Value": "ПредставлениеДанных [Представление parity]: string",
                    "DataSet": "МестаИспользования",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-field-role",
                    "Value": "ПредставлениеДанных @dimension",
                    "DataSet": "МестаИспользования",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-total",
                    "Value": "ВременныйИтог: Сумма(КоличествоДанных)",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-params",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "reorder-parameters",
            "Value": "ПараметрParityПереименованный, ДатаФормирования, ПредставлениеСписка",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ПараметрParity",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-parameter",
                    "Value": "ПараметрParity [Параметр parity] @hidden @always",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "rename-parameter",
                    "Value": "ПараметрParity => ПараметрParityПереименованный",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-settings",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-drilldown",
            "Value": "КоличествоДанных",
            "Variant": "ParityVariant",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityVariant [Parity variant]",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-structure",
                    "Value": "Ссылка @name=ParityRoot > details",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-structure",
                    "Value": "Данные @name=ParityRoot",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "ВедущееИзмерение = false",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "remove-filter",
                    "Value": "ВедущееИзмерение",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-conditionalAppearance",
                    "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-conditionalAppearance",
                    "Value": "*",
                    "Variant": "ParityVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-add-filter",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-filter",
            "Value": "ВедущееИзмерение = false",
        },
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-calculated-field-lifecycle",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-calculated-field",
            "Value": "ParityCalc",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-calculated-field",
                    "Value": "ParityCalc: decimal(10,2) = КоличествоДанных + 1",
                    "NoSelection": True,
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-datasets-and-variant-params",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "modify-filter",
            "Value": "КоличествоДанных >= 2",
            "Variant": "ParityDataVariant",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityDataVariant [Parity data variant]",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataSet",
                    "Value": "ParityDataSet: ВЫБРАТЬ 1 КАК КоличествоДанных",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataSetLink",
                    "Value": "МестаИспользования > ParityDataSet on КоличествоДанных = КоличествоДанных [param ParityLink]",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-outputParameter",
                    "Value": "Заголовок = ParityTitle",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataParameter",
                    "Value": "ДатаФормирования = LastMonth",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-dataParameter",
                    "Value": "ДатаФормирования = ThisMonth",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                    "Variant": "ParityDataVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-dcs-edit-basic-ops-lifecycle",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-parameter",
            "Value": "ParityCleanupParam",
        },
        setup_steps=(
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityOpsVariant [Parity ops variant]",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityCleanupField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ParityCleanupParam",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-selection",
                    "Value": "ParityCleanupField",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-order",
                    "Value": "ParityCleanupField desc",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "ParityCleanupField = 1",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-selection",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-order",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-filter",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                tool="unica.dcs.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "remove-field",
                    "Value": "ParityCleanupField",
                    "DataSet": "МестаИспользования",
                    "Variant": "ParityOpsVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
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
        name="mxl-decompile-simple-outfile",
        tool="unica.mxl.decompile",
        skill="mxl-decompile",
        script="mxl-decompile.py",
        arguments={
            "TemplatePath": "templates/MXL.xml",
            "OutputPath": "mxl.json",
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
        compare_files=True,
    ),
    ParityScenario(
        name="mxl-info-text",
        tool="unica.mxl.info",
        skill="mxl-info",
        script="mxl-info.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
            "WithText": True,
        },
        setup_steps=(
            SetupStep(
                skill="mxl-compile",
                script="mxl-compile.py",
                arguments={
                    "JsonPath": "fixtures/mxl-simple.json",
                    "OutputPath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
                },
            ),
        ),
        fixtures=(FileFixture("mxl-simple.json", "fixtures/mxl-simple.json"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="mxl-validate-detailed",
        tool="unica.mxl.validate",
        skill="mxl-validate",
        script="mxl-validate.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
            "Detailed": True,
        },
        setup_steps=(
            SetupStep(
                skill="mxl-compile",
                script="mxl-compile.py",
                arguments={
                    "JsonPath": "fixtures/mxl-simple.json",
                    "OutputPath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
                },
            ),
        ),
        fixtures=(FileFixture("mxl-simple.json", "fixtures/mxl-simple.json"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-mxl-info-real-template",
        tool="unica.mxl.info",
        skill="mxl-info",
        script="mxl-info.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
            "WithText": True,
            "Limit": 200,
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
        name="bsp-mxl-validate-real-template",
        tool="unica.mxl.validate",
        skill="mxl-validate",
        script="mxl-validate.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Power/Ext/Template.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(
            FileFixture(
                BSP_MXL_POWER_OF_ATTORNEY_FIXTURE,
                "src/Reports/ParityReport/Templates/Power/Ext/Template.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-mxl-decompile-real-template-outfile",
        tool="unica.mxl.decompile",
        skill="mxl-decompile",
        script="mxl-decompile.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Receipt/Ext/Template.xml",
            "OutputPath": "mxl-bsp.json",
        },
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
                    "OutputPath": "mxl-bsp.json",
                },
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
    ParityScenario(
        name="role-info-show-denied",
        tool="unica.role.info",
        skill="role-info",
        script="role-info.py",
        arguments={
            "RightsPath": "src/Roles/SalesReader/Ext/Rights.xml",
            "ShowDenied": True,
            "Limit": 0,
        },
        fixtures=(
            FileFixture("role-info/SalesReader.xml", "src/Roles/SalesReader.xml"),
            FileFixture(
                "role-info/SalesReader/Ext/Rights.xml",
                "src/Roles/SalesReader/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="role-info-outfile-pagination",
        tool="unica.role.info",
        skill="role-info",
        script="role-info.py",
        arguments={
            "RightsPath": "src/Roles/SalesReader/Ext/Rights.xml",
            "Limit": 5,
            "Offset": 1,
            "OutFile": "role-info.txt",
        },
        fixtures=(
            FileFixture("role-info/SalesReader.xml", "src/Roles/SalesReader.xml"),
            FileFixture(
                "role-info/SalesReader/Ext/Rights.xml",
                "src/Roles/SalesReader/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="role-validate-detailed",
        tool="unica.role.validate",
        skill="role-validate",
        script="role-validate.py",
        arguments={
            "RightsPath": "src/Roles/SalesReader/Ext/Rights.xml",
            "Detailed": True,
            "OutFile": "role-validate.txt",
        },
        fixtures=(
            FileFixture("role-info/SalesReader.xml", "src/Roles/SalesReader.xml"),
            FileFixture(
                "role-info/SalesReader/Ext/Rights.xml",
                "src/Roles/SalesReader/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-role-info-full",
        tool="unica.role.info",
        skill="role-info",
        script="role-info.py",
        arguments={
            "RightsPath": "src/Roles/АдминистраторСистемы/Ext/Rights.xml",
            "Limit": 0,
        },
        fixtures=(
            FileFixture(
                BSP_ROLE_ADMIN_RIGHTS_FIXTURE,
                "src/Roles/АдминистраторСистемы/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-role-info-administration-show-denied",
        tool="unica.role.info",
        skill="role-info",
        script="role-info.py",
        arguments={
            "RightsPath": "src/Roles/Администрирование/Ext/Rights.xml",
            "ShowDenied": True,
            "Limit": 0,
        },
        fixtures=(
            FileFixture(
                BSP_ROLE_ADMINISTRATION_RIGHTS_FIXTURE,
                "src/Roles/Администрирование/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-role-validate-detailed",
        tool="unica.role.validate",
        skill="role-validate",
        script="role-validate.py",
        arguments={
            "RightsPath": "src/Roles/АдминистраторСистемы/Ext/Rights.xml",
            "Detailed": True,
            "MaxErrors": 80,
        },
        fixtures=(
            FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
            FileFixture(
                BSP_ROLE_ADMIN_RIGHTS_FIXTURE,
                "src/Roles/АдминистраторСистемы/Ext/Rights.xml",
            ),
        ),
        expect_ok=True,
    ),
]


VALIDATION_FAILURE_SCENARIOS = [
    ParityScenario(
        name="meta-validate-missing-owner",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Enums/LanguageAware.xml",
            "Detailed": True,
        },
        expect_ok=False,
        fixtures=(
            FileFixture(
                "meta-validate-language-aware/Enums/LanguageAware.xml",
                "src/Enums/LanguageAware.xml",
            ),
        ),
    ),
    ParityScenario(
        name="meta-validate-missing-registered-language",
        tool="unica.meta.validate",
        skill="meta-validate",
        script="meta-validate.py",
        arguments={
            "ObjectPath": "src/Enums/LanguageAware.xml",
            "Detailed": True,
        },
        expect_ok=False,
        fixtures=(
            FileFixture(
                "meta-validate-language-aware/Configuration.xml",
                "src/Configuration.xml",
            ),
            FileFixture(
                "meta-validate-language-aware/Languages/Русский.xml",
                "src/Languages/Русский.xml",
            ),
            FileFixture(
                "meta-validate-language-aware/Enums/LanguageAware.xml",
                "src/Enums/LanguageAware.xml",
            ),
        ),
    ),
    ParityScenario(
        name="form-validate-bare-type-is-error",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "Detailed": True,
        },
        expect_ok=False,
        fixtures=(
            FileFixture(
                "form-validate/BareType.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
    ),
    ParityScenario(
        name="dcs-validate-bad-prefix-namespace",
        tool="unica.dcs.validate",
        skill="dcs-validate",
        script="dcs-validate.py",
        arguments={"TemplatePath": "templates/BadPrefix.xml"},
        expect_ok=False,
        fixtures=(FileFixture("dcs-validate/BadPrefix.xml", "templates/BadPrefix.xml"),),
    ),
    ParityScenario(
        name="dcs-edit-patch-query-once-ambiguous",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "templates/DCS.xml",
            "Operation": "patch-query",
            "Value": "Code => ItemCode @once",
        },
        expect_ok=False,
        setup_steps=(
            SetupStep(
                skill="dcs-compile",
                script="dcs-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/dcs-simple.json",
                    "OutputPath": "templates/DCS.xml",
                },
            ),
            SetupStep(
                skill="dcs-edit",
                script="dcs-edit.py",
                arguments={
                    "TemplatePath": "templates/DCS.xml",
                    "Operation": "set-query",
                    "Value": "SELECT Code AS Code",
                },
            ),
        ),
        fixtures=(FileFixture("dcs-simple.json", "fixtures/dcs-simple.json"),),
    ),
    ParityScenario(
        name="form-validate-duplicate-names-are-errors",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "Detailed": True,
        },
        expect_ok=False,
        fixtures=(
            FileFixture(
                "form-validate/DuplicateNames.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
    ),
    ParityScenario(
        name="form-validate-logform-namespace-is-required-for-structure",
        tool="unica.form.validate",
        skill="form-validate",
        script="form-validate.py",
        arguments={
            "FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "Detailed": True,
        },
        expect_ok=False,
        fixtures=(
            FileFixture(
                "form-validate/NoNamespace.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
    ),
    ParityScenario(
        name="bsp-dcs-info-query-named-union-fails",
        tool="unica.dcs.info",
        skill="dcs-info",
        script="dcs-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОбщееКоличествоЭлементов",
        },
        expect_ok=False,
        fixtures=(FileFixture(BSP_DCS_UNION_FIXTURE, "src/Template.xml"),),
    ),
    ParityScenario(
        name="bsp-dcs-edit-missing-variant-fails",
        tool="unica.dcs.edit",
        skill="dcs-edit",
        script="dcs-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-selection",
            "Value": "КоличествоДанных",
            "Variant": "DefinitelyMissingVariant",
        },
        expect_ok=False,
        fixtures=(FileFixture(BSP_DCS_OBJECT_FIXTURE, "src/Template.xml"),),
    ),
]


MISSING_INPUT_SCENARIOS = [
    ParityScenario(
        "cf-edit-missing-config",
        "unica.cf.edit",
        "cf-edit",
        "cf-edit.py",
        {"ConfigPath": "missing/Configuration.xml", "Operation": "modify-property", "Value": "Version=1.0"},
        False,
    ),
    ParityScenario(
        "cf-info-missing-config",
        "unica.cf.info",
        "cf-info",
        "cf-info.py",
        {"ConfigPath": "missing/Configuration.xml", "Mode": "brief"},
        False,
    ),
    ParityScenario(
        "cf-validate-missing-config",
        "unica.cf.validate",
        "cf-validate",
        "cf-validate.py",
        {"ConfigPath": "missing/Configuration.xml"},
        False,
    ),
    ParityScenario(
        "cfe-borrow-missing-inputs",
        "unica.cfe.borrow",
        "cfe-borrow",
        "cfe-borrow.py",
        {
            "ExtensionPath": "missing-extension",
            "ConfigPath": "missing-config",
            "Object": "Catalog.ParityCatalog",
        },
        False,
    ),
    ParityScenario(
        "cfe-diff-missing-extension",
        "unica.cfe.diff",
        "cfe-diff",
        "cfe-diff.py",
        {"ExtensionPath": "missing-extension", "ConfigPath": "missing-config"},
        False,
    ),
    ParityScenario(
        "cfe-validate-missing-extension",
        "unica.cfe.validate",
        "cfe-validate",
        "cfe-validate.py",
        {"ExtensionPath": "missing-extension"},
        False,
    ),
    ParityScenario(
        "meta-edit-missing-object",
        "unica.meta.edit",
        "meta-edit",
        "meta-edit.py",
        {"ObjectPath": "missing/Catalog.xml", "Operation": "modify-property", "Value": "Synonym=Missing"},
        False,
    ),
    ParityScenario(
        "meta-info-missing-object",
        "unica.meta.info",
        "meta-info",
        "meta-info.py",
        {"ObjectPath": "missing/Catalog.xml", "Mode": "brief"},
        False,
    ),
    ParityScenario(
        "meta-remove-missing-config",
        "unica.meta.remove",
        "meta-remove",
        "meta-remove.py",
        {"ConfigDir": "missing-src", "Object": "Catalog.ParityCatalog", "Force": True},
        False,
    ),
    ParityScenario(
        "meta-validate-missing-object",
        "unica.meta.validate",
        "meta-validate",
        "meta-validate.py",
        {"ObjectPath": "missing/Catalog.xml", "Detailed": True},
        False,
    ),
    ParityScenario(
        "form-add-missing-object",
        "unica.form.add",
        "form-add",
        "form-add.py",
        {"ObjectPath": "missing/Catalog.xml", "FormName": "ФормаЭлемента", "Purpose": "Item"},
        False,
    ),
    ParityScenario(
        "form-edit-missing-form",
        "unica.form.edit",
        "form-edit",
        "form-edit.py",
        {"FormPath": "missing/Form.xml", "JsonPath": "missing/form-edit.json"},
        False,
    ),
    ParityScenario(
        "form-info-missing-form",
        "unica.form.info",
        "form-info",
        "form-info.py",
        {"FormPath": "missing/Form.xml"},
        False,
    ),
    ParityScenario(
        "form-remove-missing-object",
        "unica.form.remove",
        "form-remove",
        "remove-form.py",
        {"ObjectName": "ParityCatalog", "FormName": "ФормаЭлемента", "SrcDir": "missing-src/Catalogs"},
        False,
    ),
    ParityScenario(
        "form-validate-missing-form",
        "unica.form.validate",
        "form-validate",
        "form-validate.py",
        {"FormPath": "missing/Form.xml"},
        False,
    ),
    ParityScenario(
        "form-validate-dangling-binding-tags",
        "unica.form.validate",
        "form-validate",
        "form-validate.py",
        {"FormPath": "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml", "Detailed": True},
        False,
        fixtures=(
            FileFixture(
                "form-validate/DanglingBindings.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
        ),
    ),
    ParityScenario(
        "interface-edit-missing-command-interface",
        "unica.interface.edit",
        "interface-edit",
        "interface-edit.py",
        {"CIPath": "missing/CommandInterface.xml", "Operation": "hide", "Value": "Catalog.ParityCatalog"},
        False,
    ),
    ParityScenario(
        "interface-validate-missing-command-interface",
        "unica.interface.validate",
        "interface-validate",
        "interface-validate.py",
        {"CIPath": "missing/CommandInterface.xml"},
        False,
    ),
    ParityScenario(
        "subsystem-edit-missing-subsystem",
        "unica.subsystem.edit",
        "subsystem-edit",
        "subsystem-edit.py",
        {"SubsystemPath": "missing/Subsystem.xml", "Operation": "add-content", "Value": "Catalog.ParityCatalog"},
        False,
    ),
    ParityScenario(
        "subsystem-info-missing-subsystem",
        "unica.subsystem.info",
        "subsystem-info",
        "subsystem-info.py",
        {"SubsystemPath": "missing/Subsystem.xml", "Mode": "content"},
        False,
    ),
    ParityScenario(
        "subsystem-validate-missing-subsystem",
        "unica.subsystem.validate",
        "subsystem-validate",
        "subsystem-validate.py",
        {"SubsystemPath": "missing/Subsystem.xml"},
        False,
    ),
    ParityScenario(
        "template-add-missing-object",
        "unica.template.add",
        "template-add",
        "add-template.py",
        {
            "ObjectName": "ParityReport",
            "TemplateName": "MainSchema",
            "TemplateType": "DataCompositionSchema",
            "SrcDir": "missing-src/Reports",
        },
        False,
    ),
    ParityScenario(
        "template-remove-missing-object",
        "unica.template.remove",
        "template-remove",
        "remove-template.py",
        {"ObjectName": "ParityReport", "TemplateName": "MainSchema", "SrcDir": "missing-src/Reports"},
        False,
    ),
    ParityScenario(
        "dcs-edit-missing-template",
        "unica.dcs.edit",
        "dcs-edit",
        "dcs-edit.py",
        {"TemplatePath": "missing/Template.xml", "Operation": "add-field", "Value": "Amount: decimal(15,2)"},
        False,
    ),
    ParityScenario(
        "dcs-info-missing-template",
        "unica.dcs.info",
        "dcs-info",
        "dcs-info.py",
        {"TemplatePath": "missing/Template.xml", "Mode": "overview"},
        False,
    ),
    ParityScenario(
        "dcs-validate-missing-template",
        "unica.dcs.validate",
        "dcs-validate",
        "dcs-validate.py",
        {"TemplatePath": "missing/Template.xml", "Detailed": True},
        False,
    ),
    ParityScenario(
        "mxl-decompile-missing-template",
        "unica.mxl.decompile",
        "mxl-decompile",
        "mxl-decompile.py",
        {"TemplatePath": "missing/Template.xml", "OutputPath": "out/mxl.json"},
        False,
    ),
    ParityScenario(
        "mxl-info-missing-template",
        "unica.mxl.info",
        "mxl-info",
        "mxl-info.py",
        {"TemplatePath": "missing/Template.xml", "Format": "text"},
        False,
    ),
    ParityScenario(
        "mxl-validate-missing-template",
        "unica.mxl.validate",
        "mxl-validate",
        "mxl-validate.py",
        {"TemplatePath": "missing/Template.xml"},
        False,
    ),
    ParityScenario(
        "role-info-missing-rights",
        "unica.role.info",
        "role-info",
        "role-info.py",
        {"RightsPath": "missing/Rights.xml"},
        False,
    ),
    ParityScenario(
        "role-validate-missing-rights",
        "unica.role.validate",
        "role-validate",
        "role-validate.py",
        {"RightsPath": "missing/Rights.xml"},
        False,
    ),
]

SCENARIOS = tuple(SUCCESS_SCENARIOS + VALIDATION_FAILURE_SCENARIOS + MISSING_INPUT_SCENARIOS)
MIN_NATIVE_PARITY_COVERAGE = 1.0

NATIVE_PARITY_TOOLS = {
    "unica.cf.edit",
    "unica.cf.info",
    "unica.cf.init",
    "unica.cf.validate",
    "unica.cfe.borrow",
    "unica.cfe.init",
    "unica.cfe.diff",
    "unica.cfe.patch_method",
    "unica.cfe.validate",
    "unica.form.validate",
    "unica.meta.compile",
    "unica.meta.edit",
    "unica.meta.info",
    "unica.meta.remove",
    "unica.meta.validate",
    "unica.help.add",
    "unica.form.add",
    "unica.form.compile",
    "unica.form.edit",
    "unica.form.info",
    "unica.form.remove",
    "unica.form.validate",
    "unica.subsystem.compile",
    "unica.subsystem.edit",
    "unica.subsystem.info",
    "unica.subsystem.validate",
    "unica.interface.edit",
    "unica.interface.validate",
    "unica.template.add",
    "unica.template.remove",
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.mxl.info",
    "unica.mxl.validate",
    "unica.role.compile",
    "unica.role.info",
    "unica.role.validate",
}

MUTATING_FORM_DCS_PARITY_TOOLS = {
    "unica.form.add",
    "unica.form.compile",
    "unica.form.edit",
    "unica.form.remove",
    "unica.dcs.compile",
    "unica.dcs.edit",
}

EXPECTED_TOOLS = {
    "unica.cf.edit",
    "unica.cf.info",
    "unica.cf.init",
    "unica.cf.validate",
    "unica.cfe.borrow",
    "unica.cfe.diff",
    "unica.cfe.init",
    "unica.cfe.patch_method",
    "unica.cfe.validate",
    "unica.meta.compile",
    "unica.meta.edit",
    "unica.meta.info",
    "unica.meta.remove",
    "unica.meta.validate",
    "unica.help.add",
    "unica.form.add",
    "unica.form.compile",
    "unica.form.edit",
    "unica.form.info",
    "unica.form.remove",
    "unica.form.validate",
    "unica.interface.edit",
    "unica.interface.validate",
    "unica.subsystem.compile",
    "unica.subsystem.edit",
    "unica.subsystem.info",
    "unica.subsystem.validate",
    "unica.template.add",
    "unica.template.remove",
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.mxl.info",
    "unica.mxl.validate",
    "unica.role.compile",
    "unica.role.info",
    "unica.role.validate",
}

BSP_PARITY_REQUIRED_TOOLS = {
    "unica.cf.info",
    "unica.cf.validate",
    "unica.cfe.borrow",
    "unica.meta.info",
    "unica.meta.validate",
    "unica.form.info",
    "unica.form.validate",
    "unica.form.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
    "unica.dcs.edit",
    "unica.mxl.info",
    "unica.mxl.validate",
    "unica.mxl.decompile",
    "unica.mxl.compile",
    "unica.role.info",
    "unica.role.validate",
    "unica.subsystem.info",
    "unica.subsystem.validate",
    "unica.interface.validate",
    "unica.interface.edit",
    "unica.template.add",
    "unica.template.remove",
}

BSP_MUTATING_REQUIRED_TOOLS = {
    "unica.cfe.borrow",
    "unica.form.edit",
    "unica.dcs.edit",
    "unica.mxl.compile",
    "unica.interface.edit",
    "unica.template.add",
    "unica.template.remove",
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


def dcs_edit_operations_in_args(arguments: dict[str, Any]) -> set[str]:
    operation = arguments.get("Operation") or arguments.get("operation")
    return {operation} if isinstance(operation, str) and operation else set()


class UnicaMcpScriptParityTests(unittest.TestCase):
    unica_bin: Path

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

    def test_cfe_patch_method_parity_uses_only_the_supported_v1_contract(self) -> None:
        scenarios = [
            scenario
            for scenario in SUCCESS_SCENARIOS
            if scenario.tool == "unica.cfe.patch_method"
        ]
        self.assertGreater(len(scenarios), 0)
        for scenario in scenarios:
            with self.subTest(scenario=scenario.name):
                self.assertIn(
                    scenario.arguments.get("InterceptorType"),
                    {"Before", "After"},
                )
                self.assertFalse(scenario.arguments.get("IsFunction", False))
                self.assertEqual(
                    scenario.arguments.get("MethodName"),
                    "ОбновитьДанные",
                    "the fixture exposes this caller-verified zero-parameter procedure",
                )
                self.assertTrue(
                    any(step.tool == "unica.cfe.borrow" for step in scenario.setup_steps),
                    "the target must be registered and adopted through the public borrow tool",
                )
                self.assertTrue(
                    any(
                        fixture.source
                        == "cfe-patch-method/base-common-module.bsl"
                        for fixture in scenario.fixtures
                    ),
                    "the base source must prove the documented procedure signature",
                )

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
        used_sources = {fixture.source for scenario in SCENARIOS for fixture in scenario.fixtures}
        self.assertEqual(manifest_sources - used_sources, set())

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

    def test_cf_edit_child_object_round_trip_preserves_bsp_configuration_bytes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-issue55-bsp-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            cache = temp_root / "cache"
            (workspace / "src" / "Catalogs").mkdir(parents=True)
            config_path = workspace / "src" / "Configuration.xml"
            shutil.copyfile(FIXTURES_ROOT / BSP_CF_CONFIGURATION_FIXTURE, config_path)
            shutil.copyfile(
                FIXTURES_ROOT / BSP_META_CATALOG_FIXTURE,
                workspace / "src" / "Catalogs" / "Валюты.xml",
            )
            before = config_path.read_bytes()

            remove = self.call_mcp_tool(
                "unica.cf.edit",
                {
                    "ConfigPath": "src/Configuration.xml",
                    "Operation": "remove-childObject",
                    "Value": "Catalog.Валюты",
                    "NoValidate": True,
                },
                workspace,
                cache,
            )
            self.assertTrue(remove["ok"], json.dumps(remove, ensure_ascii=False, indent=2))

            add = self.call_mcp_tool(
                "unica.cf.edit",
                {
                    "ConfigPath": "src/Configuration.xml",
                    "Operation": "add-childObject",
                    "Value": "Catalog.Валюты",
                    "NoValidate": True,
                },
                workspace,
                cache,
            )
            self.assertTrue(add["ok"], json.dumps(add, ensure_ascii=False, indent=2))

            validate = self.call_mcp_tool(
                "unica.cf.validate",
                {
                    "ConfigPath": "src/Configuration.xml",
                    "Detailed": True,
                    "MaxErrors": 20,
                },
                workspace,
                cache,
            )
            self.assertTrue(validate["ok"], json.dumps(validate, ensure_ascii=False, indent=2))
            self.assertEqual(config_path.read_bytes(), before)

    def test_form_edit_rejects_invalid_platform_event_without_writing(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-issue77-form-events-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            cache = temp_root / "cache"
            workspace.mkdir()
            form_path = workspace / "Form.xml"
            form_path.write_text(
                """<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
      xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"
      xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
\t<AutoCommandBar name="FormCommandBar" id="-1"/>
\t<ChildItems/>
\t<Attributes>
\t\t<Attribute name="Object" id="1">
\t\t\t<Type><v8:Type>cfg:DataProcessorObject.EventProbe</v8:Type></Type>
\t\t\t<MainAttribute>true</MainAttribute>
\t\t</Attribute>
\t</Attributes>
\t<Commands/>
</Form>""",
                encoding="utf-8",
            )
            definition_path = workspace / "invalid-events.json"
            shutil.copyfile(
                FIXTURES_ROOT / "form-edit" / "invalid-events.json",
                definition_path,
            )
            before = form_path.read_bytes()

            result = self.call_mcp_tool(
                "unica.form.edit",
                {
                    "FormPath": "Form.xml",
                    "JsonPath": "invalid-events.json",
                },
                workspace,
                cache,
            )

            self.assertFalse(result["ok"], json.dumps(result, ensure_ascii=False, indent=2))
            self.assertIn("FORM_EVENT_NOT_ALLOWED", "\n".join(result.get("errors", [])))
            self.assertEqual(result.get("changes"), [])
            self.assertEqual(form_path.read_bytes(), before)

    def test_form_edit_accepts_extended_persistent_event_families(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-issue77-persistent-events-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            cache = temp_root / "cache"
            workspace.mkdir()
            persistent_types = [
                "ChartOfAccountsObject.Main",
                "ChartOfCalculationTypesObject.Payroll",
                "AccumulationRegisterRecordSet.Stock",
                "AccountingRegisterRecordSet.Accounting",
                "CalculationRegisterRecordSet.Payroll",
            ]

            for index, persistent_type in enumerate(persistent_types, start=1):
                with self.subTest(persistent_type=persistent_type):
                    form_path = workspace / f"Form{index}.xml"
                    form_path.write_text(
                        f"""<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
      xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"
      xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
\t<AutoCommandBar name="FormCommandBar" id="-1"/>
\t<ChildItems/>
\t<Attributes>
\t\t<Attribute name="Object" id="1">
\t\t\t<Type><v8:Type>cfg:{persistent_type}</v8:Type></Type>
\t\t\t<MainAttribute>true</MainAttribute>
\t\t</Attribute>
\t</Attributes>
\t<Commands/>
</Form>""",
                        encoding="utf-8",
                    )

                    edit = self.call_mcp_tool(
                        "unica.form.edit",
                        {
                            "FormPath": form_path.name,
                            "definition": {
                                "formEvents": [
                                    {"name": "OnReadAtServer", "handler": "ObjectOnReadAtServer"}
                                ]
                            },
                        },
                        workspace,
                        cache,
                    )

                    self.assertTrue(edit["ok"], json.dumps(edit, ensure_ascii=False, indent=2))
                    updated = form_path.read_text(encoding="utf-8-sig")
                    self.assertEqual(updated.count('name="OnReadAtServer"'), 1)

                    validate = self.call_mcp_tool(
                        "unica.form.validate",
                        {"FormPath": form_path.name},
                        workspace,
                        cache,
                    )
                    self.assertTrue(
                        validate["ok"], json.dumps(validate, ensure_ascii=False, indent=2)
                    )

    def test_form_compile_dry_run_uses_event_registry_without_writing(self) -> None:
        with tempfile.TemporaryDirectory(prefix="unica-issue77-compile-preview-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            cache = temp_root / "cache"
            workspace.mkdir()
            (workspace / "src" / "cf").mkdir(parents=True)
            (workspace / "v8project.yaml").write_text(
                "format: DESIGNER\nsource-set:\n  main:\n    type: CONFIGURATION\n    path: src/cf\n",
                encoding="utf-8",
            )
            shutil.copyfile(
                FIXTURES_ROOT / "meta-remove" / "Configuration.xml",
                workspace / "src" / "cf" / "Configuration.xml",
            )
            invalid_definition = workspace / "invalid.json"
            valid_definition = workspace / "valid.json"
            invalid_definition.write_text(
                json.dumps({"events": {"Opening": "OnOpening"}}),
                encoding="utf-8",
            )
            valid_definition.write_text(
                json.dumps({"events": {"OnCreateAtServer": "OnCreateAtServer"}}),
                encoding="utf-8",
            )
            invalid_output = workspace / "src" / "cf" / "InvalidForm.xml"
            valid_output = workspace / "src" / "cf" / "ValidForm.xml"
            invalid_before = (
                b'<?xml version="1.0" encoding="UTF-8"?>\n'
                b'<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>\n'
            )
            valid_before = invalid_before
            invalid_output.write_bytes(invalid_before)
            valid_output.write_bytes(valid_before)
            messages = [
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "unica.form.compile",
                        "arguments": {
                            "cwd": str(workspace),
                            "JsonPath": "invalid.json",
                            "OutputPath": "src/cf/InvalidForm.xml",
                            "dryRun": True,
                        },
                    },
                },
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": "unica.form.compile",
                        "arguments": {
                            "cwd": str(workspace),
                            "JsonPath": "valid.json",
                            "OutputPath": "src/cf/ValidForm.xml",
                            "dryRun": True,
                        },
                    },
                },
            ]

            responses = self.call_mcp_messages(messages, cache)
            invalid = json.loads(responses[1]["result"]["content"][0]["text"])
            valid = json.loads(responses[2]["result"]["content"][0]["text"])

            self.assertFalse(invalid["ok"], json.dumps(invalid, ensure_ascii=False, indent=2))
            self.assertIn("FORM_EVENT_NOT_ALLOWED", "\n".join(invalid.get("errors", [])))
            self.assertEqual(invalid.get("changes"), [])
            self.assertTrue(valid["ok"], json.dumps(valid, ensure_ascii=False, indent=2))
            self.assertTrue(
                any("would update" in change and "ValidForm.xml" in change for change in valid["changes"]),
                json.dumps(valid, ensure_ascii=False, indent=2),
            )
            self.assertEqual(invalid_output.read_bytes(), invalid_before)
            self.assertEqual(valid_output.read_bytes(), valid_before)

    def test_bsp_dcs_edit_parity_covers_documented_operations(self) -> None:
        covered = set()
        for scenario in SCENARIOS:
            if not (
                scenario.tool == "unica.dcs.edit"
                and scenario.expect_ok
                and scenario.compare_files
            ):
                continue
            covered.update(dcs_edit_operations_in_args(scenario.arguments))

        self.assertEqual(covered & DCS_EDIT_REQUIRED_OPS, DCS_EDIT_REQUIRED_OPS)

    def test_every_skill_tools_call_example_executes_as_mcp_dry_run(self) -> None:
        examples = list(iter_skill_mcp_examples())
        self.assertGreater(len(examples), 0)

        with tempfile.TemporaryDirectory(prefix="unica-skill-example-mcp-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            workspace.mkdir()
            (workspace / "src" / "cf").mkdir(parents=True)
            (workspace / "v8project.yaml").write_text(
                "format: DESIGNER\nsource-set:\n  main:\n    type: CONFIGURATION\n    path: src/cf\n",
                encoding="utf-8",
            )
            shutil.copyfile(
                FIXTURES_ROOT / "meta-remove" / "Configuration.xml",
                workspace / "src" / "cf" / "Configuration.xml",
            )
            for example in examples:
                arguments = example.payload["params"]["arguments"]
                if example.skill == "form-edit":
                    form_path = workspace / arguments["FormPath"]
                    form_path.parent.mkdir(parents=True, exist_ok=True)
                    form_path.write_text(
                        """<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
\t<AutoCommandBar name="FormCommandBar" id="-1"/>
\t<ChildItems/>
\t<Attributes/>
\t<Commands/>
</Form>
""",
                        encoding="utf-8",
                    )
                    json_path = workspace / arguments["JsonPath"]
                    json_path.parent.mkdir(parents=True, exist_ok=True)
                    json_path.write_text("{}\n", encoding="utf-8")
                elif example.skill == "form-compile":
                    output_path = (
                        workspace
                        / "src"
                        / "cf"
                        / "Catalogs"
                        / "SkillExample"
                        / "Forms"
                        / f"Form{example.line}"
                        / "Ext"
                        / "Form.xml"
                    )
                    arguments["OutputPath"] = str(output_path.relative_to(workspace))
                    if arguments.get("FromObject") is True:
                        object_path = workspace / "src" / "cf" / "Catalogs" / "Валюты.xml"
                        object_path.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(FIXTURES_ROOT / BSP_META_CATALOG_FIXTURE, object_path)
                        arguments["ObjectPath"] = str(object_path.relative_to(workspace))
                        arguments["Purpose"] = "Item"
                    else:
                        json_path = workspace / "fixtures" / f"form-{example.line}.json"
                        json_path.parent.mkdir(parents=True, exist_ok=True)
                        json_path.write_text("{}\n", encoding="utf-8")
                        arguments["JsonPath"] = str(json_path.relative_to(workspace))
                elif example.skill == "code-patch":
                    module_path = workspace / arguments["path"]
                    module_path.parent.mkdir(parents=True, exist_ok=True)
                    module_path.write_text(
                        """Процедура ПриСозданииНаСервере()\n
    Сообщить(\"Готово\");\n
КонецПроцедуры\n""",
                        encoding="utf-8",
                    )
                    owner_directory = module_path.parent.parent
                    descriptor_path = (
                        owner_directory.parent / f"{owner_directory.name}.xml"
                    )
                    descriptor_path.write_text(
                        "<MetaDataObject/>\n", encoding="utf-8"
                    )
            messages = [
                dry_run_message_for_example(example, index + 1, workspace)
                for index, example in enumerate(examples)
            ]
            responses = self.call_mcp_messages(messages, temp_root / "cache")

        self.assertEqual(len(responses), len(examples))
        for example, message in zip(examples, messages):
            with self.subTest(skill=example.skill, line=example.line):
                response = responses[message["id"]]
                self.assertNotIn("error", response)
                result = json.loads(response["result"]["content"][0]["text"])
                self.assertTrue(result["ok"], json.dumps(result, ensure_ascii=False, indent=2))
                self.assertIn("dry run", result["summary"])

    def test_mcp_calls_match_unica_reference_models(self) -> None:
        for scenario in SCENARIOS:
            with self.subTest(scenario=scenario.name, tool=scenario.tool):
                self.assert_parity(scenario)

    def test_every_donor_case_has_one_reviewed_relation(self) -> None:
        cases = {case.case_id for case in iter_cc_1c_skill_cases()}
        relations = load_donor_relations()
        self.assertEqual(set(relations), cases)

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

            direct = run_unica_reference_model(scenario.skill, scenario.script, scenario.arguments, direct_ws)
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
                        command_for_script(scenario.skill, scenario.script, scenario.arguments),
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
            else:
                result = run_unica_reference_model(step.skill, step.script, step.arguments, workspace)
                if result.returncode != 0:
                    raise AssertionError(
                        f"setup step {step.skill}/{step.script} failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                    )

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
                target = workspace / write_file["path"]
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
            args = cc_step_raw_args(step.get("args") or {}, workspace, pre_input)
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
        arguments["dryRun"] = False
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

    def run_mcp_messages(
        self,
        messages: list[dict[str, Any]],
        env: dict[str, str],
    ) -> list[dict[str, Any]]:
        process = subprocess.Popen(
            [str(self.unica_bin)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            cwd=REPO_ROOT,
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
        deadline = time.monotonic() + 30
        try:
            for message in messages:
                process.stdin.write(json.dumps(message, ensure_ascii=False) + "\n")
            process.stdin.flush()

            responses = []
            for _ in messages:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail("timed out waiting for MCP response")
                try:
                    line = lines.get(timeout=remaining)
                except queue.Empty:
                    self.fail("timed out waiting for MCP response")
                if not line:
                    self.fail("MCP process exited before all responses arrived")
                responses.append(json.loads(line))

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
    ) -> dict[int, dict[str, Any]]:
        env = os.environ.copy()
        env["UNICA_PLUGIN_ROOT"] = str(PLUGIN_ROOT)
        env["UNICA_CACHE_DIR"] = str(cache_dir)
        responses = []
        for start in range(0, len(messages), 32):
            batch = messages[start : start + 32]
            responses.extend(self.run_mcp_messages(batch, env))
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


CC_CASE_TOOLS = {
    "meta-compile": "unica.meta.compile",
    # Kept until the first pristine refresh removes the adapted local alias.
    "dcs-compile": "unica.dcs.compile",
    "skd-compile": "unica.dcs.compile",
    "form-compile": "unica.form.compile",
    "form-compile-from-object": "unica.form.compile",
    "cfe-borrow": "unica.cfe.borrow",
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


def load_donor_relations() -> dict[str, dict[str, Any]]:
    registry = donor_contract.load_json(DONOR_RELATIONS_PATH)
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
        value = cc_mapping_value(mapping, case.case_data, workspace, input_file)
        if value is CC_OMIT:
            continue
        arguments[key] = value

    for key, value in cc_args_extra(case.case_data.get("args_extra") or [], workspace).items():
        arguments[key] = value
    return arguments, input_file


CC_OMIT = object()


def cc_mapping_value(
    mapping: dict[str, Any],
    case_data: dict[str, Any],
    workspace: Path,
    input_file: Path | None,
) -> Any:
    source = mapping["from"]
    if source == "inputFile":
        if input_file is None:
            return CC_OMIT
        return input_file.as_posix()
    if source == "workDir":
        return "."
    if source == "outputPath":
        return cc_workspace_path(workspace, case_data.get("outputPath") or "")
    if source == "workPath":
        field = mapping.get("field") or "objectPath"
        raw = case_data.get("params", {}).get(field, case_data.get(field))
        if raw in (None, ""):
            return CC_OMIT if mapping.get("optional") else "."
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


def cc_args_extra(args_extra: list[Any], workspace: Path) -> dict[str, Any]:
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
            value = value.replace("{workDir}", workspace.as_posix())
        result[key] = value
        index += 2
    return result


def cc_step_raw_args(args_map: dict[str, Any], workspace: Path, input_file: Path | None) -> list[str]:
    args: list[str] = []
    for flag, raw_value in args_map.items():
        args.append(flag)
        if raw_value is True or raw_value == "":
            continue
        value = str(raw_value).replace("{workDir}", workspace.as_posix())
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


def iter_skill_mcp_examples() -> list[SkillMcpExample]:
    examples: list[SkillMcpExample] = []
    for skill_doc in sorted(SKILLS_ROOT.glob("*/SKILL.md")):
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
                    skill=skill_doc.parent.name,
                    line=line,
                    payload=payload,
                )
            )
    return examples


def dry_run_message_for_example(
    example: SkillMcpExample,
    request_id: int,
    workspace: Path,
) -> dict[str, Any]:
    message = json.loads(json.dumps(example.payload, ensure_ascii=False))
    message["id"] = request_id
    message["jsonrpc"] = "2.0"
    params = message.setdefault("params", {})
    arguments = params.setdefault("arguments", {})
    arguments["cwd"] = str(workspace)
    arguments["dryRun"] = True
    return message


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


class WindowsParityNormalizationTests(unittest.TestCase):
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
