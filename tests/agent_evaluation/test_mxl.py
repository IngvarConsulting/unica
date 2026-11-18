"""Локальная агентная приёмка сценария MXL."""
from scripts.ci.prepared_binary import binary_path, ensure_binary
import json
import os
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

from tests.agent_evaluation.mxl_driver import REPO, evaluate, independent_review
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import select_profile
from tests.ci.test_acceptance_scenarios import run_corpus


class MxlAgentAcceptanceTests(unittest.TestCase):
    def test_agent_uses_current_contract_without_retired_skills(self):
        self.run_mxl()

    def test_optional_docs_cancellation_keeps_real_mxl_acceptance(self):
        """A real provider stays unanswered; only the product can cancel its Task."""
        arrived, release = threading.Event(), threading.Event()

        class PendingDocumentation(BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                arrived.set()
                # This is a pending supplier, not a fabricated Task response.
                # The event releases it during cleanup; the bound prevents leaks.
                release.wait(120)
                self.close_connection = True

            def log_message(self, *_):
                pass

        provider = ThreadingHTTPServer(("127.0.0.1", 0), PendingDocumentation)
        provider.daemon_threads = True
        worker = threading.Thread(target=provider.serve_forever, daemon=True)
        worker.start()
        try:
            with tempfile.TemporaryDirectory() as raw:
                fixture = Path(raw) / "agent-mxl-cancel"
                shutil.copytree(REPO / "tests/fixtures/acceptance/agent-mxl", fixture)
                prompt = fixture / "prompt.md"
                prompt.write_text(prompt.read_text() + """

Перед основной работой начни необязательный поиск справки:
unica.docs {"query":"табличный документ","source":"development-standard","limit":1}.
Поставщик этого дополнительного поиска намеренно не отвечает. Получив Task,
сразу запроси unica.task.cancel для его taskId и затем наблюдай именно
unica.task.result того же taskId до конечного ответа. Отправка cancel сама
не доказывает завершения. После отмены продолжай всю основную работу с MXL;
эта дополнительная справка для неё не требуется. Не отменяй основные операции
и не выдавай отменённый поиск за успешно полученную справку.
""", encoding="utf-8")
                endpoint = f"http://127.0.0.1:{provider.server_port}/mcp"
                with patch.dict(os.environ, {"UNICA_STANDARDS_MCP_URL": endpoint}):
                    self.run_mxl(fixture=fixture, require_cancel=True)
                self.assertTrue(arrived.is_set(), "the real docs provider was never reached")
        finally:
            release.set()
            provider.shutdown()
            provider.server_close()
            worker.join(timeout=5)
            self.assertFalse(worker.is_alive(), "documentation fixture did not stop")

    def run_mxl(self, *, fixture=None, require_cancel=False):
        ensure_binary(REPO)
        corpus = select_profile(load_acceptance_corpus(), "agent-evaluation")
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["evaluation"] == "mxl-contract"]
        proofs = REPO / ".session-temp/acceptance-agent"
        def driver(server, scenario):
            proof = evaluate(server, scenario, proofs, fixture=fixture)
            if require_cancel:
                from tests.agent_evaluation.mcp_recorder import recorded_calls
                from tests.agent_evaluation.dcs_driver import terminal_calls
                records = [json.loads(line) for line in (proof / "mcp.jsonl").read_text().splitlines()]
                _, calls = recorded_calls(records)
                cancelled = [call for call in terminal_calls(calls)
                             if call["params"]["name"] == "unica.docs"
                             and (call["result"].get("data") or {}).get("task", {}).get("status") == "cancelled"]
                self.assertEqual(len(cancelled), 1, "no actual cancelled docs terminal result")
                task_id = cancelled[0]["result"]["data"]["task"]["taskId"]
                self.assertTrue(any(call["params"]["name"] == "unica.task.cancel"
                                    and call["params"]["arguments"].get("taskId") == task_id for call in calls))
            def finish(verification):
                (proof / "verification.json").write_text(json.dumps(verification, ensure_ascii=False, indent=2))
                independent_review(proof)
                print(f"accepted agent proof: {proof}", flush=True)
            return finish
        run_corpus(self, corpus, scenario_driver=driver)
