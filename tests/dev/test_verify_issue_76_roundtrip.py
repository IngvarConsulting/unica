import copy
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dev/verify-issue-76-roundtrip.py"

CATALOG_METADATA_PATH = "Catalog.ЗависимостиСчетов"
MODULE_METADATA_PATH = (
    "CommonModule.СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер.Module"
)
CATALOG_RELATIVE_PATH = Path("src/Catalogs/ЗависимостиСчетов.xml")
MODULE_RELATIVE_PATH = Path(
    "src/CommonModules/"
    "СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер/Ext/Module.bsl"
)
PARENT_CONFIGURATION_RELATIVE_PATH = Path(
    "src/Ext/ParentConfigurations/УправлениеХолдингом.cf"
)
METADATA_MARKER = "UNICA_ISSUE_76_ROUND_TRIP"
BSL_MARKER = f"// {METADATA_MARKER}"


def load_verifier():
    if not SCRIPT.is_file():
        raise AssertionError(f"issue #76 verifier implementation is missing: {SCRIPT}")
    spec = importlib.util.spec_from_file_location(
        "verify_issue_76_roundtrip",
        SCRIPT,
    )
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load issue #76 verifier: {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def write_workspace(root: Path) -> Path:
    workspace = root / "private-workspace"
    source = workspace / "src"
    catalog = workspace / CATALOG_RELATIVE_PATH
    module = workspace / MODULE_RELATIVE_PATH
    catalog.parent.mkdir(parents=True)
    module.parent.mkdir(parents=True)
    (source / "Ext").mkdir(parents=True)
    (source / "Configuration.xml").write_text(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" "
        "version=\"2.20\"><Configuration "
        "uuid=\"11111111-1111-1111-1111-111111111111\">"
        "<Properties><Name>УправлениеХолдингом</Name></Properties>"
        "</Configuration></MetaDataObject>",
        encoding="utf-8",
    )
    catalog.write_text(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" "
        "version=\"2.20\"><Catalog "
        "uuid=\"73dc29de-fd29-481e-9f17-ed571894d270\">"
        "<Properties><Name>ЗависимостиСчетов</Name><Comment/>"
        "</Properties></Catalog></MetaDataObject>",
        encoding="utf-8",
    )
    common_module_descriptor = module.parents[1].with_suffix(".xml")
    common_module_descriptor.write_text(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" "
        "version=\"2.20\"><CommonModule "
        "uuid=\"65cddf39-104d-4dd1-b953-b99f8092546b\">"
        "<Properties><Name>"
        "СообщенияВСлужбуТехническойПоддержкиБПКлиентСервер"
        "</Name></Properties></CommonModule></MetaDataObject>",
        encoding="utf-8",
    )
    module.write_bytes(
        b"\xef\xbb\xbf"
        + (
            "Функция ЛинияПоддержки()\r\n"
            "\tВозврат Неопределено;\r\n"
            "КонецФункции"
        ).encode("utf-8")
    )
    (source / "Ext/ParentConfigurations.bin").write_text(
        "synthetic support fixture",
        encoding="utf-8",
    )
    (source / "ConfigDumpInfo.xml").write_text(
        "<ConfigDumpInfo/>",
        encoding="utf-8",
    )
    (workspace / "v8project.yaml").write_text(
        "format: DESIGNER\n"
        "builder: DESIGNER\n"
        "source-set:\n"
        "  - name: main\n"
        "    type: CONFIGURATION\n"
        "    path: src\n",
        encoding="utf-8",
    )
    return workspace


