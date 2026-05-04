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
    "interface-edit": "unica.interface.edit",
    "interface-validate": "unica.interface.validate",
    "subsystem-compile": "unica.subsystem.compile",
    "subsystem-edit": "unica.subsystem.edit",
    "subsystem-info": "unica.subsystem.info",
    "subsystem-validate": "unica.subsystem.validate",
    "template-add": "unica.template.add",
    "template-remove": "unica.template.remove",
    "skd-compile": "unica.skd.compile",
    "skd-edit": "unica.skd.edit",
    "skd-info": "unica.skd.info",
    "skd-validate": "unica.skd.validate",
    "mxl-compile": "unica.mxl.compile",
    "mxl-decompile": "unica.mxl.decompile",
    "mxl-info": "unica.mxl.info",
    "mxl-validate": "unica.mxl.validate",
    "role-compile": "unica.role.compile",
    "role-info": "unica.role.info",
    "role-validate": "unica.role.validate",
}

OUT_OF_SCOPE = [
    "web-test",
    "help-add",
    "img-grid",
]

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
    "epf-init",
    "epf-build",
    "epf-dump",
    "epf-validate",
    "erf-init",
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
    "interface-edit": ["CIPath", "Operation", "Value"],
    "interface-validate": ["CIPath"],
    "subsystem-compile": ["Value", "OutputDir"],
    "subsystem-edit": ["SubsystemPath", "Operation", "Value"],
    "subsystem-info": ["SubsystemPath"],
    "subsystem-validate": ["SubsystemPath"],
    "template-add": ["ObjectName", "TemplateName", "TemplateType", "SrcDir"],
    "template-remove": ["ObjectName", "TemplateName", "SrcDir"],
    "skd-compile": ["DefinitionFile", "OutputPath"],
    "skd-edit": ["TemplatePath", "Operation", "Value"],
    "skd-info": ["TemplatePath"],
    "skd-validate": ["TemplatePath"],
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
    "skd-compile": 5,
    "skd-info": 12,
    "skd-validate": 2,
    "mxl-info": 6,
    "mxl-validate": 2,
    "role-info": 2,
    "skd-edit": 4,
    "role-compile": 3,
}

ALLOWED_ADDITIONAL_MCP_TOOL_NAMES = {
    "cf-init": {"unica.cf.info", "unica.cf.validate"},
    "cfe-borrow": {"unica.cfe.validate"},
    "cfe-init": {"unica.cfe.validate"},
    "form-compile": {"unica.form.info", "unica.form.validate"},
    "interface-edit": {"unica.interface.validate"},
    "meta-edit": {"unica.meta.info", "unica.meta.validate"},
    "role-compile": {"unica.role.info", "unica.role.validate"},
    "skd-compile": {"unica.skd.info", "unica.skd.validate"},
    "skd-edit": {"unica.skd.info", "unica.skd.validate"},
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
        '"FromObject": true',
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
    "skd-compile": [
        '"DefinitionFile": "<json>"',
        '"Value": "<json-string>"',
        '"name": "unica.skd.validate"',
        '"name": "unica.skd.info"',
        '"Mode": "variant"',
    ],
    "skd-edit": [
        '"Operation": "add-field"',
        '"Value": "Цена: decimal(15,2) ;; Количество: decimal(15,3) ;; Сумма: decimal(15,2)"',
        '"name": "unica.skd.validate"',
        '"name": "unica.skd.info"',
    ],
    "skd-info": [
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
            "use-cases/web-publication-testing.md",
            "use-cases/code-quality-review.md",
            "use-cases/integrations.md",
            "specs/README.md",
            "platform/development-standards.md",
            "platform/platform-solutions.md",
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
            r"/db-",
            r"/epf-(init|build|dump|validate)",
            r"/erf-(init|build|dump|validate)",
            r"1c-code-metadata-mcp",
            r"1c-metadata-manage",
            r"deploy_and_test",
        ]
        scanned_roots = [self.reference_root(), self.skill_root()]
        for root in scanned_roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                for pattern in forbidden_patterns:
                    with self.subTest(path=path.relative_to(self.repo_root()), pattern=pattern):
                        self.assertIsNone(re.search(pattern, text))

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

    def test_parity_reference_scripts_are_test_only_python(self) -> None:
        reference_root = self.parity_reference_root()
        referenced_skills = {
            path.parent.parent.name for path in reference_root.glob("*/scripts/*.py")
        }
        self.assertEqual(referenced_skills, set(IN_SCOPE_TOOLS))
        for path in reference_root.rglob("*"):
            if path.is_file():
                with self.subTest(path=path.relative_to(reference_root)):
                    self.assertEqual(path.suffix, ".py")

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
