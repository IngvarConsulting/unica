"""Public MCP search over an actual managed external analyzer process."""
from __future__ import annotations

import importlib.util
import json
import os
import platform
import tempfile
import time
import unittest
from contextlib import ExitStack, contextmanager
from pathlib import Path

from scripts.ci.prepared_binary import ensure_binary
from tests.ci.search_failure_fixture import CANARIES, ROOT, prepare_fixture


CONVERSION_MESSAGE = "search provider response conversion failed: unsupported search payload"



def load_smoke():
    spec = importlib.util.spec_from_file_location(
        "search_failure_smoke", ROOT / "scripts/ci/smoke-unica-mcp.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module




@unittest.skipUnless(
    (platform.system() == "Darwin" and platform.machine().lower() in {"arm64", "aarch64"})
    or (platform.system() == "Linux" and platform.machine().lower() in {"amd64", "x86_64"}),
    "host capability executable reader stand-ins: existing fixture has no native Windows launcher",
)
class SearchFailureDiagnosisTests(unittest.TestCase):
    """rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE"""

    @classmethod
    def setUpClass(cls):
        cls.binary = ensure_binary(ROOT)
        cls.smoke = load_smoke()

    @contextmanager
    def session(self, root, workspace, extra):
        cache = root / "workspace-cache"
        state = root / "state"
        env = os.environ.copy()
        for name in ("UNICA_RUNTIME_MANIFEST", "UNICA_STARTUP_NOTICE", "UNICA_STARTUP_NOTICE_ACK", "CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR"):
            env.pop(name, None)
        env.update(extra)
        env.update({
            "UNICA_PROVIDER_STATE_DIR": str(state), "UNICA_CACHE_DIR": str(cache),
            "UNICA_DAEMON_IDLE_GRACE_MS": "100",
        })
        session = self.smoke.McpSession(
            [str(self.binary)], env, 45, cwd=workspace,
            service_cache_root=cache,
        )
        identities = set()
        try:
            init = session.request({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                           "clientInfo": {"name": "search-failure-fixture", "version": "1"}},
            })
            self.assertIn("result", init)
            yield session
        finally:
            pids = set()
            for path in state.glob("*/endpoint.json"):
                try:
                    pids.add(json.loads(path.read_text(encoding="utf-8"))["pid"])
                except FileNotFoundError:
                    pass
            identities.update(session.process_ownership.capture_identities(pids))
            session.observe_workspace_services(cache)
            identities.update(session.observed_service_identities())
            try:
                self.smoke._shutdown_workspace_services(cache, 10)
                session.close()
            finally:
                session.terminate_tree(cache, identities)

    def domain(self, response):
        self.assertNotIn("error", response, response)
        result = response["result"]
        if "structuredContent" in result:
            return result["structuredContent"]
        return json.loads(result["content"][0]["text"])

    def run_search(self, mode):
        self.polled_same_task = False
        with tempfile.TemporaryDirectory() as directory, ExitStack() as releases:
            root = Path(directory).resolve()
            workspace, extra, log = prepare_fixture(root, mode)
            with self.session(root, workspace, extra) as session:
                response = session.request({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "unica.search", "arguments": {
                        "query": "QueryPrivate2470", "role": "symbol",
                        "scope": "main:Configuration", "limit": 2,
                    }},
                })
                result = self.domain(response)
                first_task_id = None
                for request_id in range(3, 20):
                    for canary in CANARIES:
                        self.assertNotIn(canary, json.dumps(response, ensure_ascii=False))
                    data = result.get("data") or {}
                    task_id = (data.get("task") or {}).get("taskId")
                    if task_id is None:
                        break
                    self.polled_same_task = True
                    if first_task_id is None:
                        first_task_id = task_id
                        if mode == "blocked-unknown-json":
                            # O_RDWR never waits for a reader to reach open().
                            # Keep this owned descriptor until the session ends,
                            # so the release byte survives even if preparation
                            # handed off to Task before the analyzer reached it.
                            release_fd = os.open(extra["UNICA_SEARCH_FAILURE_RELEASE"], os.O_RDWR | os.O_NONBLOCK)
                            releases.callback(os.close, release_fd)
                            self.assertEqual(os.write(release_fd, b"1"), 1)
                    else:
                        self.assertEqual(task_id, first_task_id)
                    response = session.request({
                        "jsonrpc": "2.0", "id": request_id, "method": "tools/call",
                        "params": {"name": "unica.task.result", "arguments": {
                            "taskId": task_id, "waitMs": 7000,
                        }},
                    })
                    result = self.domain(response)
                else:
                    self.fail("the same search Task did not reach a terminal result")
                serialized = json.dumps(response, ensure_ascii=False)
                for canary in CANARIES:
                    self.assertNotIn(canary, serialized)
            self.assertTrue(log.exists(), "the external analyzer was never called")
            calls = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]
            searches = [call for call in calls if call.get("name") == "search"]
            self.assertEqual(len(searches), 1, "Task polling must not rerun search")
            self.assertEqual(searches[0]["arguments"]["query"], "QueryPrivate2470")
            return result

    def assert_failure(self, result, code, outcome, termination):
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["diagnostics"][0]["code"], code)
        self.assertEqual(result["diagnostics"][0]["outcome"], outcome)
        section = result["data"]["matches"][0]
        self.assertEqual(section["provider"], "bsl-analyzer")
        self.assertFalse(section["searchComplete"])
        self.assertNotEqual(section["status"], "empty")
        self.assertEqual(section["termination"]["code"], termination)
        return section

    def test_unknown_json_has_safe_conversion_diagnosis(self):
        result = self.run_search("unknown-json")
        self.assert_failure(result, "provider_failed", "deadEnd", "providerFailed")
        self.assertEqual(result["diagnostics"][0]["message"], CONVERSION_MESSAGE)

    def test_unknown_text_has_safe_conversion_diagnosis(self):
        result = self.run_search("unknown-text")
        self.assert_failure(result, "provider_failed", "deadEnd", "providerFailed")
        self.assertEqual(result["diagnostics"][0]["message"], CONVERSION_MESSAGE)

    def test_task_result_preserves_conversion_diagnosis_without_repeating_search(self):
        result = self.run_search("blocked-unknown-json")
        self.assertTrue(self.polled_same_task, "fixture must cross the synchronous handoff window")
        self.assert_failure(result, "provider_failed", "deadEnd", "providerFailed")
        self.assertEqual(result["diagnostics"][0]["message"], CONVERSION_MESSAGE)

    def test_index_not_ready_remains_retryable(self):
        result = self.run_search("pending")
        section = self.assert_failure(result, "provider_unavailable", "retry", "dependencyPending")
        self.assertTrue(section["termination"]["retryable"])
        self.assertEqual(result["diagnostics"][0]["detailCode"], "dependency_pending")

    def test_external_process_connection_failure_is_not_conversion_failure(self):
        result = self.run_search("connection")
        self.assert_failure(result, "provider_failed", "deadEnd", "providerFailed")
        self.assertEqual(
            result["diagnostics"][0]["message"],
            "search provider connection failed while reading the analyzer response",
        )

    def test_remote_tool_error_is_not_conversion_failure(self):
        result = self.run_search("remote-error")
        self.assert_failure(result, "provider_failed", "deadEnd", "providerFailed")
        self.assertEqual(
            result["diagnostics"][0]["message"],
            "search provider reported an operation failure",
        )

    def test_explicit_provider_timeout_remains_retryable(self):
        result = self.run_search("timeout")
        section = self.assert_failure(result, "deadline_exceeded", "retry", "deadlineExceeded")
        self.assertTrue(section["termination"]["retryable"])

    def test_known_empty_response_still_succeeds(self):
        result = self.run_search("empty")
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["data"]["matches"][0]["status"], "empty")

    def test_known_empty_ready_footer_still_succeeds(self):
        result = self.run_search("empty-ready")
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["data"]["matches"][0]["status"], "empty")


if __name__ == "__main__":
    unittest.main()
