from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "unica"
SKILLS_ROOT = PLUGIN_ROOT / "skills"
FIXTURES_ROOT = REPO_ROOT / "tests" / "fixtures" / "unica_mcp_script_parity"
REFERENCE_SKILLS_ROOT = FIXTURES_ROOT / "reference_skills"
BSP_SKD_QUERY_FIXTURE = (
    "bsp/skd/Catalogs__ПравилаОбработкиЭлектроннойПочты__"
    "СхемаПравилаОбработкиЭлектроннойПочты/Template.xml"
)
BSP_SKD_UNION_FIXTURE = (
    "bsp/skd/DataProcessors__ВыгрузкаЗагрузкаEnterpriseData__"
    "СхемаКомпоновкиДанных/Template.xml"
)
BSP_SKD_OBJECT_FIXTURE = (
    "bsp/skd/DataProcessors__ЗаменаИОбъединениеЭлементов__"
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
        name="cfe-patch-method-before",
        tool="unica.cfe.patch_method",
        skill="cfe-patch-method",
        script="cfe-patch-method.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ModulePath": "CommonModule.Server",
            "MethodName": "BeforeWrite",
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
                    "NoRole": True,
                },
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-patch-method-after-form",
        tool="unica.cfe.patch_method",
        skill="cfe-patch-method",
        script="cfe-patch-method.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ModulePath": "Document.Заказ.Form.ФормаДокумента",
            "MethodName": "ПослеЗаписиНаСервере",
            "InterceptorType": "After",
            "Context": "НаКлиенте",
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
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="cfe-patch-method-modification-function",
        tool="unica.cfe.patch_method",
        skill="cfe-patch-method",
        script="cfe-patch-method.py",
        arguments={
            "ExtensionPath": "src-cfe",
            "ModulePath": "CommonModule.ОбщийМодуль",
            "MethodName": "ПолучитьДанные",
            "InterceptorType": "ModificationAndControl",
            "IsFunction": True,
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
        ),
        fixtures=(
            FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),
            FileFixture("cf-edit/ops.json", "fixtures/cf-edit-ops.json"),
        ),
        expect_ok=True,
        compare_files=True,
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
        fixtures=(
            FileFixture("meta-remove/Configuration.xml", "src/Configuration.xml"),
            FileFixture("meta-remove/Catalogs/ParityCatalog.xml", "src/Catalogs/ParityCatalog.xml"),
            FileFixture(
                "meta-remove/Catalogs/ParityCatalog/Ext/ObjectModule.bsl",
                "src/Catalogs/ParityCatalog/Ext/ObjectModule.bsl",
            ),
            FileFixture("meta-remove/Subsystems/Sales.xml", "src/Subsystems/Sales.xml"),
            FileFixture(
                "meta-remove/Subsystems/Sales/Ext/CommandInterface.xml",
                "src/Subsystems/Sales/Ext/CommandInterface.xml",
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
        fixtures=(FileFixture("meta-catalog.json", "fixtures/meta-catalog.json"),),
        expect_ok=True,
        compare_files=True,
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
        fixtures=(FileFixture(BSP_META_CATALOG_FIXTURE, "src/Catalogs/Валюты.xml"),),
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
        fixtures=(
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
        fixtures=(FileFixture(BSP_META_REPORT_FIXTURE, "src/Reports/АнализВерсийОбъектов.xml"),),
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
        fixtures=(FileFixture(BSP_META_ENUM_FIXTURE, "src/Enums/ВажностьПроблемыУчета.xml"),),
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
        fixtures=(
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
        fixtures=(
            FileFixture("form-remove/ParityReport.xml", "src/Reports/ParityReport.xml"),
            FileFixture(
                "form-remove/ParityReport/Forms/MainForm.xml",
                "src/Reports/ParityReport/Forms/MainForm.xml",
            ),
            FileFixture(
                "form-remove/ParityReport/Forms/MainForm/Ext/Form.xml",
                "src/Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            ),
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
        fixtures=(FileFixture("template-remove/ParityReport.xml", "src/Reports/ParityReport.xml"),),
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
        fixtures=(
            FileFixture("template-remove/ParityReport.xml", "src/Reports/ParityReport.xml"),
            FileFixture(
                "template-remove/ParityReport/Templates/MainSchema.xml",
                "src/Reports/ParityReport/Templates/MainSchema.xml",
            ),
            FileFixture(
                "template-remove/ParityReport/Templates/MainSchema/Ext/Template.xml",
                "src/Reports/ParityReport/Templates/MainSchema/Ext/Template.xml",
            ),
        ),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-compile-simple",
        tool="unica.skd.compile",
        skill="skd-compile",
        script="skd-compile.py",
        arguments={
            "DefinitionFile": "fixtures/skd-simple.json",
            "OutputPath": "templates/SKD.xml",
        },
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-compile-bsp-data-usage",
        tool="unica.skd.compile",
        skill="skd-compile",
        script="skd-compile.py",
        arguments={
            "DefinitionFile": "fixtures/skd-bsp-data-usage.json",
            "OutputPath": "templates/SKD.xml",
        },
        fixtures=(FileFixture("skd-bsp-data-usage.json", "fixtures/skd-bsp-data-usage.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-info-overview-outfile",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Mode": "overview",
            "OutFile": "skd-info.txt",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-info-overview",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "overview", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-query",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОсновнойНаборДанных",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_SKD_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-query-named-nested-union",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОпределениеПолей",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-fields",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "fields", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-links",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "links", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-calculated",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "calculated", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-resources",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "resources", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-params",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "params", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-variant",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "variant", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-trace",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "trace",
            "Name": "КоличествоДанных",
            "Limit": 200,
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-templates",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "templates", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-info-full",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={"TemplatePath": "src/Template.xml", "Mode": "full", "Limit": 200},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="bsp-skd-validate-real-template-detailed",
        tool="unica.skd.validate",
        skill="skd-validate",
        script="skd-validate.py",
        arguments={"TemplatePath": "src/Template.xml", "Detailed": True, "MaxErrors": 80},
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
    ),
    ParityScenario(
        name="skd-validate-detailed-outfile",
        tool="unica.skd.validate",
        skill="skd-validate",
        script="skd-validate.py",
        arguments={
            "TemplatePath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
            "Detailed": True,
            "OutFile": "skd-validate.txt",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "src/Reports/ParityReport/Templates/Main/Ext/Template.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-modify-structure",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "modify-structure",
            "Value": "Price @name=G2",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "set-structure",
                    "Value": "Code @name=G1 > Quantity @name=G2 > details",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-selection-in-named-variant",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-selection",
            "Value": "Code",
            "Variant": "Alt",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "add-variant",
                    "Value": "Alt [Alt presentation]",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-selection-folder",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-selection",
            "Value": "Folder(Parity folder: Code, Quantity)",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-selection-to-named-structure-group",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-selection",
            "Value": "Quantity @group=G1",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "set-structure",
                    "Value": "Code @name=G1 > details",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-query",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "patch-query",
            "Value": "1 => 2 @once",
            "DataSet": "ОсновнойНаборДанных",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-query",
                    "Value": "ВЫБРАТЬ\n\t1 КАК Ссылка",
                    "DataSet": "ОсновнойНаборДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-set-query-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-query",
            "Value": "ВЫБРАТЬ\n\t2 КАК Ссылка",
            "DataSet": "ОсновнойНаборДанных",
        },
        fixtures=(FileFixture(BSP_SKD_QUERY_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-variant-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-variant",
            "Value": "ParityVariantFinal [Parity variant final]",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-calculated-field-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-calculated-field",
            "Value": "ParityCalcFinal: decimal(10,2) = КоличествоДанных + 1",
            "NoSelection": True,
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-modify-field-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "modify-field",
            "Value": "ПредставлениеДанных [Представление parity final]: string",
            "DataSet": "МестаИспользования",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-set-field-role-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-field-role",
            "Value": "ПредставлениеДанных @dimension",
            "DataSet": "МестаИспользования",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-order-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-order",
            "Value": "КоличествоДанных desc",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-clear-order-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-order",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-order",
                    "Value": "КоличествоДанных desc",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-clear-selection-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-selection",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-selection",
                    "Value": "КоличествоДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-clear-filter-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-filter",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-remove-filter-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-filter",
            "Value": "КоличествоДанных",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-data-parameter-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataParameter",
            "Value": "ДатаФормирования = LastMonth",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-data-set-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataSet",
            "Value": "ParityDataSetFinal: ВЫБРАТЬ 1 КАК КоличествоДанных",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-data-set-link-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-dataSetLink",
            "Value": "МестаИспользования > ParityDataSetFinal on КоличествоДанных = КоличествоДанных [param ParityLinkFinal]",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-set-output-parameter-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-outputParameter",
            "Value": "Заголовок = ParityTitleFinal",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-set-structure-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "set-structure",
            "Value": "Ссылка @name=ParityRootFinal > details",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-rename-parameter-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "rename-parameter",
            "Value": "ParityRenameParam => ParityRenameParamFinal",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ParityRenameParam",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-remove-field-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-field",
            "Value": "ParityRemoveField",
            "DataSet": "МестаИспользования",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityRemoveField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-remove-field-keeps_group_items",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-field",
            "Value": "ParityGroupedField",
            "DataSet": "МестаИспользования",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityGroupedField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-structure",
                    "Value": "ParityGroupedField @name=ParityGroupedRoot > details",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-conditional-appearance-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-conditionalAppearance",
            "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-clear-conditional-appearance-final",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "clear-conditionalAppearance",
            "Value": "*",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-conditionalAppearance",
                    "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-field-preserves_schema_order_and_role_markers",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-field",
            "Value": "Amount: decimal(10,2) @dimension #noFilter",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-total-aggregate-shorthand",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-total",
            "Value": "Amount: Сумма",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-parameter-typed-available-values",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-parameter",
            "Value": (
                "Period: StandardPeriod = LastMonth "
                "availableValue=LastMonth:Прошлый месяц, ThisMonth:Текущий месяц"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-add-parameter-quoted-value-list-and-available-values",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "add-parameter",
            "Value": (
                "Tags: string = \"one,two\", 'three:four' "
                "availableValue=\"one,two\":\"One, two\", 'three:four':'Three: four'"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-modify-parameter-preserves_typed_value_and_sets_available_values",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "modify-parameter",
            "Value": (
                "Period [Период] value=ThisMonth denyIncompleteValues=true use=Always "
                "availableValue=ThisMonth:Текущий месяц, LastMonth:Прошлый месяц"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "add-parameter",
                    "Value": "Period: StandardPeriod = LastMonth",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-modify-parameter-quoted-value-list-and-available-values",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "modify-parameter",
            "Value": (
                "Tags value=\"one,two\", 'three:four' "
                "availableValue=\"one,two\":\"One, two\", 'three:four':'Three: four'"
            ),
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "add-parameter",
                    "Value": "Tags: string = initial",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-modify-filter-preserves_existing_disabled_state",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "modify-filter",
            "Value": "Code >= 2",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "add-filter",
                    "Value": "Code = 1 @off",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="skd-edit-modify-data-parameter-preserves_existing_value",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "modify-dataParameter",
            "Value": "Period @off",
        },
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "add-dataParameter",
                    "Value": "Period = LastMonth @off",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-fields-and-resources",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-total",
            "Value": "ВременныйИтог",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-field",
                    "Value": "ПредставлениеДанных [Представление parity]: string",
                    "DataSet": "МестаИспользования",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-field-role",
                    "Value": "ПредставлениеДанных @dimension",
                    "DataSet": "МестаИспользования",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-total",
                    "Value": "ВременныйИтог: Сумма(КоличествоДанных)",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-params",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "reorder-parameters",
            "Value": "ПараметрParityПереименованный, ДатаФормирования, ПредставлениеСписка",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ПараметрParity",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-parameter",
                    "Value": "ПараметрParity [Параметр parity] @hidden @always",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "rename-parameter",
                    "Value": "ПараметрParity => ПараметрParityПереименованный",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-settings",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-drilldown",
            "Value": "КоличествоДанных",
            "Variant": "ParityVariant",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityVariant [Parity variant]",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-structure",
                    "Value": "Ссылка @name=ParityRoot > details",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-structure",
                    "Value": "Данные @name=ParityRoot",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "ВедущееИзмерение = false",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "remove-filter",
                    "Value": "ВедущееИзмерение",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-conditionalAppearance",
                    "Value": "ЦветТекста = web:Red when ВедущееИзмерение = false for КоличествоДанных",
                    "Variant": "ParityVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-conditionalAppearance",
                    "Value": "*",
                    "Variant": "ParityVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-add-filter",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-filter",
            "Value": "ВедущееИзмерение = false",
        },
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-calculated-field-lifecycle",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-calculated-field",
            "Value": "ParityCalc",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-calculated-field",
                    "Value": "ParityCalc: decimal(10,2) = КоличествоДанных + 1",
                    "NoSelection": True,
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-datasets-and-variant-params",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "modify-filter",
            "Value": "КоличествоДанных >= 2",
            "Variant": "ParityDataVariant",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityDataVariant [Parity data variant]",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataSet",
                    "Value": "ParityDataSet: ВЫБРАТЬ 1 КАК КоличествоДанных",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataSetLink",
                    "Value": "МестаИспользования > ParityDataSet on КоличествоДанных = КоличествоДанных [param ParityLink]",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "set-outputParameter",
                    "Value": "Заголовок = ParityTitle",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-dataParameter",
                    "Value": "ДатаФормирования = LastMonth",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "modify-dataParameter",
                    "Value": "ДатаФормирования = ThisMonth",
                    "Variant": "ParityDataVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "КоличествоДанных = 1",
                    "Variant": "ParityDataVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
        expect_ok=True,
        compare_files=True,
    ),
    ParityScenario(
        name="bsp-skd-edit-basic-ops-lifecycle",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "remove-parameter",
            "Value": "ParityCleanupParam",
        },
        setup_steps=(
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-variant",
                    "Value": "ParityOpsVariant [Parity ops variant]",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-field",
                    "Value": "ParityCleanupField: decimal(10,0)",
                    "DataSet": "МестаИспользования",
                    "NoSelection": True,
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-parameter",
                    "Value": "ParityCleanupParam",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-selection",
                    "Value": "ParityCleanupField",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-order",
                    "Value": "ParityCleanupField desc",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "add-filter",
                    "Value": "ParityCleanupField = 1",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-selection",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-order",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "clear-filter",
                    "Value": "*",
                    "Variant": "ParityOpsVariant",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                tool="unica.skd.edit",
                arguments={
                    "TemplatePath": "src/Template.xml",
                    "Operation": "remove-field",
                    "Value": "ParityCleanupField",
                    "DataSet": "МестаИспользования",
                    "Variant": "ParityOpsVariant",
                },
            ),
        ),
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
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
        name="skd-validate-bad-prefix-namespace",
        tool="unica.skd.validate",
        skill="skd-validate",
        script="skd-validate.py",
        arguments={"TemplatePath": "templates/BadPrefix.xml"},
        expect_ok=False,
        fixtures=(FileFixture("skd-validate/BadPrefix.xml", "templates/BadPrefix.xml"),),
    ),
    ParityScenario(
        name="skd-edit-patch-query-once-ambiguous",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "templates/SKD.xml",
            "Operation": "patch-query",
            "Value": "Code => ItemCode @once",
        },
        expect_ok=False,
        setup_steps=(
            SetupStep(
                skill="skd-compile",
                script="skd-compile.py",
                arguments={
                    "DefinitionFile": "fixtures/skd-simple.json",
                    "OutputPath": "templates/SKD.xml",
                },
            ),
            SetupStep(
                skill="skd-edit",
                script="skd-edit.py",
                arguments={
                    "TemplatePath": "templates/SKD.xml",
                    "Operation": "set-query",
                    "Value": "SELECT Code AS Code",
                },
            ),
        ),
        fixtures=(FileFixture("skd-simple.json", "fixtures/skd-simple.json"),),
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
        name="bsp-skd-info-query-named-union-fails",
        tool="unica.skd.info",
        skill="skd-info",
        script="skd-info.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Mode": "query",
            "Name": "ОбщееКоличествоЭлементов",
        },
        expect_ok=False,
        fixtures=(FileFixture(BSP_SKD_UNION_FIXTURE, "src/Template.xml"),),
    ),
    ParityScenario(
        name="bsp-skd-edit-missing-variant-fails",
        tool="unica.skd.edit",
        skill="skd-edit",
        script="skd-edit.py",
        arguments={
            "TemplatePath": "src/Template.xml",
            "Operation": "add-selection",
            "Value": "КоличествоДанных",
            "Variant": "DefinitelyMissingVariant",
        },
        expect_ok=False,
        fixtures=(FileFixture(BSP_SKD_OBJECT_FIXTURE, "src/Template.xml"),),
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
        "skd-edit-missing-template",
        "unica.skd.edit",
        "skd-edit",
        "skd-edit.py",
        {"TemplatePath": "missing/Template.xml", "Operation": "add-field", "Value": "Amount: decimal(15,2)"},
        False,
    ),
    ParityScenario(
        "skd-info-missing-template",
        "unica.skd.info",
        "skd-info",
        "skd-info.py",
        {"TemplatePath": "missing/Template.xml", "Mode": "overview"},
        False,
    ),
    ParityScenario(
        "skd-validate-missing-template",
        "unica.skd.validate",
        "skd-validate",
        "skd-validate.py",
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
    "unica.skd.compile",
    "unica.skd.edit",
    "unica.skd.info",
    "unica.skd.validate",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.mxl.info",
    "unica.mxl.validate",
    "unica.role.compile",
    "unica.role.info",
    "unica.role.validate",
}

MUTATING_FORM_SKD_PARITY_TOOLS = {
    "unica.form.add",
    "unica.form.compile",
    "unica.form.edit",
    "unica.form.remove",
    "unica.skd.compile",
    "unica.skd.edit",
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
    "unica.skd.compile",
    "unica.skd.edit",
    "unica.skd.info",
    "unica.skd.validate",
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
    "unica.skd.info",
    "unica.skd.validate",
    "unica.skd.edit",
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
    "unica.skd.edit",
    "unica.mxl.compile",
    "unica.interface.edit",
    "unica.template.add",
    "unica.template.remove",
}

SKD_EDIT_REQUIRED_OPS = {
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


def skd_edit_operations_in_args(arguments: dict[str, Any]) -> set[str]:
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
            covered_by_success_snapshot & MUTATING_FORM_SKD_PARITY_TOOLS,
            MUTATING_FORM_SKD_PARITY_TOOLS,
        )

    def test_native_parity_coverage_stays_above_required_threshold(self) -> None:
        covered = {scenario.tool for scenario in SCENARIOS if scenario.tool in NATIVE_PARITY_TOOLS}
        coverage = len(covered) / len(NATIVE_PARITY_TOOLS)
        self.assertGreaterEqual(coverage, MIN_NATIVE_PARITY_COVERAGE)
        self.assertEqual(NATIVE_PARITY_TOOLS - covered, set())

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

    def test_bsp_skd_edit_parity_covers_documented_operations(self) -> None:
        covered = set()
        for scenario in SCENARIOS:
            if not (
                scenario.tool == "unica.skd.edit"
                and scenario.expect_ok
                and scenario.compare_files
            ):
                continue
            covered.update(skd_edit_operations_in_args(scenario.arguments))

        self.assertEqual(covered & SKD_EDIT_REQUIRED_OPS, SKD_EDIT_REQUIRED_OPS)

    def test_every_skill_tools_call_example_executes_as_mcp_dry_run(self) -> None:
        examples = list(iter_skill_mcp_examples())
        self.assertGreater(len(examples), 0)

        with tempfile.TemporaryDirectory(prefix="unica-skill-example-mcp-") as temp:
            temp_root = Path(temp)
            workspace = temp_root / "workspace"
            workspace.mkdir()
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

    def test_mcp_calls_match_reference_python_scripts(self) -> None:
        for scenario in SCENARIOS:
            with self.subTest(scenario=scenario.name, tool=scenario.tool):
                self.assert_parity(scenario)

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

            direct = run_python_script(scenario.skill, scenario.script, scenario.arguments, direct_ws)
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
                result = run_python_script(step.skill, step.script, step.arguments, workspace)
                if result.returncode != 0:
                    raise AssertionError(
                        f"setup step {step.skill}/{step.script} failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
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
        result = subprocess.run(
            [str(self.unica_bin)],
            input=json.dumps(message, ensure_ascii=False) + "\n",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=REPO_ROOT,
            env=env,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        response_lines = [line for line in result.stdout.splitlines() if line.strip()]
        self.assertEqual(len(response_lines), 1, result.stdout)
        response = json.loads(response_lines[0])
        if "error" in response:
            raise AssertionError(json.dumps(response["error"], ensure_ascii=False, indent=2))
        return json.loads(response["result"]["content"][0]["text"])

    def call_mcp_messages(
        self,
        messages: list[dict[str, Any]],
        cache_dir: Path,
    ) -> dict[int, dict[str, Any]]:
        env = os.environ.copy()
        env["UNICA_PLUGIN_ROOT"] = str(PLUGIN_ROOT)
        env["UNICA_CACHE_DIR"] = str(cache_dir)
        result = subprocess.run(
            [str(self.unica_bin)],
            input="\n".join(json.dumps(message, ensure_ascii=False) for message in messages) + "\n",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=REPO_ROOT,
            env=env,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        return {response["id"]: response for response in responses}


def run_python_script(
    skill: str,
    script: str,
    arguments: dict[str, Any],
    workspace: Path,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command_for_script(skill, script, arguments),
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )


def command_for_script(skill: str, script: str, arguments: dict[str, Any]) -> list[str]:
    script_path = REFERENCE_SKILLS_ROOT / skill / "scripts" / script
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
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    normalized = normalized.replace(str(workspace.resolve()), "<WORKSPACE>")
    normalized = normalized.replace(str(workspace), "<WORKSPACE>")
    normalized = normalized.replace(str(REPO_ROOT), "<REPO>")
    normalized = re.sub(
        r"<REPO>/tests/fixtures/unica_mcp_script_parity/reference_skills/([^/\s\"']+)/scripts/([^/\s\"']+)",
        r"<REPO>/<SKILL_SCRIPT>/\1/\2",
        normalized,
    )
    normalized = UUID_RE.sub("<UUID>", normalized)
    return normalized


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
        snapshot[rel] = normalize_text(text, workspace)
    return snapshot


if __name__ == "__main__":
    unittest.main()
