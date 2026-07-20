from __future__ import annotations

import importlib.util
import json
import os
import stat
import tarfile
import tempfile
import unittest
from pathlib import Path


def load_assessment_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "release-assessment.py"
    spec = importlib.util.spec_from_file_location("release_assessment", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_bsp_harvest_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "harvest-bsp-parity-fixtures.py"
    spec = importlib.util.spec_from_file_location("harvest_bsp_parity_fixtures", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseAssessmentTests(unittest.TestCase):
    def write_response_id_mcp(self, path: Path, response_ids: list[int]) -> None:
        path.write_text(
            f"""#!/usr/bin/env python3
from __future__ import annotations

import json
import sys

message = json.loads(sys.stdin.readline())
for response_id in {json.dumps(response_ids)}:
    print(json.dumps({{
        "jsonrpc": "2.0",
        "id": response_id,
        "result": {{"content": [{{"type": "text", "text": "ok"}}]}},
    }}), flush=True)
for _raw in sys.stdin:
    pass
""",
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def write_eof_sensitive_mcp(self, path: Path) -> None:
        path.write_text(
            """#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import threading
import time

cancelled = threading.Event()
workers = []

def respond(message):
    time.sleep(0.2)
    response = {"jsonrpc": "2.0", "id": message.get("id")}
    if cancelled.is_set():
        response["error"] = {"code": -32800, "message": "request cancelled"}
    else:
        response["result"] = {"content": [{"type": "text", "text": "ok"}]}
    print(json.dumps(response), flush=True)

for raw in sys.stdin:
    print(json.dumps({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}}), flush=True)
    worker = threading.Thread(target=respond, args=(json.loads(raw),))
    worker.start()
    workers.append(worker)

cancelled.set()
for worker in workers:
    worker.join()
""",
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def write_fake_mcp(self, path: Path) -> None:
        path.write_text(
            """#!/usr/bin/env python3
from __future__ import annotations

import json
import sys

TOOLS = [
    "unica.project.status",
    "unica.project.map",
    "unica.cf.info",
    "unica.cf.validate",
    "unica.code.diagnostics",
    "unica.code.search",
    "unica.code.grep",
    "unica.code.outline",
    "unica.meta.profile",
    "unica.standards.explain",
]

for raw in sys.stdin:
    message = json.loads(raw)
    method = message.get("method")
    response = {"jsonrpc": "2.0", "id": message.get("id")}
    if method == "initialize":
        response["result"] = {"serverInfo": {"name": "unica"}}
    elif method == "tools/list":
        response["result"] = {"tools": [{"name": name} for name in TOOLS]}
    elif method == "tools/call":
        params = message["params"]
        name = params["name"]
        arguments = params.get("arguments", {})
        payload = {
            "ok": True,
            "summary": f"{name} completed",
            "stdout": "",
            "warnings": [],
            "errors": [],
            "artifacts": [],
        }
        if name == "unica.project.map":
            payload["stdout"] = json.dumps({
                "sourceSets": [
                    {"name": "main", "path": "src/cf", "sourceFormat": "platform_xml"}
                ]
            }, ensure_ascii=False)
        elif name == "unica.code.diagnostics" and arguments.get("mode") == "workspace":
            payload["diagnostics"] = [
                {"code": "UnusedLocalVariable", "file": "CommonModules/Test/Ext/Module.bsl"}
            ]
        elif name == "unica.standards.explain":
            payload["stdout"] = "UnusedLocalVariable: standard explanation"
        response["result"] = {"content": [{"type": "text", "text": json.dumps(payload)}]}
    else:
        response["error"] = {"code": -32601, "message": f"unsupported {method}"}
    print(json.dumps(response), flush=True)
""",
            encoding="utf-8",
        )
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def test_scenario_runner_records_success_metrics_and_json_lines(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            self.write_fake_mcp(fake_mcp)
            bsp_root = root / "bsp"
            (bsp_root / "src" / "cf").mkdir(parents=True)
            (bsp_root / "src" / "cf" / "Module.bsl").write_text("Процедура Smoke()\nКонецПроцедуры\n", encoding="utf-8")
            (bsp_root / "description.json").write_text(
                json.dumps({"Версия": "3.2.1.446", "Дата": "2026-06-17T00:00:00"}, ensure_ascii=False),
                encoding="utf-8",
            )
            out_dir = root / "out"

            report = module.build_assessment_report(
                run_unica=fake_mcp,
                bsp_root=bsp_root,
                cache_dir=root / "cache",
                out_dir=out_dir,
                release_tag="v9.9.9",
                github_run_id="12345",
                candidate_package="unica-codex-marketplace-linux-x64.tar.gz",
                bsp_commit="abc123",
                timeout_seconds=10,
            )

            self.assertEqual(report["schemaVersion"], 1)
            self.assertEqual(report["summary"]["status"], "passed")
            self.assertEqual(report["bsp"]["commit"], "abc123")
            self.assertEqual(report["bsp"]["ref"], module.BSP_REF)
            self.assertEqual(report["bsp"]["requestedRef"], module.BSP_REF)
            self.assertTrue(all(scenario["durationMs"] >= 0 for scenario in report["scenarios"]))
            self.assertIn("UnusedLocalVariable", report["summary"]["qualityFindings"]["diagnosticCodes"])
            self.assertTrue((out_dir / "assessment.json").is_file())
            self.assertTrue((out_dir / "assessment.ndjson").is_file())
            lines = (out_dir / "assessment.ndjson").read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(lines), len(report["scenarios"]))
            self.assertTrue((out_dir / "index.html").read_text(encoding="utf-8").startswith("<!doctype html>"))
            self.assertIn("v9.9.9", (out_dir / "summary.md").read_text(encoding="utf-8"))

    def test_mcp_client_keeps_stdin_open_until_delayed_response(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            self.write_eof_sensitive_mcp(fake_mcp)

            responses, _duration_ms, _stdout, stderr, returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertEqual(returncode, 0, stderr)
            self.assertEqual(len(responses), 1)
            self.assertNotIn("error", responses[0])

    def test_mcp_client_rejects_unexpected_response_id(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            self.write_response_id_mcp(fake_mcp, [999])

            responses, _duration_ms, _stdout, _stderr, _returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertTrue(
                any("unexpected JSON-RPC response id" in response.get("error", {}).get("message", "") for response in responses)
            )

    def test_mcp_client_rejects_duplicate_response_id(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            self.write_response_id_mcp(fake_mcp, [1, 1])

            responses, _duration_ms, _stdout, _stderr, _returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertTrue(
                any("duplicate JSON-RPC response id" in response.get("error", {}).get("message", "") for response in responses)
            )

    def test_mcp_client_returns_responses_in_request_order(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            self.write_response_id_mcp(fake_mcp, [2, 1])

            responses, _duration_ms, _stdout, _stderr, _returncode = module.call_mcp(
                fake_mcp,
                [
                    module.tool_call_message(1, "unica.project.status", {}),
                    module.tool_call_message(2, "unica.project.map", {}),
                ],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertEqual([response.get("id") for response in responses], [1, 2])

    def test_mcp_client_reports_invalid_json_line(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            fake_mcp.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.stdin.readline()\nprint('not-json', flush=True)\nfor _raw in sys.stdin: pass\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(fake_mcp.stat().st_mode | stat.S_IXUSR)

            responses, _duration_ms, _stdout, _stderr, _returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertTrue(
                any("invalid JSON-RPC line" in response.get("error", {}).get("message", "") for response in responses)
            )

    def test_mcp_client_preserves_early_exit_stderr(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            fake_mcp.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.stderr.write('fatal stderr\\n')\nsys.stderr.flush()\nraise SystemExit(7)\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(fake_mcp.stat().st_mode | stat.S_IXUSR)

            responses, _duration_ms, _stdout, stderr, returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertEqual(returncode, 7)
            self.assertIn("fatal stderr", stderr)
            self.assertTrue(any("missing JSON-RPC responses" in response.get("error", {}).get("message", "") for response in responses))

    def test_mcp_client_times_out_and_reaps_silent_process(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            fake_mcp.write_text(
                "#!/usr/bin/env python3\nimport sys\nimport time\nsys.stdin.readline()\ntime.sleep(60)\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(fake_mcp.stat().st_mode | stat.S_IXUSR)

            responses, duration_ms, _stdout, stderr, returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=0.1,
            )

            self.assertEqual(responses, [])
            self.assertEqual(returncode, 124)
            self.assertIn("timed out", stderr)
            self.assertLess(duration_ms, 2_000)

    def test_mcp_client_closes_and_reaps_process_after_write_error(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fake_mcp = root / ("run-unica.py" if os.name == "nt" else "run-unica")
            marker = fake_mcp.with_suffix(".closed")
            fake_mcp.write_text(
                "#!/usr/bin/env python3\nfrom pathlib import Path\nPath(__file__).with_suffix('.closed').write_text('closed', encoding='utf-8')\n",
                encoding="utf-8",
            )
            fake_mcp.chmod(fake_mcp.stat().st_mode | stat.S_IXUSR)

            _responses, _duration_ms, _stdout, stderr, returncode = module.call_mcp(
                fake_mcp,
                [module.tool_call_message(1, "unica.cf.info", {"value": "Ж" * 1_000_000})],
                cwd=root,
                cache_dir=root / "cache",
                timeout_seconds=2,
            )

            self.assertNotEqual(returncode, 0)
            self.assertIn("failed to write MCP request", stderr)
            self.assertEqual(marker.read_text(encoding="utf-8"), "closed")

    def test_report_rendering_escapes_failure_text(self) -> None:
        module = load_assessment_module()

        report = {
            "schemaVersion": 1,
            "unicaVersion": "0.4.4",
            "releaseTag": "v0.4.4",
            "githubRunId": "run",
            "candidatePackage": "pkg.tar.gz",
            "bsp": {
                "repo": "https://github.com/1c-syntax/ssl_3_2",
                "ref": "master",
                "commit": "abc",
                "descriptionVersion": "3.2.1.446",
                "descriptionDate": "2026-06-17T00:00:00",
            },
            "environment": {"os": "Linux", "python": "3.12"},
            "summary": {
                "status": "failed",
                "blockingFailures": 1,
                "qualityFindings": {"diagnosticCodes": []},
                "performance": {"totalDurationMs": 7},
            },
            "scenarios": [
                {
                    "id": "broken",
                    "title": "Broken scenario",
                    "tool": "unica.cf.info",
                    "argumentsDigest": "sha256:abc",
                    "status": "failed",
                    "durationMs": 7,
                    "blocking": True,
                    "metrics": {},
                    "errors": ["<script>alert('x')</script>"],
                    "artifacts": [],
                }
            ],
        }

        html = module.render_html(report)

        self.assertIn("&lt;script&gt;alert", html)
        self.assertNotIn("<script>alert", html)
        self.assertIn("Blocking failures", html)

    def test_summary_passes_when_only_non_blocking_scenarios_failed(self) -> None:
        module = load_assessment_module()

        scenarios = [
            module.scenario_result(
                scenario_id="quality-smoke",
                title="Quality smoke",
                tool="unica.form.info",
                arguments={},
                status="failed",
                duration_ms=7,
                blocking=False,
                errors=["runtime dependency missing"],
            )
        ]

        summary = module.build_summary(scenarios, [], Path("/tmp/unica-no-cache"))

        self.assertEqual(summary["status"], "passed")
        self.assertEqual(summary["blockingFailures"], 0)
        self.assertEqual(summary["qualityFindings"]["nonBlockingFailures"], 1)

    def test_default_bsp_ref_is_pinned_and_report_records_requested_ref(self) -> None:
        module = load_assessment_module()

        self.assertNotEqual(module.BSP_REF, "master")
        self.assertEqual(module.BSP_REF, "3.2.1.446")

    def test_bsp_parity_harvest_selects_text_fixtures_and_writes_manifest(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            (src / "Catalogs" / "Партнеры" / "Forms" / "ФормаЭлемента" / "Ext").mkdir(parents=True)
            (
                src
                / "Reports"
                / "ОтчетПродажи"
                / "Templates"
                / "ОсновнаяСхемаКомпоновкиДанных"
                / "Ext"
            ).mkdir(parents=True)
            (src / "Roles" / "ПолныеПрава" / "Ext").mkdir(parents=True)
            (src / ".build").mkdir(parents=True)
            (src / ".build" / "bsl-search.db").write_bytes(b"cache")
            (src / "Configuration.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            (src / "Catalogs" / "Партнеры.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            (src / "Catalogs" / "Партнеры" / "Forms" / "ФормаЭлемента" / "Ext" / "Form.xml").write_text(
                "<Form/>", encoding="utf-8"
            )
            (
                src
                / "Reports"
                / "ОтчетПродажи"
                / "Templates"
                / "ОсновнаяСхемаКомпоновкиДанных"
                / "Ext"
                / "Template.xml"
            ).write_text("<DataCompositionSchema/>", encoding="utf-8")
            (src / "Roles" / "ПолныеПрава" / "Ext" / "Rights.xml").write_text("<Rights/>", encoding="utf-8")

            out = root / "fixtures"
            manifest = module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123")

            self.assertEqual(manifest["bsp"]["ref"], "test-ref")
            self.assertEqual(manifest["bsp"]["commit"], "abc123")
            self.assertEqual(json.loads((out / "manifest.json").read_text(encoding="utf-8")), manifest)
            self.assertEqual(
                manifest["files"],
                sorted(manifest["files"], key=lambda entry: (entry["target"], entry["source"])),
            )
            self.assertTrue(
                all({"category", "sha256", "size", "source", "target"} <= set(entry) for entry in manifest["files"])
            )
            self.assertEqual(
                module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123"),
                manifest,
            )
            harvested = sorted(path.relative_to(out).as_posix() for path in out.rglob("*") if path.is_file())
            self.assertIn("manifest.json", harvested)
            self.assertIn("cf/Configuration.xml", harvested)
            self.assertTrue(any(path.startswith("forms/") and path.endswith("/Form.xml") for path in harvested))
            self.assertTrue(any(path.startswith("dcs/") and path.endswith("/Template.xml") for path in harvested))
            self.assertTrue(any(path.startswith("roles/") and path.endswith("/Rights.xml") for path in harvested))
            self.assertFalse(any(".build" in path or path.endswith(".db") for path in harvested))

    def test_bsp_parity_harvest_rejects_dangerous_out_root_and_leaves_sentinel(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            src.mkdir(parents=True)
            (src / "Configuration.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            sentinel = bsp / "sentinel.txt"
            sentinel.write_text("keep", encoding="utf-8")

            with self.assertRaises(ValueError):
                module.harvest(bsp_root=bsp, out_root=bsp, bsp_ref="test-ref", bsp_commit="abc123")

            self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")

            symlink_target = root / "symlink-target"
            symlink_target.mkdir()
            symlink_sentinel = symlink_target / "sentinel.txt"
            symlink_sentinel.write_text("keep", encoding="utf-8")
            symlink_out = root / "out-link"
            try:
                symlink_out.symlink_to(symlink_target, target_is_directory=True)
            except (NotImplementedError, OSError):
                return

            with self.assertRaises(ValueError):
                module.harvest(bsp_root=bsp, out_root=symlink_out, bsp_ref="test-ref", bsp_commit="abc123")

            self.assertEqual(symlink_sentinel.read_text(encoding="utf-8"), "keep")

    def test_bsp_parity_harvest_rejects_existing_unmarked_directory(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            src.mkdir(parents=True)
            (src / "Configuration.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            out = root / "fixtures"
            out.mkdir()
            sentinel = out / "sentinel.txt"
            sentinel.write_text("keep", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "without BSP harvest manifest marker"):
                module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123")

            self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")

    def test_bsp_parity_harvest_rejects_parity_fixture_parent(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            src.mkdir(parents=True)
            (src / "Configuration.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            fixture_parent = root / "tests" / "fixtures" / "unica_mcp_script_parity"
            fixture_parent.mkdir(parents=True)
            sentinel = fixture_parent / "existing-fixture.xml"
            sentinel.write_text("<Fixture/>", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "without BSP harvest manifest marker"):
                module.harvest(
                    bsp_root=bsp,
                    out_root=fixture_parent,
                    bsp_ref="test-ref",
                    bsp_commit="abc123",
                )

            self.assertEqual(sentinel.read_text(encoding="utf-8"), "<Fixture/>")

    def test_bsp_parity_harvest_skips_symlinked_source_file(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            catalogs = src / "Catalogs"
            catalogs.mkdir(parents=True)
            external = root / "external.xml"
            external.write_text("<MetaDataObject/>", encoding="utf-8")
            try:
                (catalogs / "Linked.xml").symlink_to(external)
            except (NotImplementedError, OSError) as exc:
                self.skipTest(f"symlink not available: {exc}")

            out = root / "fixtures"
            manifest = module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123")

            targets = {entry["target"] for entry in manifest["files"]}
            self.assertNotIn("meta/Catalogs/Linked.xml", targets)
            self.assertFalse((out / "meta" / "Catalogs" / "Linked.xml").exists())

    def test_bsp_parity_harvest_includes_common_module_descriptor_and_bsl(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            module_dir = src / "CommonModules" / "Demo" / "Ext"
            module_dir.mkdir(parents=True)
            (src / "CommonModules" / "Demo.xml").write_text("<MetaDataObject/>", encoding="utf-8")
            (module_dir / "Module.bsl").write_text("Процедура Demo()\nКонецПроцедуры\n", encoding="utf-8")

            out = root / "fixtures"
            manifest = module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123")

            targets = {entry["target"] for entry in manifest["files"]}
            self.assertIn("meta/CommonModules/Demo.xml", targets)
            self.assertIn("meta/CommonModules/Demo/Module.bsl", targets)
            self.assertEqual(
                (out / "meta" / "CommonModules" / "Demo" / "Module.bsl").read_text(encoding="utf-8"),
                "Процедура Demo()\nКонецПроцедуры\n",
            )

    def test_bsp_parity_harvest_skips_non_utf8_and_large_fixture_candidates(self) -> None:
        module = load_bsp_harvest_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bsp = root / "bsp"
            src = bsp / "src" / "cf"
            (src / "Catalogs").mkdir(parents=True)
            (src / "Documents").mkdir(parents=True)
            (src / "Catalogs" / "BadEncoding.xml").write_bytes(b"\xff\xfe\x00")
            (src / "Documents" / "Huge.xml").write_text("x" * (256 * 1024 + 1), encoding="utf-8")

            out = root / "fixtures"
            manifest = module.harvest(bsp_root=bsp, out_root=out, bsp_ref="test-ref", bsp_commit="abc123")

            targets = {entry["target"] for entry in manifest["files"]}
            self.assertNotIn("meta/Catalogs/BadEncoding.xml", targets)
            self.assertNotIn("meta/Documents/Huge.xml", targets)
            self.assertFalse((out / "meta" / "Catalogs" / "BadEncoding.xml").exists())
            self.assertFalse((out / "meta" / "Documents" / "Huge.xml").exists())

    def test_versioned_pages_copy_preserves_existing_versions_and_latest(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pages_root = root / "pages"
            existing = pages_root / "assessments" / "v0.4.4"
            existing.mkdir(parents=True)
            (existing / "index.html").write_text("old", encoding="utf-8")
            report_dir = root / "report"
            report_dir.mkdir()
            (report_dir / "index.html").write_text("new", encoding="utf-8")
            (report_dir / "assessment.json").write_text("{}", encoding="utf-8")

            module.copy_versioned_pages(report_dir, pages_root, "v0.4.5")

            self.assertEqual((existing / "index.html").read_text(encoding="utf-8"), "old")
            self.assertEqual(
                (pages_root / "assessments" / "v0.4.5" / "index.html").read_text(encoding="utf-8"),
                "new",
            )
            self.assertEqual(
                (pages_root / "assessments" / "latest" / "assessment.json").read_text(encoding="utf-8"),
                "{}",
            )

    def test_extract_unica_binary_from_linux_marketplace_archive(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package_root = root / "pkg" / "unica-codex-marketplace-linux-x64"
            plugin_root = package_root / "plugins" / "unica"
            bin_dir = plugin_root / "bin" / "linux-x64"
            bin_dir.mkdir(parents=True)
            (plugin_root / ".codex-plugin").mkdir(parents=True)
            (plugin_root / ".codex-plugin" / "plugin.json").write_text("{}", encoding="utf-8")
            run_unica = bin_dir / "unica"
            run_unica.write_text("#!/usr/bin/env sh\n", encoding="utf-8")
            run_unica.chmod(run_unica.stat().st_mode | stat.S_IXUSR)
            archive = root / "unica-codex-marketplace-linux-x64.tar.gz"
            with tarfile.open(archive, "w:gz") as tf:
                tf.add(package_root, arcname="unica-codex-marketplace-linux-x64")

            extracted = module.extract_marketplace_archive(archive, root / "extract")

            self.assertEqual(extracted.name, "unica")
            self.assertEqual(module.plugin_root_for(extracted).name, "unica")
            self.assertTrue(extracted.is_file())

    def test_extract_unica_binary_from_thin_delivery_runtime_archive(self) -> None:
        module = load_assessment_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            runtime_root = root / "runtime"
            binary = runtime_root / "bin" / "linux-x64" / "unica"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/usr/bin/env sh\n", encoding="utf-8")
            (runtime_root / "third-party").mkdir()
            (runtime_root / "third-party" / "manifest.json").write_text("{}", encoding="utf-8")
            archive = root / "unica-runtime-linux-x64.tar.gz"
            with tarfile.open(archive, "w:gz") as tf:
                tf.add(binary, arcname="bin/linux-x64/unica")
                tf.add(
                    runtime_root / "third-party" / "manifest.json",
                    arcname="third-party/manifest.json",
                )

            extracted = module.extract_marketplace_archive(archive, root / "extract")

            self.assertEqual(extracted.relative_to(module.plugin_root_for(extracted)).as_posix(), "bin/linux-x64/unica")


if __name__ == "__main__":
    unittest.main()