def write_packaged_manifest(plugin_root: Path, *, include_secrets: bool = False) -> None:
    third_party = plugin_root / "third-party"
    third_party.mkdir(parents=True)
    payload = {
        "schemaVersion": 2,
        "targetTriple": "aarch64-apple-darwin",
        "tools": [
            {
                "name": "unica",
                "version": "0.12.0",
                "sourceCommit": "workspace",
                "sourceTag": "workspace",
                "sha256": "a" * 64,
            },
            {
                "name": "v8-runner",
                "version": "0.5.1",
                "sourceCommit": "7ce1b062843d86644fe55741dbe0ee79f7ca767d",
                "sourceTag": "master",
                "sha256": "b" * 64,
            },
        ],
    }
    if include_secrets:
        payload["privateToken"] = "manifest-top-secret"
        payload["tools"][1]["password"] = "manifest-runner-secret"
    (third_party / "manifest.json").write_text(
        json.dumps(payload, ensure_ascii=False),
        encoding="utf-8",
    )


def write_gate_inputs(root: Path) -> dict:
    fixture = write_workspace(root / "fixture")
    database = root / "database"
    database.mkdir()
    (database / "1Cv8.1CD").write_bytes(b"synthetic infobase")
    plugin = root / "plugin"
    write_packaged_manifest(plugin)
    platform = root / "platform"
    platform.mkdir()
    binary = root / "unica"
    binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    binary.chmod(0o700)
    parent_configuration = root / "1cv8.cf"
    parent_configuration.write_bytes(b"synthetic vendor configuration")
    return {
        "binary": binary,
        "binary_args": [],
        "plugin_root": plugin,
        "database": database,
        "sources": fixture / "src",
        "parent_configuration": parent_configuration,
        "platform_path": platform,
        "platform_version": "8.3.27.2214",
        "report_path": root / "report.json",
        "builder": "DESIGNER",
        "db_user": "Администратор",
        "timeout_seconds": 60,
        "execute": True,
        "allow_empty_password": True,
    }


