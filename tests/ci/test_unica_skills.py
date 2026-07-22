from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


IN_SCOPE_TOOLS = {
    "cf-edit": "unica.cf.edit",
    "cf-info": "unica.cf.info",
    "cf-init": "unica.cf.init",
    "cf-validate": "unica.cf.validate",
    "cfe-borrow": "unica.cfe.borrow",
    "cfe-diff": "unica.cfe.diff",
    "cfe-init": "unica.cfe.init",
    "cfe-patch-method": "unica.cfe.patch_method",
    "cfe-validate": "unica.cfe.validate",
    "epf-init": "unica.epf.init",
    "erf-init": "unica.erf.init",
    "meta-compile": "unica.meta.compile",
    "meta-edit": "unica.meta.edit",
    "meta-info": "unica.meta.info",
    "meta-remove": "unica.meta.remove",
    "meta-validate": "unica.meta.validate",
    "form-add": "unica.form.add",
    "form-compile": "unica.form.compile",
    "form-edit": "unica.form.edit",
    "form-info": "unica.form.info",
    "form-remove": "unica.form.remove",
    "form-validate": "unica.form.validate",
    "help-add": "unica.help.add",
    "interface-edit": "unica.interface.edit",
    "interface-validate": "unica.interface.validate",
    "subsystem-compile": "unica.subsystem.compile",
    "subsystem-edit": "unica.subsystem.edit",
    "subsystem-info": "unica.subsystem.info",
    "subsystem-validate": "unica.subsystem.validate",
    "template-add": "unica.template.add",
    "template-remove": "unica.template.remove",
    "dcs-compile": "unica.dcs.compile",
    "dcs-edit": "unica.dcs.edit",
    "dcs-info": "unica.dcs.info",
    "dcs-validate": "unica.dcs.validate",
    "mxl-compile": "unica.mxl.compile",
    "mxl-decompile": "unica.mxl.decompile",
    "mxl-info": "unica.mxl.info",
    "mxl-validate": "unica.mxl.validate",
    "role-compile": "unica.role.compile",
    "role-info": "unica.role.info",
    "role-validate": "unica.role.validate",
}

