from __future__ import annotations

import importlib.util
import json
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


class ReleaseAssessmentTests(unittest.TestCase):
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
            fake_mcp = root / "run-unica"
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


if __name__ == "__main__":
    unittest.main()