class ScriptedClient:
    """Small public-tool fake; it never launches 1C or an MCP process."""

    def __init__(
        self,
        workspace: Path,
        *,
        mutate_on_partial: bool = False,
        lose_after_full_dump: str | None = None,
        full_dump_noop: bool = False,
        cdfi_build_changes: set[int] | None = None,
        diagnostic: str = "",
    ) -> None:
        self.workspace = workspace
        self.mutate_on_partial = mutate_on_partial
        self.lose_after_full_dump = lose_after_full_dump
        self.full_dump_noop = full_dump_noop
        self.cdfi_build_changes = cdfi_build_changes or set()
        self.build_count = 0
        self.database_catalog: bytes | None = None
        self.database_module: bytes | None = None
        self.diagnostic = diagnostic
        self.calls: list[tuple[str, dict]] = []

    def call(self, name: str, arguments: dict, **_kwargs) -> dict:
        arguments = copy.deepcopy(arguments)
        self.calls.append((name, arguments))

        if name == "unica.cf.info":
            return {
                "ok": True,
                "data": {"support": {"editingEnabled": True}},
                "errors": [],
            }
        if name == "unica.support.edit":
            return {"ok": True, "data": {"changed": True}, "errors": []}
        if name == "unica.meta.edit":
            if arguments.get("dryRun") is False:
                catalog = self.workspace / CATALOG_RELATIVE_PATH
                catalog.write_text(
                    catalog.read_text(encoding="utf-8").replace(
                        "<Comment/>",
                        f"<Comment>{METADATA_MARKER}</Comment>",
                    ),
                    encoding="utf-8",
                )
            return {
                "ok": True,
                "data": {
                    "changed": True,
                    "validation": {"status": "passed"},
                },
                "errors": [],
            }
        if name == "unica.code.patch":
            module = self.workspace / MODULE_RELATIVE_PATH
            before = module.read_bytes()
            after = before.replace(
                "Функция ЛинияПоддержки()".encode("utf-8"),
                (BSL_MARKER + "\r\nФункция ЛинияПоддержки()").encode("utf-8"),
            )
            if arguments.get("dryRun") is False:
                module.write_bytes(after)
            return {
                "ok": True,
                "data": {
                    "changed": True,
                    "preHash": sha256(before),
                    "postHash": sha256(after),
                    "affectedTarget": {
                        "sourceSet": "main",
                        "metadataPath": MODULE_METADATA_PATH,
                    },
                    "validation": {"status": "passed"},
                },
                "errors": [],
            }
        if name != "unica.runtime.execute":
            raise AssertionError(f"unexpected public tool call: {name}")

        operation = arguments.get("operation")
        mode = arguments.get("mode")
        if operation == "dump" and mode == "partial":
            if self.mutate_on_partial:
                module = self.workspace / MODULE_RELATIVE_PATH
                module.write_bytes(module.read_bytes() + b"\r\n// partial wrote here")
            return {
                "ok": False,
                "summary": "unica.runtime.execute blocked by source sync guard",
                "errors": [
                    "applied partial dump requires a divergence-safe merge; "
                    "wait for alkoleft/v8-runner-rust#30"
                ],
            }
        if operation == "build":
            self.build_count += 1
            self.database_catalog = (
                self.workspace / CATALOG_RELATIVE_PATH
            ).read_bytes()
            self.database_module = (
                self.workspace / MODULE_RELATIVE_PATH
            ).read_bytes()
            if self.build_count in self.cdfi_build_changes:
                cdfi = self.workspace / "src/ConfigDumpInfo.xml"
                cdfi.write_text(
                    f"<ConfigDumpInfo build=\"{self.build_count}\"/>",
                    encoding="utf-8",
                )
            return {
                "ok": True,
                "summary": "configuration built",
                "stdout": self.diagnostic,
                "errors": [],
            }
        if operation == "dump" and mode == "full":
            if not self.full_dump_noop:
                if self.database_catalog is None or self.database_module is None:
                    raise AssertionError("full dump requires a preceding build")
                (self.workspace / CATALOG_RELATIVE_PATH).write_bytes(
                    self.database_catalog
                )
                (self.workspace / MODULE_RELATIVE_PATH).write_bytes(
                    self.database_module
                )
            if self.lose_after_full_dump == "metadata":
                catalog = self.workspace / CATALOG_RELATIVE_PATH
                catalog.write_text(
                    catalog.read_text(encoding="utf-8").replace(
                        f"<Comment>{METADATA_MARKER}</Comment>",
                        "<Comment/>",
                    ),
                    encoding="utf-8",
                )
            if self.lose_after_full_dump == "module":
                module = self.workspace / MODULE_RELATIVE_PATH
                module.write_bytes(
                    module.read_bytes().replace((BSL_MARKER + "\r\n").encode("utf-8"), b"")
                )
            return {
                "ok": True,
                "summary": "configuration dumped through verified staging",
                "stdout": self.diagnostic,
                "errors": [],
            }
        raise AssertionError(f"unexpected runtime arguments: {arguments}")


class ScriptedSession(ScriptedClient):
    def start(self, required_tools) -> None:
        self.required_tools = frozenset(required_tools)

    def close(self) -> None:
        return None


