"""Failures that must not silently remove acceptance scenarios from CI."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from tests.ci.acceptance_profiles import (
    REPO, SYMBOL_WORKSPACE, DCS_WORKSPACE, MXL_WORKSPACE, completed_delivery_call, host_target,
    isolated_environment, select_profile, stage_analyzer, tools_builder,
)


class AcceptanceProfileTests(unittest.TestCase):
    def corpus(self):
        return {"workspace": "tests/fixtures/acceptance/workspace", "scenarios": [
            {"id": "source", "wire": [{"tool": "unica.view", "args": {}}]},
            {"id": "delivery", "profile": "delivery", "driver": "bsl-analyzer",
             "workspace": SYMBOL_WORKSPACE, "wire": [
                 {"tool": "unica.search", "args": {"role": "symbol"}}]},
            {"id": "agent", "profile": "agent-evaluation", "driver": "codex",
             "evaluation": "dcs-contract", "workspace": DCS_WORKSPACE,
             "wire": [{"tool": "unica.view", "args": {}}]},
            {"id": "mxl", "profile": "agent-evaluation", "driver": "codex",
             "evaluation": "mxl-contract", "workspace": MXL_WORKSPACE,
             "wire": [{"tool": "unica.check", "args": {}}]},
        ]}

    def test_profiles_partition_without_omitting_or_duplicating_scenarios(self):
        corpus = self.corpus()
        source = {s["id"] for s in select_profile(corpus, "source")["scenarios"]}
        delivery = {s["id"] for s in select_profile(corpus, "delivery")["scenarios"]}
        self.assertEqual(source, {"source"})
        self.assertEqual(delivery, {"delivery"})
        agent = {s["id"] for s in select_profile(corpus, "agent-evaluation")["scenarios"]}
        self.assertEqual(agent, {"agent", "mxl"})
        self.assertFalse(agent & (source | delivery))
        self.assertEqual(source | delivery | agent, {s["id"] for s in corpus["scenarios"]})
        self.assertFalse(source & delivery)

    def test_agent_evaluation_cannot_silently_switch_fixture_or_driver(self):
        for field, value in [("evaluation", "mxl-contract"), ("workspace", MXL_WORKSPACE),
                             ("evaluation", "unknown"), ("driver", "bsl-analyzer")]:
            corpus = self.corpus()
            corpus["scenarios"][2][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                select_profile(corpus, "source")

    def test_unsupported_profile_driver_and_fixture_fail_even_when_not_selected(self):
        for field, value in [("profile", "delivrey"), ("profile", "runtime"),
                             ("driver", None), ("driver", "typo"), ("workspace", "other")]:
            corpus = self.corpus()
            corpus["scenarios"][1][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                select_profile(corpus, "source")
        corpus = self.corpus()
        corpus["scenarios"][0]["driver"] = "bsl-analyzer"
        with self.assertRaises(ValueError):
            select_profile(corpus, "delivery")
        corpus["scenarios"][0]["driver"] = None
        with self.assertRaises(ValueError):
            select_profile(corpus, "delivery")
        corpus = self.corpus()
        corpus["scenarios"][1]["wire"] = []
        with self.assertRaises(ValueError):
            select_profile(corpus, "source")
        with self.assertRaises(ValueError):
            select_profile(self.corpus(), "unknown")
        for index, tool, args in [(0, "unica.run", {}), (1, "unica.search", {"role": "lexical"})]:
            corpus = self.corpus()
            corpus["scenarios"][index]["wire"] = [{"tool": tool, "args": args}]
            with self.subTest(tool=tool, args=args), self.assertRaises(ValueError):
                select_profile(corpus, "source")

    def test_host_target_uses_the_lock_and_rejects_unsupported_platform(self):
        lock = json.loads((REPO / "plugins/unica/third-party/tools.lock.json").read_text())
        for system, machine, expected in [("Darwin", "arm64", "darwin-arm64"),
                                          ("Linux", "x86_64", "linux-x64"),
                                          ("Windows", "AMD64", "win-x64")]:
            self.assertEqual(host_target(lock, system, machine), expected)
        with self.assertRaises(ValueError):
            host_target(lock, "Linux", "arm64")

    def test_corrupt_pinned_asset_never_creates_a_runnable_manifest(self):
        builder = tools_builder()
        def corrupt(_url, destination, *, timeout):
            self.assertEqual(timeout, 60)
            destination.parent.mkdir(parents=True)
            destination.write_bytes(b"corrupt analyzer")
        with tempfile.TemporaryDirectory() as raw, patch.object(builder, "download", corrupt):
            plugin = Path(raw) / "plugin"
            with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
                stage_analyzer(plugin, builder=builder, system="Linux", machine="x86_64")
            self.assertFalse((plugin / "third-party/manifest.json").exists())

    def test_download_preserves_the_existing_socket_default_unless_a_deadline_is_requested(self):
        from io import BytesIO
        builder = tools_builder()
        with tempfile.TemporaryDirectory() as raw:
            for timeout, expected in [(None, {}), (60, {"timeout": 60})]:
                with self.subTest(timeout=timeout), patch.object(
                    builder.urllib.request, "urlopen", return_value=BytesIO(b"asset")
                ) as open_url:
                    destination = Path(raw) / "asset"
                    builder.download("https://example.invalid/asset", destination, timeout=timeout)
                    open_url.assert_called_once_with("https://example.invalid/asset", **expected)
                    self.assertEqual(destination.read_bytes(), b"asset")

    def test_delivery_environment_cannot_select_global_project_runtime_or_engine(self):
        keys = ["CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR", "UNICA_RUNTIME_MANIFEST",
                "UNICA_HOST_CONTEXT_REQUIRED", "UNICA_TEST_WORKSPACE_SERVICE_EXE", "EMBEDDING_URL"]
        with patch.dict(os.environ, {**dict.fromkeys(keys, "unrelated"),
                                     "UNICA_PLUGIN_ROOT": "installed", "UNICA_ARTIFACT_CACHE": "global"}):
            result = isolated_environment(Path("overlay"), Path("state"))
        self.assertFalse(any(key in result for key in keys))
        self.assertEqual(result["UNICA_PLUGIN_ROOT"], "overlay")
        self.assertEqual(result["UNICA_ARTIFACT_CACHE"], str(Path("state") / "empty-artifact-cache"))


class DeliveryTaskTests(unittest.TestCase):
    def test_failed_delivery_initialization_closes_partial_resources(self):
        from io import BytesIO
        from tests.ci.test_acceptance_scenarios import DeliveredAcceptanceServer
        for failure in ["invalid-json", "broken-pipe", "spawn"]:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as raw:
                state = Path(raw)
                process = Mock(stdin=BytesIO(), stdout=BytesIO(b"not JSON\n"))
                if failure == "broken-pipe":
                    process.stdin = Mock()
                    process.stdin.write.side_effect = BrokenPipeError("initialize")
                server = DeliveredAcceptanceServer.__new__(DeliveredAcceptanceServer)
                with patch("tests.ci.test_acceptance_scenarios.subprocess.Popen",
                           side_effect=OSError("spawn") if failure == "spawn" else None,
                           return_value=process):
                    with self.assertRaises((ValueError, OSError)):
                        server.__init__(state, state, "2025-03-26", {})
                self.assertTrue(server.stderr_file.closed)
                if failure != "spawn":
                    process.terminate.assert_called_once()
                    process.wait.assert_called_once_with(timeout=10)
                    self.assertTrue(process.stdout.closed)
                    self.assertFalse(server.reader.is_alive())
                    if failure == "broken-pipe":
                        process.stdin.close.assert_called_once()
                    else:
                        self.assertTrue(process.stdin.closed)

    def execute(self, results):
        class Server:
            def __init__(self):
                self.calls = []
                self.results = iter(results)
            def call(self, tool, args, label, *, timeout):
                if not 0 < timeout <= 120:
                    raise AssertionError(f"invalid remaining deadline: {timeout}")
                self.calls.append((tool, args))
                return {"result": {"structuredContent": next(self.results)}}
        server = Server()
        result = completed_delivery_call(server, "unica.search", {"role": "symbol"}, "S324")
        return result["result"]["structuredContent"], server.calls

    def test_task_is_observed_without_resubmitting_search(self):
        terminal = {"ok": True, "data": {"matches": []}}
        receipt = lambda state: {"ok": True, "data": {"task": {"taskId": "task-a", "status": state}}}
        result, calls = self.execute([receipt("queued"), receipt("working"), terminal])
        self.assertEqual(result, terminal)
        self.assertEqual(calls, [("unica.search", {"role": "symbol"}),
                                ("unica.task.result", {"taskId": "task-a", "waitMs": 7000}),
                                ("unica.task.result", {"taskId": "task-a", "waitMs": 7000})])
        with self.assertRaisesRegex(ValueError, "identity"):
            self.execute([receipt("queued"), {"ok": True, "data": {"task": {
                "taskId": "task-b", "status": "working"}}}])

    def test_only_typed_pending_index_is_retried_and_exhaustion_fails(self):
        pending = {"ok": False, "data": {"matches": [{
            "role": "symbol", "provider": "bsl-analyzer", "termination": {
                "code": "dependencyPending", "retryable": True}}]}}
        terminal = {"ok": True, "data": {"matches": []}}
        result, calls = self.execute([pending, terminal])
        self.assertEqual(result, terminal)
        self.assertEqual(len(calls), 2)
        with self.assertRaises(TimeoutError):
            self.execute([pending] * 20)
        for code in ["providerFailed", "deadlineExceeded"]:
            refused = {"ok": False, "data": {"matches": [{"termination": {"code": code}}]}}
            result, calls = self.execute([refused])
            self.assertEqual(result, refused)
            self.assertEqual(len(calls), 1)

    def test_elapsed_deadline_prevents_another_provider_request(self):
        server = Mock()
        server.call.return_value = {"result": {"structuredContent": {
            "ok": False, "data": {"matches": [{"role": "symbol", "provider": "bsl-analyzer",
                "termination": {"code": "dependencyPending", "retryable": True}}]}}}}
        with patch("tests.ci.acceptance_profiles.time.monotonic", side_effect=[0, 0, 121]):
            with self.assertRaises(TimeoutError):
                completed_delivery_call(server, "unica.search", {"role": "symbol"}, "S324")
        self.assertEqual(server.call.call_count, 1)
        self.assertEqual(server.call.call_args.kwargs["timeout"], 120)

    def test_unrelated_jsonrpc_messages_do_not_reset_the_delivery_deadline(self):
        import queue
        from tests.ci.test_acceptance_scenarios import AcceptanceServer
        server = AcceptanceServer.__new__(AcceptanceServer)
        server.lines = queue.Queue()
        server.lines.put(b'{"id":9}')
        server.lines.put(b'{"id":1}')
        server.label = "S324"
        server.close = Mock()
        server.stderr_tail = lambda: ""
        with patch("tests.ci.test_acceptance_scenarios.time.monotonic", side_effect=[0, 0.05, 0.15]):
            with self.assertRaises(TimeoutError):
                server.receive(expected_id=1, timeout=0.1)
        server.close.assert_called_once()
        self.assertEqual(server.lines.qsize(), 1)