SCENARIO_SKILLS = {
    "api-design": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.grep",
        "unica.code.diagnostics",
        "unica.project.map",
        "unica.subsystem.info",
        "unica.meta.info",
        "unica.meta.profile",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "code-search": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.outline",
        "unica.code.grep",
        "unica.meta.profile",
        "unica.project.map",
    ],
    "code-diagnostics": [
        "unica.code.diagnostics",
        "unica.code.search",
        "unica.standards.explain",
        "unica.standards.search",
        "unica.runtime.execute",
    ],
    "code-review": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.outline",
        "unica.code.grep",
        "unica.code.diagnostics",
        "unica.meta.profile",
        "unica.standards.explain",
        "unica.standards.search",
        "unica.project.map",
        "unica.runtime.execute",
    ],
    "query-optimize": [
        "unica.code.search",
        "unica.code.outline",
        "unica.code.grep",
        "unica.dcs.info",
        "unica.meta.info",
        "unica.meta.profile",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "test-authoring": [
        "unica.code.search",
        "unica.project.map",
        "unica.runtime.execute",
    ],
    "platform-help": [
        "unica.standards.search",
        "unica.standards.explain",
        "unica.code.search",
        "unica.project.map",
        "unica.runtime.execute",
    ],
    "bsp-patterns": [
        "unica.code.search",
        "unica.meta.info",
        "unica.form.info",
        "unica.role.info",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "integration-implement": [
        "unica.project.map",
        "unica.meta.info",
        "unica.meta.compile",
        "unica.meta.edit",
        "unica.code.search",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "autonomous-server": [
        "unica.project.map",
        "unica.runtime.execute",
        "unica.meta.info",
        "unica.code.search",
        "unica.code.diagnostics",
    ],
    "log-analysis": [
        "unica.code.search",
        "unica.meta.info",
        "unica.project.map",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
    ],
    "background-jobs": [
        "unica.project.map",
        "unica.code.search",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "data-exchange": [
        "unica.project.map",
        "unica.code.search",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "db-performance": [
        "unica.project.map",
        "unica.code.search",
        "unica.code.outline",
        "unica.code.grep",
        "unica.meta.info",
        "unica.meta.profile",
        "unica.dcs.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "security-auth-crypto": [
        "unica.project.map",
        "unica.code.search",
        "unica.meta.info",
        "unica.role.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "data-separation": [
        "unica.project.map",
        "unica.code.search",
        "unica.meta.info",
        "unica.role.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "release-support": [
        "unica.project.map",
        "unica.code.search",
        "unica.cf.info",
        "unica.cfe.diff",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
}

SCENARIO_REQUIRED_TOKENS = {
    "api-design": [
        "483",
        "543",
        "551",
        "553",
        "644",
        "Программный интерфейс",
        "Служебный программный интерфейс",
        "Переопределяемый интерфейс",
        "Устаревшие процедуры и функции",
        "API-first",
    ],
    "code-search": ["MCP-first", "what was tried"],
    "code-diagnostics": ["АПК", "EDT", "BSL LS", "отключ", "v8std"],
    "code-review": ["Findings first", "severity", "file/line"],
    "query-optimize": ["СКД", "virtual", "query-in-loop"],
    "test-authoring": ['"testRunner": "yaxunit"', '"testRunner": "va"'],
    "platform-help": ["Unica MCP contract gap", "method signatures"],
    "bsp-patterns": ["БСП", "СведенияОВнешнейОбработке"],
    "integration-implement": ["HTTP-сервис", "webhook", "secrets"],
    "autonomous-server": ["HTTP-сервис", "веб-клиент", "external browser-testing tool"],
    "log-analysis": ["журнала регистрации", "технологического журнала", "ЖР", "ТЖ"],
    "background-jobs": ["Фоновые", "регламентные", "idempotency", "retry"],
    "data-exchange": ["планы обмена", "РИБ", "регистрация изменений", "контракт обмена"],
    "db-performance": ["SQL/DBMS trace", "индексы", "блокировки", "TEMPDB/WAL"],
    "security-auth-crypto": ["OpenID", "сертификаты", "CryptoPro", "секреты"],
    "data-separation": ["tenant-boundaries", "RLS", "разделители", "безопасные запросы"],
    "release-support": ["сравнение/объединение", "Поставка", "поддержка", "совместимость"],
}

REPLACED_RUNTIME_SKILLS = {
    "db-create",
    "db-list",
    "db-dump-xml",
    "db-dump-cf",
    "db-load-xml",
    "db-load-cf",
    "db-load-git",
    "db-update",
    "db-run",
    "workspace-init",
    "epf-build",
    "epf-dump",
    "epf-validate",
    "erf-build",
    "erf-dump",
    "erf-validate",
}

TASK_EXAMPLE_ARGUMENT_KEYS = {
    "cf-edit": ["ConfigPath", "Operation", "Value"],
    "cf-info": ["ConfigPath"],
    "cf-init": ["Name", "OutputDir"],
    "cf-validate": ["ConfigPath"],
    "cfe-borrow": ["ExtensionPath", "ConfigPath", "Object"],
    "cfe-diff": ["ExtensionPath", "ConfigPath"],
    "cfe-init": ["Name", "OutputDir"],
    "cfe-patch-method": ["ExtensionPath", "ModulePath", "MethodName"],
    "cfe-validate": ["ExtensionPath"],
    "epf-init": ["Name", "OutputDir", "FormName"],
    "erf-init": ["Name", "OutputDir", "FormName"],
    "meta-compile": ["JsonPath", "OutputDir"],
    "meta-edit": ["ObjectPath", "Operation", "Value"],
    "meta-info": ["ObjectPath"],
    "meta-remove": ["ConfigDir", "Object"],
    "meta-validate": ["ObjectPath"],
    "form-add": ["ObjectPath", "FormName", "Purpose"],
    "form-compile": ["JsonPath", "OutputPath"],
    "form-edit": ["FormPath", "JsonPath"],
    "form-info": ["FormPath"],
    "form-remove": ["ObjectName", "FormName", "SrcDir"],
    "form-validate": ["FormPath"],
    "help-add": ["ObjectName", "Lang", "SrcDir"],
    "interface-edit": ["CIPath", "Operation", "Value"],
    "interface-validate": ["CIPath"],
    "subsystem-compile": ["Value", "OutputDir"],
    "subsystem-edit": ["SubsystemPath", "Operation", "Value"],
    "subsystem-info": ["SubsystemPath"],
    "subsystem-validate": ["SubsystemPath"],
    "template-add": ["ObjectName", "TemplateName", "TemplateType", "SrcDir"],
    "template-remove": ["ObjectName", "TemplateName", "SrcDir"],
    "dcs-compile": ["DefinitionFile", "OutputPath"],
    "dcs-edit": ["TemplatePath", "Operation", "Value"],
    "dcs-info": ["TemplatePath"],
    "dcs-validate": ["TemplatePath"],
    "mxl-compile": ["JsonPath", "OutputPath"],
    "mxl-decompile": ["TemplatePath", "OutputPath"],
    "mxl-info": ["TemplatePath", "WithText"],
    "mxl-validate": ["TemplatePath"],
    "role-compile": ["JsonPath", "OutputDir"],
    "role-info": ["RightsPath"],
    "role-validate": ["RightsPath"],
}

SCENARIO_PRESERVING_MIN_MCP_CALLS = {
    "cf-edit": 6,
    "cf-info": 6,
    "cf-init": 6,
    "cf-validate": 2,
    "cfe-borrow": 7,
    "cfe-diff": 3,
    "cfe-init": 6,
    "cfe-patch-method": 4,
    "cfe-validate": 2,
    "meta-edit": 11,
    "meta-info": 14,
    "meta-remove": 6,
    "meta-validate": 2,
    "form-add": 6,
    "form-compile": 4,
    "form-validate": 2,
    "interface-edit": 8,
    "interface-validate": 2,
    "subsystem-compile": 4,
    "subsystem-edit": 6,
    "subsystem-info": 8,
    "subsystem-validate": 2,
    "template-add": 2,
    "dcs-compile": 5,
    "dcs-info": 12,
    "dcs-validate": 2,
    "mxl-info": 6,
    "mxl-validate": 2,
    "role-info": 2,
    "dcs-edit": 4,
    "role-compile": 3,
}

ALLOWED_ADDITIONAL_MCP_TOOL_NAMES = {
    "cf-init": {"unica.cf.info", "unica.cf.validate"},
    "cfe-borrow": {"unica.cfe.validate"},
    "cfe-init": {"unica.cfe.validate"},
    "epf-init": {"unica.runtime.execute"},
    "erf-init": {"unica.runtime.execute"},
    "form-compile": {"unica.form.info", "unica.form.validate"},
    "interface-edit": {"unica.interface.validate"},
    "meta-edit": {"unica.meta.info", "unica.meta.validate"},
    "role-compile": {"unica.role.info", "unica.role.validate"},
    "dcs-compile": {"unica.dcs.info", "unica.dcs.validate"},
    "dcs-edit": {"unica.dcs.info", "unica.dcs.validate"},
}

SCENARIO_PRESERVING_TOKENS = {
    "cf-edit": [
        '"Operation": "modify-property"',
        '"Value": "Version=1.0.0.1 ;; Vendor=Фирма 1С"',
        '"Operation": "add-childObject"',
        '"Operation": "remove-childObject"',
        '"Operation": "add-defaultRole"',
        '"Operation": "set-defaultRoles"',
    ],
    "cf-info": [
        '"Mode": "brief"',
        '"Mode": "full"',
        '"Limit": 50',
        '"Offset": 100',
        '"Section": "home-page"',
    ],
    "cf-init": [
        '"Name": "МояКонфигурация"',
        '"Version": "1.0.0.1"',
        '"Vendor": "Фирма 1С"',
        '"CompatibilityMode": "Version8_3_27"',
        '"name": "unica.cf.info"',
        '"name": "unica.cf.validate"',
    ],
    "cfe-borrow": [
        '"Object": "Catalog.Контрагенты"',
        '"Object": "Catalog.Контрагенты.Form.ФормаЭлемента"',
        '"Object": "Catalog.Контрагенты ;; CommonModule.ОбщийМодуль ;; Enum.ВидыОплат"',
        '"BorrowMainAttribute": true',
        '"BorrowMainAttribute": "All"',
        '"name": "unica.cfe.validate"',
    ],
    "cfe-diff": ['"Mode": "A"', '"Mode": "B"'],
    "cfe-init": [
        '"ConfigPath": "C:\\\\WS\\\\tasks\\\\cfsrc\\\\erp_8.3.24"',
        '"Purpose": "Patch"',
        '"CompatibilityMode": "Version8_3_17"',
        '"Version": "1.0.0.1"',
        '"NamePrefix": "ИБ_"',
        '"NoRole": true',
        '"name": "unica.cfe.validate"',
    ],
    "cfe-patch-method": [
        '"InterceptorType": "Before"',
        '"InterceptorType": "After"',
        '"Context": "НаКлиенте"',
        '"InterceptorType": "ModificationAndControl"',
        '"IsFunction": true',
    ],
    "meta-edit": [
        '"Value": "Комментарий: Строка(200) ;; Сумма: Число(15,2) | index"',
        '"Value": "Значение: Строка + Число(15,2) + Дата + CatalogRef.Контрагенты"',
        '"Operation": "add-ts"',
        '"Value": "Товары: Ном: CatalogRef.Ном | req, Кол: Число(15,3), Цена: Число(15,2)"',
        '"Operation": "remove-attribute"',
        '"Operation": "modify-attribute"',
        '"Operation": "modify-property"',
        '"Operation": "set-owners"',
        '"Value": "Catalog.Контрагенты ;; Catalog.Организации"',
        '"name": "unica.meta.validate"',
        '"name": "unica.meta.info"',
    ],
    "meta-info": [
        '"ObjectPath": "Catalogs/Валюты/Валюты.xml"',
        '"ObjectPath": "Documents/АвансовыйОтчет/АвансовыйОтчет.xml"',
        '"Name": "Товары"',
        '"ObjectPath": "HTTPServices/ExternalAPI/ExternalAPI.xml"',
        '"Name": "TestConnection"',
        '"ObjectPath": "DefinedTypes/GLN/GLN.xml"',
    ],
    "meta-remove": [
        '"Object": "Catalog.Устаревший"',
        '"dryRun": true',
        '"Force": true',
        '"KeepFiles": true',
        '"Object": "CommonModule.МойМодуль"',
    ],
    "form-add": [
        '"ObjectPath": "Documents/АвансовыйОтчет.xml"',
        '"Purpose": "List"',
        '"Purpose": "Record"',
        '"Purpose": "Choice"',
        '"Synonym": "Выбор номенклатуры"',
        '"SetDefault": true',
    ],
    "form-compile": [
        '"OutputPath": "<.../TypePlural/ObjectName/Forms/FormName/Ext/Form.xml>"',
        '"name": "unica.form.validate"',
        '"name": "unica.form.info"',
    ],
    "interface-edit": [
        '"Operation": "hide"',
        '"Operation": "show"',
        '"Operation": "place"',
        '"Operation": "subsystem-order"',
        '"CreateIfMissing": true',
        '"name": "unica.interface.validate"',
    ],
    "subsystem-compile": [
        '"Value": "{\\"name\\":\\"Тест\\"}"',
        'CommonPicture.Продажи',
        '"Parent": "config/Subsystems/Продажи.xml"',
    ],
    "subsystem-edit": [
        '"Operation": "add-content"',
        '"Operation": "remove-content"',
        '"Operation": "add-child"',
        '"Operation": "set-property"',
    ],
    "subsystem-info": [
        '"Mode": "content"',
        '"Name": "Document"',
        '"Mode": "ci"',
        '"Mode": "tree"',
        '"Name": "Администрирование"',
    ],
    "template-add": [
        '"TemplateType": "DataCompositionSchema"',
        '"SrcDir": "src/cfe/МоёРасширение/Reports"',
        '"SetMainSKD": true',
    ],
    "role-compile": [
        '"name": "unica.role.validate"',
        '"name": "unica.role.info"',
    ],
    "dcs-compile": [
        '"DefinitionFile": "<json>"',
        '"Value": "<json-string>"',
        '"name": "unica.dcs.validate"',
        '"name": "unica.dcs.info"',
        '"Mode": "variant"',
    ],
    "dcs-edit": [
        '"Operation": "add-field"',
        '"Value": "Цена: decimal(15,2) ;; Количество: decimal(15,3) ;; Сумма: decimal(15,2)"',
        '"name": "unica.dcs.validate"',
        '"name": "unica.dcs.info"',
    ],
    "dcs-info": [
        '"Mode": "query"',
        '"Name": "НоменклатураСЦенами"',
        '"Batch": 3',
        '"Mode": "fields"',
        '"Mode": "calculated"',
        '"Mode": "resources"',
        '"Mode": "trace"',
        '"Mode": "variant"',
        '"Mode": "templates"',
        '"Name": "ВидНалоговойБазы"',
        '"Mode": "trace"',
    ],
    "mxl-info": [
        '"ProcessorName": "<Имя>"',
        '"TemplateName": "<Макет>"',
        '"WithText": true',
        '"Format": "json"',
        '"MaxParams": 20',
        '"Offset": 150',
    ],
    "role-info": ['"OutFile": "<output.txt>"', '"Offset": 150'],
}


class UnicaSkillRoutingTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def skill_root(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "skills"

    def reference_root(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "references"

    def parity_reference_root(self) -> Path:
        return (
            self.repo_root()
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "reference_skills"
        )

    def test_in_scope_skills_route_to_single_unica_mcp(self) -> None:
        for skill, tool_name in IN_SCOPE_TOOLS.items():
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertIn("## MCP routing", text)
                self.assertIn("MCP `unica`", text)
                self.assertIn(tool_name, text)
                self.assertNotIn("unica-coder", text)
                self.assertNotIn("unica-v8-runner", text)
                self.assertNotIn("unica-bsl-workspace", text)
                self.assertNotIn("unica-rlm-tools-bsl", text)
                self.assertNotIn("unica-v8std", text)

    def test_scenario_skills_cover_requested_unica_workflows(self) -> None:
        for skill, tool_names in SCENARIO_SKILLS.items():
            with self.subTest(skill=skill):
                path = self.skill_root() / skill / "SKILL.md"
                self.assertTrue(path.is_file())
                text = path.read_text(encoding="utf-8")
                self.assertIn(f"name: {skill}", text)
                self.assertRegex(text, r"(?m)^description:\s+")
                self.assertIn("## MCP routing", text)
                self.assertIn("MCP `unica`", text)
                for tool_name in tool_names:
                    self.assertIn(tool_name, text)
                for token in SCENARIO_REQUIRED_TOKENS.get(skill, []):
                    self.assertIn(token, text)

    def test_unica_owned_guidance_contains_required_operational_concepts(self) -> None:
        docs = {
            "code-search": self.skill_root() / "code-search" / "SKILL.md",
            "code-diagnostics": self.skill_root() / "code-diagnostics" / "SKILL.md",
            "test-authoring": self.skill_root() / "test-authoring" / "SKILL.md",
            "background-jobs": self.skill_root() / "background-jobs" / "SKILL.md",
            "db-performance": self.skill_root() / "db-performance" / "SKILL.md",
            "integration-implement": self.skill_root() / "integration-implement" / "SKILL.md",
            "platform-mechanics": self.reference_root() / "platform" / "platform-mechanics.md",
            "runtime-diagnostics": self.reference_root() / "platform" / "runtime-diagnostics.md",
            "db-performance-ref": self.reference_root() / "platform" / "db-performance.md",
            "integration-contracts": self.reference_root() / "platform" / "integration-contracts.md",
        }
        joined = "\n".join(path.read_text(encoding="utf-8") for path in docs.values())

        for token in [
            "MCP-first",
            "what was tried",
            "verification gate",
            "impact analysis",
            "managed locks",
            "lock order",
            "structured logging",
            "DCS",
            "idempotency key",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, joined)

    def test_all_skills_do_not_expose_internal_mcp_names(self) -> None:
        forbidden = [
            "unica-coder",
            "unica-v8-runner",
            "unica-bsl-reference",
            "unica-bsl-workspace",
            "unica-rlm-tools-bsl",
            "unica-v8std",
        ]
        for skill_path in self.skill_root().glob("*/SKILL.md"):
            with self.subTest(skill=skill_path.parent.name):
                text = skill_path.read_text(encoding="utf-8")
                for name in forbidden:
                    self.assertNotIn(name, text)

    def test_skills_and_references_do_not_instruct_direct_rlm_mcp_calls(self) -> None:
        forbidden = ["rlm_index", "rlm_start", "rlm_execute", "rlm_end"]
        docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        for doc in docs:
            text = doc.read_text(encoding="utf-8")
            for token in forbidden:
                with self.subTest(path=doc.relative_to(self.repo_root()), token=token):
                    self.assertNotIn(token, text)

    def test_v8_runner_replaces_runtime_and_external_skills_with_single_mcp_skill(self) -> None:
        skill_dir = self.skill_root() / "v8-runner"
        self.assertTrue((skill_dir / "SKILL.md").is_file())
        for skill in REPLACED_RUNTIME_SKILLS:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill).exists())

        scanned_docs = [
            self.repo_root() / "README.md",
            self.repo_root() / "plugins" / "unica" / "README.md",
            self.reference_root() / "README.md",
            self.reference_root() / "tooling" / "v8project.md",
            self.reference_root() / "tooling" / "runtime-build.md",
            self.reference_root() / "use-cases" / "workspace-runtime.md",
            self.reference_root() / "use-cases" / "forms-ui.md",
            self.reference_root() / "use-cases" / "reports-printing.md",
        ]
        for doc in scanned_docs:
            text = doc.read_text(encoding="utf-8")
            for skill in REPLACED_RUNTIME_SKILLS:
                with self.subTest(path=doc.relative_to(self.repo_root()), skill=skill):
                    self.assertNotIn(f"/{skill}", text)
                    self.assertNotIn(f"`{skill}`", text)

        for doc in skill_dir.glob("**/*.md"):
            with self.subTest(path=doc.relative_to(skill_dir)):
                text = doc.read_text(encoding="utf-8")
                self.assertNotIn("run-v8-runner.sh", text)
                self.assertNotIn("unica-v8-runner", text)
                self.assertNotIn('"args"', text)
        self.assertIn(
            "unica.runtime.execute",
            (skill_dir / "SKILL.md").read_text(encoding="utf-8"),
        )

    def test_v8_runner_examples_are_parameterized_mcp_calls(self) -> None:
        skill_doc = self.skill_root() / "v8-runner" / "SKILL.md"
        text = skill_doc.read_text(encoding="utf-8")
        examples = [
            block
            for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
            if '"method": "tools/call"' in block
        ]
        self.assertGreaterEqual(len(examples), 20)
        operations = set()
        for block in examples:
            payload = json.loads(block)
            self.assertEqual(payload["params"]["name"], "unica.runtime.execute")
            arguments = payload["params"]["arguments"]
            self.assertIn("operation", arguments)
            self.assertNotEqual(set(arguments.keys()), {"cwd"})
            self.assertNotIn("args", arguments)
            operations.add(arguments["operation"])

        self.assertTrue(
            {
                "config-init",
                "init",
                "build",
                "dump",
                "convert",
                "make",
                "load",
                "syntax",
                "test",
                "launch",
                "extensions",
            }.issubset(operations)
        )
        self.assertIn('"sourceSet": "external-processors"', text)
        self.assertIn('"sourceSet": "external-reports"', text)
        self.assertIn('"output": "build/external"', text)

    def test_v8_runner_metadata_describes_runtime_trigger_surface(self) -> None:
        skill_doc = self.skill_root() / "v8-runner" / "SKILL.md"
        text = skill_doc.read_text(encoding="utf-8")
        description = re.search(r"^description:\s*(.+)$", text, flags=re.M)
        self.assertIsNotNone(description)
        description_text = description.group(1)
        for token in [
            "информационная база",
            "v8project.yaml",
            "workspace",
            "source-set",
            "EPF/ERF",
            "CF/CFE",
            "syntax/tests/launch",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, description_text)
        self.assertIn("Не используй", description_text)
        self.assertIn("XML", description_text)

    def test_references_are_structured_by_unica_use_cases(self) -> None:
        reference_root = self.reference_root()
        self.assertFalse((reference_root / "cc-1c-skills").exists())
        self.assertFalse((reference_root / "ai-rules-1c").exists())

        required_paths = [
            "README.md",
            "use-cases/workspace-runtime.md",
            "use-cases/metadata-modeling.md",
            "use-cases/forms-ui.md",
            "use-cases/reports-printing.md",
            "use-cases/extensions-cfe.md",
            "use-cases/rights-access.md",
            "use-cases/autonomous-server-debug.md",
            "use-cases/code-quality-review.md",
            "use-cases/integrations.md",
            "specs/README.md",
            "platform/development-standards.md",
            "platform/platform-solutions.md",
            "platform/runtime-diagnostics.md",
            "platform/db-performance.md",
            "platform/integration-contracts.md",
            "platform/platform-mechanics.md",
            "tooling/v8project.md",
            "tooling/internal-package.md",
            "tooling/runtime-build.md",
        ]
        for relative_path in required_paths:
            with self.subTest(path=relative_path):
                path = reference_root / relative_path
                self.assertTrue(path.is_file())
                text = path.read_text(encoding="utf-8")
                if relative_path.startswith("use-cases/"):
                    self.assertIn("## When to use", text)
                    self.assertIn("## Primary path", text)

    def test_web_publish_skill_surface_is_replaced_by_autonomous_server(self) -> None:
        self.assertTrue((self.skill_root() / "autonomous-server" / "SKILL.md").is_file())
        for skill in ["web-publish", "web-info", "web-stop", "web-unpublish"]:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill).exists())

        docs = [
            self.repo_root() / "plugins" / "unica" / "README.md",
            self.reference_root() / "README.md",
            *self.skill_root().glob("*/SKILL.md"),
            *self.reference_root().glob("use-cases/*.md"),
        ]
        forbidden = [
            "web-publish",
            "web-info",
            "web-stop",
            "web-unpublish",
            "web-publication-testing.md",
        ]
        for doc in docs:
            text = doc.read_text(encoding="utf-8")
            for token in forbidden:
                with self.subTest(path=doc.relative_to(self.repo_root()), token=token):
                    self.assertNotIn(token, text)

    def test_reference_fixtures_track_upstream_runtime_portability_fixes(self) -> None:
        dcs_scripts = [
            self.parity_reference_root()
            / "dcs-edit"
            / "scripts"
            / "dcs-edit.py",
        ]
        for path in dcs_scripts:
            with self.subTest(path=path.relative_to(self.repo_root())):
                text = path.read_text(encoding="utf-8")
                self.assertIn("dcs-edit v1.28", text)
                self.assertIn("expr_start = esc_xml", text)
                self.assertIn("expr_end = esc_xml", text)
                self.assertNotRegex(text, r"<expression>\{esc_xml\('&' \+ param_name")

        subsystem_compile = (
            self.parity_reference_root()
            / "subsystem-compile"
            / "scripts"
            / "subsystem-compile.py"
        ).read_text(encoding="utf-8")
        self.assertIn("subsystem-compile v1.8", subsystem_compile)
        self.assertIn("import subprocess", subsystem_compile)
        self.assertIn("subsystem-validate.py", subsystem_compile)
        self.assertIn("subprocess.run([sys.executable, validate_script, \"-SubsystemPath\", target_xml])", subsystem_compile)
        self.assertNotIn("powershell.exe", subsystem_compile)
        self.assertNotIn("subsystem-validate.ps1", subsystem_compile)

    def test_dcs_skills_track_upstream_dsl_features_through_unica_boundary(self) -> None:
        dcs_compile = (self.skill_root() / "dcs-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        dcs_edit = (self.skill_root() / "dcs-edit" / "SKILL.md").read_text(encoding="utf-8")
        dcs_info = (self.skill_root() / "dcs-info" / "SKILL.md").read_text(encoding="utf-8")
        dcs_dsl = (self.reference_root() / "specs" / "dcs-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        dcs_spec = (self.reference_root() / "specs" / "1c-dcs-spec.md").read_text(
            encoding="utf-8"
        )

        for text in [dcs_compile, dcs_edit, dcs_info]:
            self.assertIn("MCP `unica`", text)
            self.assertNotIn("CLAUDE_SKILL_DIR", text)
            self.assertNotIn("powershell.exe", text)
            self.assertNotIn(".ps1", text)
            self.assertNotIn(".py", text)

        for token in [
            "TypeSet",
            "balanceGroupName",
            "orderExpression",
            "valueListAllowed",
            "availableValues",
            "dataSetLinks",
            "additionalProperties",
            "parameterListAllowed",
            "startExpression",
            "linkConditionExpression",
            "viewMode",
            "itemsViewMode",
            "use: false",
            "placement",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, dcs_dsl)

        self.assertIn("Значение-список", dcs_spec)
        self.assertIn("valueListAllowed", dcs_spec)
        self.assertIn('"Raw": true', dcs_info)
        self.assertIn("сырой текст запроса целиком", dcs_info)
        self.assertIn("unica.dcs.edit", dcs_info)
        self.assertIn("patch-query", dcs_edit)
        self.assertIn("@once", dcs_edit)
        self.assertIn("availableValue=", dcs_edit)
        self.assertIn("value=", dcs_edit)

    def test_form_skills_track_upstream_dsl_features_through_unica_boundary(self) -> None:
        form_compile = (self.skill_root() / "form-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        form_edit = (self.skill_root() / "form-edit" / "SKILL.md").read_text(encoding="utf-8")
        form_info = (self.skill_root() / "form-info" / "SKILL.md").read_text(encoding="utf-8")
        form_validate = (self.skill_root() / "form-validate" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        form_dsl = (self.reference_root() / "specs" / "form-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        form_patterns = (self.reference_root() / "specs" / "form-patterns.md").read_text(
            encoding="utf-8"
        )

        for text in [form_compile, form_edit, form_info, form_validate]:
            self.assertIn("MCP `unica`", text)
            self.assertNotIn("CLAUDE_SKILL_DIR", text)
            self.assertNotIn("powershell.exe", text)
            self.assertNotIn(".ps1", text)
            self.assertNotIn(".py", text)

        for token in [
            "mobileCommandBarContent",
            "reportResult",
            "reportFormType",
            "choiceParameters",
            "choiceParameterLinks",
            "availableTypes",
            "extendedTooltip",
            "commandBar",
            "contextMenu",
            "roles",
            "CommandInterface",
            "NavigationPanel",
            "GanttChart",
            "chart",
            "dynamicDataRead",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, form_dsl)

        self.assertIn("Связанные действия командной панели", form_patterns)
        self.assertIn("mobileCommandBarContent", form_compile)
        self.assertIn("choiceParameters", form_compile)
        self.assertIn("availableTypes", form_compile)
        self.assertIn("unica.form.info", form_edit)
        self.assertIn("unica.form.validate", form_edit)

    def test_form_patterns_ux_guidance_is_mirrored_and_uses_supported_dsl(self) -> None:
        heading = "## UX-правила для элементов и компоновки форм"
        legacy_heading = "## UX-правила для элементов форм"

        def ux_section(path: Path) -> str:
            text = path.read_text(encoding="utf-8")
            start = text.index(heading if heading in text else legacy_heading)
            end = text.index("\n---", start)
            return text[start:end]

        reference_path = self.reference_root() / "specs" / "form-patterns.md"
        skill_path = self.skill_root() / "form-patterns" / "SKILL.md"
        reference_section = ux_section(reference_path)
        skill_section = ux_section(skill_path)

        self.assertIn(heading, reference_path.read_text(encoding="utf-8"))
        self.assertIn(heading, skill_path.read_text(encoding="utf-8"))
        self.assertEqual(skill_section, reference_section)
        self.assertIn(
            "https://github.com/Oxotka/1CDesignGuide/tree/edc05eaf5c191250a184b0e185006bf4b412f7a5",
            reference_section,
        )
        for token in [
            "Обычная группа",
            "прижатия элементов и заголовков к краю",
            "Сильное",
            "Обычное",
            "Слабое",
            "Сворачиваемая группа",
            "не отображайте отступ слева",
            '"showLeftMargin": false',
            "`collapsed` задаёт начальное состояние",
            "`Группа.Показать()`",
            "`Группа.Скрыть()`",
            "только в коде формы",
            "Всплывающая группа",
            "DSL пока не может настроить `ControlRepresentation`",
            "как подсказку",
            "подобно гиперссылке",
            "Командная панель",
            '"commandSource": "Form"',
            '"commandSource": "FormCommandPanelGlobalCommands"',
            '"commandName": "CommonCommand.ОткрытьПараметры"',
            "вручную устраните дубли",
            "не выдавайте `popup` или `buttonGroup` за исполнимые нативные элементы",
            "Команды формы",
            "Шапка формы",
            "функциональным опциям",
            "автозаполняются или сохраняют предыдущее значение",
            "изменяющее форму, ставьте первым",
            "Подвал формы",
            "Комментарий и Ответственный последними",
            "строковых полей с доступным выбором",
            '"choiceButton": true',
            "очевидных полей",
            '"titleLocation": "none"',
            '"titleLocation": "top"',
            '"inputHint": "По всем организациям"',
            '"showInHeader": false',
            '"readOnly": true',
            '"horizontalStretch": true',
            '"headerHorizontalAlign": "Right"',
            '"horizontalAlign": "Right"',
            "не используйте много разных стилей и цветов",
            "достаточно длинное название",
            "двойное отрицание",
            "Проводить документ при записи",
            '"tooltip": "Пояснение"',
            '"tooltipRepresentation": "Button"',
            '"checkBoxType": "switcher"',
            "3–5 значений",
            "на весь экран",
            "слева вверху",
            "модальной",
            "справа внизу",
            '"font": { "bold": true }',
            '"backColor": "#FFFF00"',
        ]:
            with self.subTest(token=token):
                self.assertIn(token, reference_section)

        self.assertEqual(reference_section.count("defaultButton"), 1)
        self.assertNotIn("buttonHint", reference_section)
        self.assertNotIn("RGB(", reference_section)
        self.assertNotRegex(reference_section, r'"radio"\s*:')
        self.assertNotRegex(reference_section, r'"(?:popup|buttonGroup)"\s*:')
        self.assertNotRegex(reference_section, r'"(?:leftIndent|showLeftIndent)"\s*:')
        self.assertNotIn("нет нативного DSL-ключа для левого отступа", reference_section)
        self.assertNotIn('"representation": "Picture"', reference_section)
        self.assertNotIn("Кнопки действий внизу", reference_section)

    def test_form_dsl_keeps_tooltip_and_command_binding_contracts_unambiguous(self) -> None:
        form_dsl = (self.reference_root() / "specs" / "form-dsl-spec.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("Обычный `<Title>` поля и `<ToolTip>`", form_dsl)
        self.assertRegex(form_dsl, r"передавать им `\{text, formatted\}`\s+нельзя")
        self.assertIn("приоритетом `command` → `commandName` → `stdCommand`", form_dsl)
        self.assertIn("`popup` и `buttonGroup` зарезервированы", form_dsl)
        self.assertNotRegex(form_dsl, r'"(?:popup|buttonGroup)"\s*:')

    def test_meta_info_tracks_upstream_type_presentation_through_unica_boundary(self) -> None:
        meta_info = (self.skill_root() / "meta-info" / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("MCP `unica`", meta_info)
        self.assertIn("unica.meta.info", meta_info)
        self.assertIn("Представление типа", meta_info)
        self.assertIn("Представление объекта", meta_info)
        self.assertNotIn("CLAUDE_SKILL_DIR", meta_info)
        self.assertNotIn("powershell.exe", meta_info)
        self.assertNotIn(".ps1", meta_info)
        self.assertNotIn(".py", meta_info)

    def test_meta_compile_tracks_upstream_choice_history_through_unica_boundary(self) -> None:
        meta_compile = (self.skill_root() / "meta-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("MCP `unica`", meta_compile)
        self.assertIn("unica.meta.compile", meta_compile)
        self.assertIn("choiceHistoryOnInput", meta_compile)
        self.assertNotIn("CLAUDE_SKILL_DIR", meta_compile)
        self.assertNotIn("powershell.exe", meta_compile)
        self.assertNotIn(".ps1", meta_compile)
        self.assertNotIn(".py", meta_compile)

    def test_support_state_reporting_is_documented_for_info_skills(self) -> None:
        for skill in (
            "cf-info",
            "meta-info",
            "form-info",
            "dcs-info",
            "mxl-info",
            "role-info",
            "subsystem-info",
        ):
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertIn("Поддержка", text)
                self.assertIn("ParentConfigurations.bin", text)
                self.assertIn("unica.", text)
                self.assertNotIn("support-edit.py", text)
                self.assertNotIn("ParentConfigurations.bin` raw", text)

        release_support = (self.skill_root() / "release-support" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("support-state", release_support)
        self.assertIn("unica.cf.info", release_support)
        self.assertIn("unica.meta.info", release_support)

    def test_source_set_format_detection_contract_is_documented(self) -> None:
        docs = {
            "workspace-runtime": self.reference_root()
            / "use-cases"
            / "workspace-runtime.md",
            "metadata-modeling": self.reference_root()
            / "use-cases"
            / "metadata-modeling.md",
            "v8project": self.reference_root() / "tooling" / "v8project.md",
            "format-index": self.reference_root() / "specs" / "format-index.md",
            "invariants": self.repo_root() / "spec" / "architecture" / "invariants.md",
        }
        joined = "\n".join(path.read_text(encoding="utf-8") for path in docs.values())

        self.assertIn("unica.project.map", joined)
        self.assertIn("sourceSets[]", joined)
        self.assertIn("sourceFormat", joined)
        self.assertIn("platform_xml", joined)
        self.assertIn("EDT configuration", joined)
        self.assertIn("platform XML external", joined)
        self.assertIn("not of the whole workspace", joined)
        self.assertNotIn("sourceFormat=mixed", joined)
        self.assertNotIn("source_format=mixed", joined)

    def test_references_do_not_contain_stale_upstream_instructions(self) -> None:
        forbidden_patterns = [
            r"references/(cc-1c-skills|ai-rules-1c)",
            r"\bClaude\b",
            r"\bclaude\b",
            r"Anthropic",
            r"\.claude",
            r"/db-(?!performance\.md\b)",
            r"/epf-(init|build|dump|validate)",
            r"/erf-(init|build|dump|validate)",
            r"1c-code-metadata-mcp",
            r"1c-metadata-manage",
            r"deploy_and_test",
            r'"mode"\s*:\s*"update"',
        ]
        scanned_roots = [self.reference_root(), self.skill_root()]
        for root in scanned_roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                for pattern in forbidden_patterns:
                    with self.subTest(path=path.relative_to(self.repo_root()), pattern=pattern):
                        self.assertIsNone(re.search(pattern, text))

    def test_v8_runner_docs_track_current_v8project_contract(self) -> None:
        v8project = (self.reference_root() / "tooling" / "v8project.md").read_text(
            encoding="utf-8"
        )
        v8_runner_docs = "\n".join(
            path.read_text(encoding="utf-8")
            for path in [
                self.skill_root() / "v8-runner" / "SKILL.md",
                self.skill_root()
                / "v8-runner"
                / "references"
                / "config-and-backends.md",
                self.skill_root()
                / "v8-runner"
                / "references"
                / "command-selection.md",
                self.skill_root() / "v8-runner" / "references" / "testing.md",
                self.reference_root() / "tooling" / "v8project.md",
            ]
        )

        self.assertIn("execution_timeout", v8project)
        self.assertIn("infobase:", v8project)
        self.assertIn("infobase.connection", v8_runner_docs)
        self.assertIn("tools-download", v8_runner_docs)
        self.assertIn("fullOutput", v8_runner_docs)
        self.assertIn("features", v8_runner_docs)
        self.assertNotRegex(v8project, r"(?m)^connection:")
        self.assertNotIn("mode=load|merge|update", v8project)

    def test_v8_runner_dump_references_keep_unsafe_modes_preview_only(self) -> None:
        v8_runner_root = self.skill_root() / "v8-runner"
        safety_context = re.compile(
            r"dryRun.{0,8}(?:true|`true`)|preview|read-only|fail-closed|block",
            re.IGNORECASE | re.DOTALL,
        )
        paths = sorted(
            [*v8_runner_root.rglob("*.md"), *self.reference_root().rglob("*.md")]
        )

        for path in paths:
            text = path.read_text(encoding="utf-8")
            for match in re.finditer(r"mode=(?:incremental|partial)", text):
                context = text[max(0, match.start() - 240) : match.end() + 240]
                with self.subTest(
                    path=path.relative_to(self.repo_root()), mode=match.group(0)
                ):
                    self.assertRegex(context, safety_context)

        for path in v8_runner_root.rglob("*.md"):
            text = path.read_text(encoding="utf-8")
            for block in re.findall(r"```json\s*(.*?)```", text, re.DOTALL):
                payload = json.loads(block)
                arguments = payload.get("params", {}).get("arguments", {})
                if (
                    arguments.get("operation") == "dump"
                    and arguments.get("mode") in {"incremental", "partial"}
                ):
                    with self.subTest(
                        path=path.relative_to(self.repo_root()),
                        mode=arguments["mode"],
                    ):
                        self.assertIs(arguments.get("dryRun"), True)

    def test_config_dump_info_version_is_documented_as_opaque_platform_state(self) -> None:
        configuration_spec = (
            self.reference_root() / "specs" / "1c-configuration-spec.md"
        ).read_text(encoding="utf-8")

        self.assertIn("`configVersion` — непрозрачное значение платформы", configuration_spec)
        self.assertNotRegex(configuration_spec, r"`configVersion`\s*\|\s*Хеш версии")

    def test_config_dump_info_docs_preserve_same_named_metadata_source(self) -> None:
        docs = [
            self.skill_root() / "v8-runner" / "SKILL.md",
            self.reference_root() / "tooling" / "v8project.md",
            self.reference_root() / "use-cases" / "metadata-modeling.md",
        ]

        for path in docs:
            text = path.read_text(encoding="utf-8")
            with self.subTest(path=path.relative_to(self.repo_root())):
                self.assertIn("platform-generated CDFI sidecar", text)
                self.assertIn("legitimate metadata descriptor", text)
                self.assertIn("remains source", text)

    def test_skills_and_references_do_not_expose_restricted_research_sources(self) -> None:
        forbidden_patterns = [
            r"docs/its",
            r"\.pdf\b",
            r"Документация\.pdf",
            r"Методическая поддержка",
            r":: 1С:Предприятие",
        ]
        scanned_roots = [self.reference_root(), self.skill_root()]
        for root in scanned_roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                for pattern in forbidden_patterns:
                    with self.subTest(path=path.relative_to(self.repo_root()), pattern=pattern):
                        self.assertIsNone(re.search(pattern, text, flags=re.I))

    def test_documented_reference_paths_exist(self) -> None:
        roots = [
            self.repo_root() / "README.md",
            self.repo_root() / "plugins" / "unica" / "README.md",
            *self.skill_root().glob("*/SKILL.md"),
            *self.reference_root().rglob("*.md"),
        ]
        pattern = re.compile(r"`(references/[^`]+?\.md)`")
        for doc in roots:
            text = doc.read_text(encoding="utf-8")
            for match in pattern.findall(text):
                with self.subTest(doc=doc.relative_to(self.repo_root()), reference=match):
                    local_target = doc.parent / match
                    plugin_target = self.repo_root() / "plugins" / "unica" / match
                    self.assertTrue(local_target.is_file() or plugin_target.is_file())

    def test_skills_do_not_use_model_specific_assistant_names(self) -> None:
        forbidden = ["Claude", "claude", "Anthropic", ".claude", "CLAUDE.md"]
        for skill_doc in self.skill_root().glob("*/**/*.md"):
            with self.subTest(path=skill_doc.relative_to(self.skill_root())):
                text = skill_doc.read_text(encoding="utf-8")
                for token in forbidden:
                    self.assertNotIn(token, text)

    def test_migrated_skills_do_not_reference_skill_local_operation_scripts(self) -> None:
        forbidden = [
            "powershell.exe",
            ".ps1",
            ".py",
            "Current Python/PowerShell scripts",
            "fallback implementation details",
            "Native execution path",
        ]
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                for token in forbidden:
                    self.assertNotIn(token, text)

    def test_migrated_skills_do_not_ship_skill_local_operation_scripts(self) -> None:
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill / "scripts").exists())

    def test_parity_reference_skills_are_test_only_donor_fixtures(self) -> None:
        reference_root = self.parity_reference_root()
        referenced_skills = {
            path.parent.parent.name for path in reference_root.glob("*/scripts/*.py")
        }
        self.assertEqual(
            referenced_skills,
            set(IN_SCOPE_TOOLS) - {"epf-init", "erf-init"},
        )
        allowed_suffixes = {".json", ".md", ".ps1", ".py"}
        for path in reference_root.rglob("*"):
            if path.is_file():
                with self.subTest(path=path.relative_to(reference_root)):
                    self.assertNotIn("__pycache__", path.parts)
                    self.assertIn(path.suffix, allowed_suffixes)

    def test_migrated_skill_verification_sections_use_mcp_examples(self) -> None:
        slash_command = re.compile(r"(?m)^/[a-z][a-z-]+\b")
        verification_section = re.compile(r"(?ms)^## Верификация\s*\n(.*?)(?=^## |\Z)")
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                match = verification_section.search(text)
                if match is None:
                    continue
                section = match.group(1)
                self.assertIsNone(slash_command.search(section))
                self.assertNotIn("powershell.exe", section)
                self.assertNotIn(".ps1", section)
                self.assertNotIn(".py", section)
                if "```" in section:
                    self.assertIn('"method": "tools/call"', section)

    def test_migrated_skills_use_task_parameterized_mcp_examples(self) -> None:
        generic_arguments = '"arguments": {\n      "cwd": "<workspace>"\n    }'
        for skill, tool_name in IN_SCOPE_TOOLS.items():
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertNotIn(generic_arguments, text)
                for key in TASK_EXAMPLE_ARGUMENT_KEYS[skill]:
                    self.assertIn(f'"{key}"', text)
                mcp_blocks = [
                    block
                    for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
                    if '"method": "tools/call"' in block
                ]
                self.assertGreater(len(mcp_blocks), 0)
                if skill in SCENARIO_PRESERVING_MIN_MCP_CALLS:
                    self.assertGreaterEqual(
                        len(mcp_blocks), SCENARIO_PRESERVING_MIN_MCP_CALLS[skill]
                    )
                for token in SCENARIO_PRESERVING_TOKENS.get(skill, []):
                    self.assertIn(token, text)
                for block in mcp_blocks:
                    payload = json.loads(block)
                    params = payload["params"]
                    allowed_tool_names = {
                        tool_name,
                        *ALLOWED_ADDITIONAL_MCP_TOOL_NAMES.get(skill, set()),
                    }
                    self.assertIn(params["name"], allowed_tool_names)
                    self.assertNotEqual(set(params["arguments"].keys()), {"cwd"})


if __name__ == "__main__":
    unittest.main()