class Issue76RoundTripTests(unittest.TestCase):
    def test_parent_configuration_is_injected_only_into_private_source_copy(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_copy = write_workspace(root) / "src"
            parent_configuration = root / "1cv8.cf"
            parent_bytes = b"exact vendor configuration payload"
            parent_configuration.write_bytes(parent_bytes)

            receipt = verifier._install_parent_configuration(
                parent_configuration,
                source_copy,
            )

            destination = (
                source_copy
                / PARENT_CONFIGURATION_RELATIVE_PATH.relative_to("src")
            )
            self.assertEqual(destination.read_bytes(), parent_bytes)
            self.assertEqual(parent_configuration.read_bytes(), parent_bytes)
            self.assertEqual(receipt["sha256"], sha256(parent_bytes))
            self.assertEqual(receipt["bytes"], len(parent_bytes))

            destination.write_bytes(b"existing private payload")
            with self.assertRaises(verifier.SourceError):
                verifier._install_parent_configuration(
                    parent_configuration,
                    source_copy,
                )
            self.assertEqual(destination.read_bytes(), b"existing private payload")

    def test_execute_gate_reports_parent_copy_and_proves_input_unchanged(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            inputs = write_gate_inputs(root)
            evidence = root / "evidence"
            evidence.mkdir()
            parent_before = inputs["parent_configuration"].read_bytes()
            sessions = []

            def session_factory(_command, _environment, _timeout, *, cwd):
                session = ScriptedSession(cwd)
                sessions.append(session)
                return session

            exit_code, report = verifier.execute_gate(
                **inputs,
                evidence_dir=evidence,
                session_factory=session_factory,
            )

            private_parent = evidence / "workspace" / PARENT_CONFIGURATION_RELATIVE_PATH
            self.assertEqual(private_parent.read_bytes(), parent_before)
            self.assertEqual(inputs["parent_configuration"].read_bytes(), parent_before)

        self.assertEqual(exit_code, 0, report)
        self.assertEqual(len(sessions), 1)
        parent_report = report["inputs"]["parentConfiguration"]
        self.assertEqual(parent_report["path"], "$PARENT_CONFIGURATION_INPUT")
        self.assertIs(parent_report["statUnchanged"], True)
        self.assertIs(parent_report["hashUnchanged"], True)
        self.assertEqual(parent_report["copy"]["sha256"], sha256(parent_before))
        self.assertIs(report["inputs"]["privateCopiesOnly"], True)
        self.assertIs(
            report["evidence"]["containsProprietaryParentConfiguration"],
            True,
        )

    def test_cli_requires_and_forwards_explicit_parent_configuration(self):
        verifier = load_verifier()
        complete = [
            "--binary",
            "/private/tmp/unica",
            "--plugin-root",
            "/private/tmp/plugin",
            "--database",
            "/private/tmp/input-ib",
            "--sources",
            "/private/tmp/input-src",
            "--parent-configuration",
            "/private/tmp/1cv8.cf",
            "--platform-path",
            "/opt/1cv8/8.3.27.2214",
            "--platform-version",
            "8.3.27.2214",
            "--report",
            "/private/tmp/issue-76-report.json",
            "--execute",
            "--allow-empty-password",
        ]

        with mock.patch.object(
            verifier,
            "execute_gate",
            return_value=(0, {"status": "pass"}),
        ) as execute:
            self.assertEqual(verifier.main(complete), 0)

        self.assertEqual(
            execute.call_args.kwargs["parent_configuration"],
            Path("/private/tmp/1cv8.cf"),
        )

    def test_automatic_evidence_directory_is_validated_before_any_copy(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            inputs = write_gate_inputs(Path(tmp))
            nested_evidence = inputs["sources"] / "unsafe-tmpdir"
            nested_evidence.mkdir()
            temporary = mock.Mock()
            temporary.name = str(nested_evidence)

            with mock.patch.object(
                verifier.tempfile,
                "TemporaryDirectory",
                return_value=temporary,
            ), mock.patch.object(
                verifier,
                "_copy_regular_tree",
                side_effect=AssertionError("copy must not start for unsafe evidence"),
            ) as copy_tree:
                exit_code, report = verifier.execute_gate(
                    **inputs,
                    evidence_dir=None,
                )

        self.assertEqual(exit_code, 2, report)
        self.assertEqual(report["status"], "source-error")
        self.assertIn("evidence directory", report["sourceError"]["message"])
        copy_tree.assert_not_called()

    def test_preexisting_scenario_marker_is_rejected_before_baseline_build(self):
        verifier = load_verifier()
        for target in ("metadata", "module"):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as tmp:
                workspace = write_workspace(Path(tmp))
                if target == "metadata":
                    path = workspace / CATALOG_RELATIVE_PATH
                    path.write_text(
                        path.read_text(encoding="utf-8").replace(
                            "<Comment/>",
                            f"<Comment>{METADATA_MARKER}</Comment>",
                        ),
                        encoding="utf-8",
                    )
                else:
                    path = workspace / MODULE_RELATIVE_PATH
                    path.write_bytes(
                        path.read_bytes().replace(
                            "Функция ЛинияПоддержки()".encode("utf-8"),
                            (BSL_MARKER + "\r\nФункция ЛинияПоддержки()").encode(
                                "utf-8"
                            ),
                        )
                    )
                client = ScriptedClient(workspace)

                exit_code, report = verifier.run_roundtrip_flow(
                    client,
                    workspace=workspace,
                    redactions=[(workspace, "$EVIDENCE")],
                )

                self.assertEqual(exit_code, 1, report)
                self.assertIn("already present", report["summary"]["failures"][0])
                self.assertEqual(client.calls, [])

    def test_redaction_covers_secret_keys_structured_values_and_plain_db_user(self):
        verifier = load_verifier()
        diagnostic = (
            "token=ghp_runtime_secret; password: yaml-secret; "
            "api_secret=api-secret; "
            '\"password\": \"json-secret\"; --api-token cli-token-secret; '
            "authenticated Администратор"
        )

        sanitized = verifier._sanitize_text(
            diagnostic,
            [("Администратор", "$DB_USER")],
        )
        structured = verifier._sanitize_value(
            {
                "token": "structured-token-secret",
                "nested": {"databasePassword": "structured-password-secret"},
                "ordinary": "visible",
            },
            [],
        )
        step = verifier._step_record(
            step_id="redaction",
            tool="unica.runtime.execute",
            arguments={"token": "step-token-secret"},
            payload={"ok": False, "errors": [diagnostic]},
            duration_ms=1,
            redactions=[("Администратор", "$DB_USER")],
        )

        rendered = json.dumps(
            {"text": sanitized, "structured": structured, "step": step},
            ensure_ascii=False,
        )
        for secret in (
            "ghp_runtime_secret",
            "yaml-secret",
            "api-secret",
            "json-secret",
            "cli-token-secret",
            "Администратор",
            "structured-token-secret",
            "structured-password-secret",
            "step-token-secret",
        ):
            self.assertNotIn(secret, rendered)
        self.assertEqual(structured["ordinary"], "visible")
        self.assertEqual(
            step["argumentsSha256"],
            verifier._json_digest({"token": "<credential-redacted>"}),
        )

    def test_packaged_manifest_provenance_is_allowlisted_and_identifies_runner(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            plugin = Path(tmp) / "plugin"
            write_packaged_manifest(plugin, include_secrets=True)

            provenance = verifier._packaged_manifest_provenance(plugin)

        self.assertEqual(provenance["schemaVersion"], 2)
        self.assertEqual(provenance["targetTriple"], "aarch64-apple-darwin")
        self.assertEqual(
            provenance["tools"]["v8-runner"]["sourceCommit"],
            "7ce1b062843d86644fe55741dbe0ee79f7ca767d",
        )
        self.assertEqual(
            set(provenance["tools"]),
            {"unica", "v8-runner"},
        )
        self.assertEqual(
            set(provenance["tools"]["v8-runner"]),
            {"version", "sourceCommit", "sourceTag", "sha256"},
        )
        rendered = json.dumps(provenance, ensure_ascii=False)
        self.assertNotIn("manifest-top-secret", rendered)
        self.assertNotIn("manifest-runner-secret", rendered)

    def test_input_change_cannot_leave_passed_summary_in_source_error_report(self):
        verifier = load_verifier()
        report = {
            "status": "pass",
            "exitCode": 0,
            "summary": {"passed": True, "failures": []},
        }

        verifier._record_source_error(
            report,
            "an input tree changed while the private live scenario ran",
        )

        self.assertEqual((report["status"], report["exitCode"]), ("source-error", 2))
        self.assertIs(report["summary"]["passed"], False)
        self.assertEqual(
            report["summary"]["failures"],
            ["an input tree changed while the private live scenario ran"],
        )
        self.assertEqual(
            report["sourceError"]["message"],
            "an input tree changed while the private live scenario ran",
        )

    def test_ibcmd_project_has_explicit_user_and_empty_password_fields(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            verifier._write_project_configuration(
                workspace,
                database_copy=workspace / "ib",
                platform_path=workspace / "platform",
                platform_version="8.3.27.2214",
                db_user="Администратор",
                builder="IBCMD",
                timeout_seconds=7200,
            )

            local = (workspace / "v8project.local.yaml").read_text(encoding="utf-8")

        self.assertIn('  user: "Администратор"\n', local)
        self.assertIn('  password: ""\n', local)
        self.assertNotIn("Usr=", local)
        self.assertNotIn("Pwd=", local)

    def test_cli_requires_both_mutation_opt_ins_before_execute_gate(self):
        verifier = load_verifier()
        complete = [
            "--binary",
            "/private/tmp/unica",
            "--plugin-root",
            "/private/tmp/plugin",
            "--database",
            "/private/tmp/input-ib",
            "--sources",
            "/private/tmp/input-src",
            "--parent-configuration",
            "/private/tmp/1cv8.cf",
            "--platform-path",
            "/opt/1cv8/8.3.27.2214",
            "--platform-version",
            "8.3.27.2214",
            "--report",
            "/private/tmp/issue-76-report.json",
            "--execute",
            "--allow-empty-password",
        ]

        for omitted in ("--execute", "--allow-empty-password"):
            with self.subTest(omitted=omitted), mock.patch.object(
                verifier,
                "execute_gate",
                side_effect=AssertionError("execute_gate must not run"),
            ) as execute, mock.patch("sys.stderr"):
                argv = [argument for argument in complete if argument != omitted]
                with self.assertRaises(SystemExit) as error:
                    verifier.main(argv)
                self.assertEqual(error.exception.code, 2)
                execute.assert_not_called()

    def test_scripted_flow_baselines_then_uses_incremental_build_and_safe_full_dump(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            workspace = write_workspace(Path(tmp))
            client = ScriptedClient(workspace)

            exit_code, report = verifier.run_roundtrip_flow(
                client,
                workspace=workspace,
                redactions=[(workspace, "$EVIDENCE")],
            )

        self.assertEqual(exit_code, 0, report)
        self.assertEqual((report["status"], report["exitCode"]), ("pass", 0))
        runtime_calls = [
            arguments
            for name, arguments in client.calls
            if name == "unica.runtime.execute"
        ]
        builds = [item for item in runtime_calls if item.get("operation") == "build"]
        self.assertEqual(len(builds), 2, runtime_calls)
        for build in builds:
            self.assertEqual(build["sourceSet"], "main")
            self.assertIs(build["dryRun"], False)
            self.assertNotIn("fullRebuild", build)
        call_names = [
            (name, arguments.get("operation"), arguments.get("dryRun"))
            for name, arguments in client.calls
        ]
        baseline_index = call_names.index(
            ("unica.runtime.execute", "build", False)
        )
        meta_apply_index = call_names.index(("unica.meta.edit", None, False))
        second_build_index = len(call_names) - 1 - call_names[::-1].index(
            ("unica.runtime.execute", "build", False)
        )
        self.assertLess(baseline_index, meta_apply_index)
        self.assertGreater(second_build_index, meta_apply_index)
        self.assertEqual(report["builds"]["baselineBuild"]["ok"], True)
        self.assertEqual(report["builds"]["mutationBuild"]["ok"], True)
        full_dump = next(
            item
            for item in runtime_calls
            if item.get("operation") == "dump" and item.get("mode") == "full"
        )
        self.assertEqual(full_dump["sourceSet"], "main")
        self.assertIs(full_dump["dryRun"], False)
        self.assertEqual(report["roundTrip"]["metadata"]["survived"], True)
        self.assertEqual(report["roundTrip"]["module"]["survived"], True)

    def test_noop_full_dump_cannot_pass_source_to_database_to_source_roundtrip(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            workspace = write_workspace(Path(tmp))
            client = ScriptedClient(workspace, full_dump_noop=True)

            exit_code, report = verifier.run_roundtrip_flow(
                client,
                workspace=workspace,
                redactions=[(workspace, "$EVIDENCE")],
            )

        self.assertEqual(exit_code, 1, report)
        self.assertEqual(report["status"], "failed")
        self.assertIs(report["roundTrip"]["metadata"]["survived"], False)
        self.assertIs(report["roundTrip"]["module"]["survived"], False)

    def test_config_dump_info_attributes_only_post_baseline_build_churn(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            workspace = write_workspace(Path(tmp))
            client = ScriptedClient(workspace, cdfi_build_changes={1})

            exit_code, report = verifier.run_roundtrip_flow(
                client,
                workspace=workspace,
                redactions=[(workspace, "$EVIDENCE")],
            )

        self.assertEqual(exit_code, 0, report)
        self.assertIs(report["configDumpInfo"]["changedByBaselineBuild"], True)
        self.assertIs(report["configDumpInfo"]["changedByBuild"], False)

    def test_applied_partial_guard_must_refuse_without_mutating_sources(self):
        verifier = load_verifier()
        for mutate, expected_exit in ((False, 0), (True, 1)):
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                workspace = write_workspace(Path(tmp))
                client = ScriptedClient(workspace, mutate_on_partial=mutate)

                exit_code, report = verifier.run_roundtrip_flow(
                    client,
                    workspace=workspace,
                    redactions=[(workspace, "$EVIDENCE")],
                )

                self.assertEqual(exit_code, expected_exit, report)
                self.assertEqual(report["partialGuard"]["blocked"], True)
                self.assertEqual(
                    report["partialGuard"]["sourceUnchanged"],
                    not mutate,
                )
                if mutate:
                    operations = [
                        arguments.get("operation")
                        for name, arguments in client.calls
                        if name == "unica.runtime.execute"
                    ]
                    self.assertEqual(operations.count("build"), 1)
                    self.assertNotIn(
                        "full",
                        [
                            arguments.get("mode")
                            for name, arguments in client.calls
                            if name == "unica.runtime.execute"
                        ],
                    )

    def test_loss_of_either_marker_after_full_dump_is_a_failed_roundtrip(self):
        verifier = load_verifier()
        for lost in ("metadata", "module"):
            with self.subTest(lost=lost), tempfile.TemporaryDirectory() as tmp:
                workspace = write_workspace(Path(tmp))
                client = ScriptedClient(workspace, lose_after_full_dump=lost)

                exit_code, report = verifier.run_roundtrip_flow(
                    client,
                    workspace=workspace,
                    redactions=[(workspace, "$EVIDENCE")],
                )

                self.assertEqual(exit_code, 1, report)
                self.assertEqual((report["status"], report["exitCode"]), ("failed", 1))
                self.assertEqual(report["roundTrip"][lost]["survived"], False)

    def test_report_redacts_credentials_and_every_absolute_input_path(self):
        verifier = load_verifier()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            workspace = write_workspace(root)
            database_input = root / "original database"
            source_input = root / "original sources"
            database_input.mkdir()
            source_input.mkdir()
            diagnostic = (
                f'File="{database_input}";Usr="Администратор";'
                f'Pwd=super-secret; source={source_input}; work={workspace}'
            )
            client = ScriptedClient(workspace, diagnostic=diagnostic)

            exit_code, report = verifier.run_roundtrip_flow(
                client,
                workspace=workspace,
                redactions=[
                    (database_input, "$DATABASE_INPUT"),
                    (source_input, "$SOURCE_INPUT"),
                    (workspace, "$EVIDENCE"),
                ],
            )

            rendered = json.dumps(report, ensure_ascii=False)
            self.assertEqual(exit_code, 0, report)
            for forbidden in (
                str(database_input),
                str(source_input),
                str(workspace),
                "super-secret",
                "Pwd=",
            ):
                self.assertNotIn(forbidden, rendered)


if __name__ == "__main__":
    unittest.main()
