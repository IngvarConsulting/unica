"""Наблюдатель адресов на приёмочной фикстуре: дерево, `can`, контракт, предпросмотр.

Тест поднимает `scripts/dev/unica-observer.py` с `unica` и ходит только через
HTTP-API наблюдателя — тем же путём, что страница. Бинарь берётся через
`ensure_binary`, как в соседних тестах `medium`; готовый можно передать в
`UNICA_OBSERVER_BINARY`. Без Cargo и без готового бинаря тест пропускается
с названной причиной.
"""
from __future__ import annotations

import hashlib
import http.client
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from collections import deque
from pathlib import Path
from scripts.ci.prepared_binary import ensure_binary

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "unica-observer.py"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "acceptance" / "workspace"
# Скелет аргументов по ключу словаря `can` — как его строит реестр операций.
SKELETON = {"from": "", "items": [], "values": {}, "text": "", "at": ""}


def build_unica() -> Path:
    """Бинарь `unica`: из манифеста CI, `UNICA_OBSERVER_BINARY` или `ensure_binary`."""
    if "UNICA_PREPARED_BINARY_MANIFEST" in os.environ:
        return ensure_binary(REPO_ROOT)
    explicit = os.environ.get("UNICA_OBSERVER_BINARY")
    if explicit:
        return Path(explicit)
    if os.environ.get("GITHUB_ACTIONS") != "true" and shutil.which("cargo") is None:
        raise unittest.SkipTest("нет cargo и не задан UNICA_OBSERVER_BINARY: unica собрать нечем")
    return ensure_binary(REPO_ROOT)


def tree_digest(root: Path) -> dict[str, str]:
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


class UnicaObserverTests(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls) -> None:
        cls.binary = build_unica()
        if not cls.binary.is_file():
            raise unittest.SkipTest(f"UNICA_OBSERVER_BINARY не указывает на файл: {cls.binary}")

    def setUp(self) -> None:
        scratch = tempfile.TemporaryDirectory(prefix="unica-observer-")
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name).resolve()
        self.workspace = root / "workspace"
        shutil.copytree(FIXTURE, self.workspace)
        state = root / "state"
        state.mkdir()
        env = dict(os.environ)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        env["UNICA_DAEMON_IDLE_GRACE_MS"] = "5000"
        self.before = tree_digest(self.workspace)
        self.process = subprocess.Popen(
            [sys.executable, str(SCRIPT), "--workspace", str(self.workspace), "--unica", str(self.binary),
             "--no-open", "--port", "0", "--timeout", "120"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self.addCleanup(self._stop)
        lines: queue.Queue[bytes] = queue.Queue()
        threading.Thread(target=lambda: lines.put(self.process.stdout.readline()), daemon=True).start()
        try:
            first = lines.get(timeout=120).decode().strip()
        except queue.Empty:
            self.fail("наблюдатель не напечатал адрес за 120 с")
        prefix = "UNICA_OBSERVER_URL="
        if not first.startswith(prefix):
            self._stop()
            self.fail(f"наблюдатель не поднялся: {first!r}; stderr: {self.process.stderr.read().decode()[-2000:]}")
        self.url = first[len(prefix):].rstrip("/")

    def _stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        for stream in (self.process.stdout, self.process.stderr):
            if stream and not stream.closed:
                stream.close()

    def post(self, path: str, body: dict, headers: dict | None = None) -> tuple[int, dict]:
        request = urllib.request.Request(
            self.url + path,
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json", **(headers or {})},
        )
        try:
            with urllib.request.urlopen(request, timeout=180) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            with error:
                return error.code, json.load(error)

    def view(self, at: str | None = None) -> dict:
        status, answer = self.post("/api/view", {} if at is None else {"at": at})
        self.assertEqual(status, 200, answer)
        self.assertIsInstance(answer["elapsedMs"], int)
        return answer["result"]

    def log_entries(self, since: int = 0) -> list[dict]:
        with urllib.request.urlopen(f"{self.url}/api/log?since={since}", timeout=30) as response:
            return json.load(response)["entries"]

    def last_seq(self) -> int:
        return max((entry["seq"] for entry in self.log_entries()), default=0)

    def sent_tool_calls(self, since: int = 0) -> list[dict]:
        return [
            entry["payload"]["params"]
            for entry in self.log_entries(since)
            if entry["dir"] == "out" and entry["payload"].get("method") == "tools/call"
        ]

    def test_tree_can_contract_and_preview_leave_the_workspace_unchanged(self) -> None:
        with urllib.request.urlopen(self.url + "/", timeout=30) as page:
            self.assertIn("Наблюдатель адресов", page.read().decode())

        root = self.view()
        self.assertTrue(root["ok"], root)
        source_sets = [entry["name"] for entry in root["data"]["sourceSets"]]
        self.assertIn("main", source_sets)

        # Раскрываем дерево по ветвям и элементам, как страница, до первого
        # объекта, у которого исполнимая операция из `can` отдаёт контракт.
        pending = deque(f"{name}:Configuration" for name in source_sets)
        found = None
        visited = 0
        while pending and found is None and visited < 80:
            at = pending.popleft()
            visited += 1
            result = self.view(at)
            if not result["ok"]:
                continue
            data = result["data"]
            self.assertEqual(data["at"], at, "view answers the address it was asked")
            for entry in data.get("can") or []:
                self.assertNotIn("implemented", entry)
                contract = self.view(f"{at}.Can.{entry['op']}")
                if contract["ok"]:
                    found = (at, entry, contract)
                    break
            for slot in ("branches", "items"):
                pending.extend(child["at"] for child in data.get(slot) or [] if "at" in child)
        self.assertIsNotNone(found, f"за {visited} узлов не нашлось операции с контрактом")
        at, entry, contract = found
        self.assertEqual(contract["data"]["kind"], "Can")
        self.assertEqual(contract["data"]["props"]["op"], entry["op"])
        self.assertEqual(contract["data"]["props"]["args"], entry["args"], "contract and can name one argument key")

        # Предпросмотр найденной операции на найденном узле со скелетом из
        # `can`. Пустой скелет может быть отвергнут значением, но не местом:
        # операция, объявленная узлом, применима к нему.
        args = {entry["args"]: SKELETON[entry["args"]]}
        status, preview = self.post("/api/apply", {"at": at, "ops": [{"op": entry["op"], "args": args}]})
        self.assertEqual(status, 200, preview)
        result = preview["result"]
        if result["ok"]:
            self.assertIn("executionToken", result["data"], "a preview returns a plan token, not a write")
        else:
            outcomes = {diagnostic.get("outcome") for diagnostic in result["diagnostics"]}
            self.assertEqual(outcomes, {"fixCall"}, result)
            self.assertNotIn("does not apply", json.dumps(result), result)

        # Токен API не пропускает ни отдельно, ни внутри плана, ни в аргументах
        # операции, ни через `extra` чтения.
        token = "observer-test-token"
        for path, body in (
            ("/api/apply", {"executionToken": token}),
            ("/api/apply", {"at": at, "ops": [], "executionToken": token}),
            ("/api/apply", {"at": at, "ops": [{"op": entry["op"], "args": {**args, "executionToken": token}}]}),
            ("/api/view", {"at": at, "extra": {"executionToken": token}}),
        ):
            status, refusal = self.post(path, body)
            self.assertEqual(status, 403, (path, body, refusal))
        # `extra` не подменяет адрес чтения.
        status, refusal = self.post("/api/view", {"at": at, "extra": {"at": "main:Configuration"}})
        self.assertEqual(status, 400, refusal)

        calls = self.sent_tool_calls()
        self.assertTrue(calls)
        self.assertFalse(
            [call for call in calls if "executionToken" in json.dumps(call["arguments"])],
            "a token must never reach unica, whatever the tool",
        )
        self.assertEqual(tree_digest(self.workspace), self.before, "observer must not change workspace files")

    def test_body_is_a_successful_text_leaf_without_source_changes(self) -> None:
        module = self.view("main:Catalog.Валюты.Module.Manager")
        at = next(branch["at"] for branch in module["data"]["branches"] if branch["at"].endswith(".Body"))
        expected = (self.workspace / "src/Catalogs/Валюты/Ext/ManagerModule.bsl").read_bytes().decode("utf-8-sig")
        for route, body in [("/api/view", {"at": at}),
                            ("/api/tool", {"tool": "unica.view", "arguments": {"at": at}})]:
            with self.subTest(route=route):
                status, answer = self.post(route, body)
                self.assertEqual(status, 200, answer)
                self.assertTrue(answer["result"]["ok"], answer)
                self.assertFalse(answer["isError"], answer)
                self.assertEqual(answer["sourceText"], expected)
                self.assertNotIn("branches", answer["result"]["data"])
                self.assertNotIn("items", answer["result"]["data"])
        self.assertEqual(tree_digest(self.workspace), self.before)

    def get(self, path: str) -> dict:
        with urllib.request.urlopen(self.url + path, timeout=30) as response:
            return json.load(response)

    def test_every_tool_answers_a_schema_valid_call_and_execution_stops_before_unica(self) -> None:
        catalog = self.get("/api/tools")
        tools = {tool["name"].removeprefix("unica."): tool for tool in catalog["tools"]}
        at = "main:Catalog.Валюты"
        foreign_task = "00000000-0000-4000-8000-000000000000"

        # Предпросмотр `run` пропускается и уходит в unica с `dryRun: true`.
        # Ответ может быть отказом поставщика (нет v8-runner), но вызов
        # состоялся; долгую операцию сервер передаёт в задачу наблюдателя.
        run_previews = (
            {"op": "extensions.list", "args": {}, "dryRun": True},
            {"op": "make", "args": {"output": "dist/main.cf"}, "dryRun": True},
        )
        mark = self.last_seq()
        for arguments in run_previews:
            with self.subTest(run_preview=arguments["op"]):
                status, answer = self.post("/api/tool", {"tool": "run", "arguments": arguments})
                self.assertEqual(status, 200, answer)
                self.assertIn("ok", answer["result"], answer)
        sent = [call for call in self.sent_tool_calls(mark) if call["name"].endswith("run")]
        self.assertEqual([call["arguments"]["op"] for call in sent], ["extensions.list", "make"])
        self.assertTrue(all(call["arguments"]["dryRun"] is True for call in sent), sent)
        own_tasks = {
            match
            for entry in self.log_entries(mark)
            if entry["dir"] == "in"
            for match in re.findall(r'"taskId":\s*"([0-9a-f-]{36})"', json.dumps(entry["payload"]))
        }
        self.assertTrue(own_tasks, "a run preview is handed off to a task the observer owns")
        own_task = sorted(own_tasks)[0]

        # По вызову на каждый инструмент поверхности: новый инструмент без
        # своего случая роняет тест, а не проходит молча.
        cases = {
            "view": {"at": at, "limit": 5},
            "apply": {"at": at, "ops": [{"op": "attribute.add", "args": {"items": [{"name": "ObserverProbe"}]}}]},
            "resolve": {"at": at},
            "search": {"query": "Валюта", "corpus": "names", "limit": 5},
            "check": {"at": at},
            "diff": {"left": at, "right": at},
            "run": {},
            "docs": {"query": "Запрос", "limit": 1},
            "task.get": {"taskId": own_task},
            "task.result": {"taskId": own_task, "waitMs": 0},
            "task.cancel": {"taskId": own_task},
        }
        self.assertEqual(set(tools), set(cases), "every tool from tools/list needs a schema-valid case")
        for verb, arguments in cases.items():
            with self.subTest(tool=verb):
                self.assertIsNotNone(tools[verb]["policy"], f"{verb} is classified by the observer policy")
                status, answer = self.post("/api/tool", {"tool": tools[verb]["name"], "arguments": arguments})
                self.assertEqual(status, 200, answer)
                self.assertEqual(answer["arguments"], arguments)
                self.assertIsInstance(answer["elapsedMs"], int)
                self.assertIsInstance(answer["result"], dict)
                self.assertIn("ok", answer["result"], answer)

        # Нарушение схемы — 400, неизвестный инструмент — 404; исполнение,
        # запись и чужие задачи — 403. Ни один из них не доходит до unica.
        token = "observer-test-token"
        refused = (
            ({"tool": "search", "arguments": {}}, 400),
            ({"tool": "view", "arguments": {"at": at, "limit": 0}}, 400),
            ({"tool": "diff", "arguments": {"left": at, "right": at, "extra": 1}}, 400),
            ({"tool": "unica.write", "arguments": {}}, 404),
            ({"tool": "apply", "arguments": {"executionToken": token}}, 403),
            ({"tool": "apply", "arguments": {"at": at, "ops": [{"op": "attribute.add", "args": {"executionToken": token}}]}}, 403),
            ({"tool": "apply", "arguments": {"at": at, "ops": [{"op": "attribute.remove"}], "dryRun": False}}, 403),
            ({"tool": "apply", "arguments": {"at": at, "ops": [{"op": "attribute.remove"}], "ifRev": "rev"}}, 403),
            ({"tool": "search", "arguments": {"query": "x", "executionToken": token}}, 403),
            ({"tool": "run", "arguments": {"op": "push", "args": {}, "dryRun": False}}, 403),
            ({"tool": "run", "arguments": {"op": "push", "args": {}}}, 403),
            ({"tool": "run", "arguments": {"op": "make", "args": {"output": "dist/main.cf"}, "dryRun": "true"}}, 403),
            ({"tool": "run", "arguments": {"op": "launch", "args": {"clientMode": "thin"}, "dryRun": True}}, 403),
            ({"tool": "run", "arguments": {"op": "unknown.op", "args": {}, "dryRun": True}}, 403),
            ({"tool": "run", "arguments": {"dryRun": False}}, 403),
            ({"tool": "task.get", "arguments": {"taskId": foreign_task}}, 403),
            ({"tool": "task.result", "arguments": {"taskId": foreign_task}}, 403),
            ({"tool": "task.cancel", "arguments": {"taskId": foreign_task}}, 403),
        )
        mark = self.last_seq()
        for body, expected in refused:
            with self.subTest(refused=body):
                status, refusal = self.post("/api/tool", body)
                self.assertEqual(status, expected, (body, refusal))
        self.assertEqual(self.sent_tool_calls(mark), [], "refused calls never reach unica")

        for call in self.sent_tool_calls():
            arguments = call["arguments"]
            self.assertNotIn("executionToken", json.dumps(arguments), call)
            if call["name"].endswith("run") and "op" in arguments:
                self.assertIs(arguments.get("dryRun"), True, call)
                self.assertNotEqual(arguments["op"], "launch", call)
            if call["name"].endswith("apply"):
                self.assertLessEqual(set(arguments), {"at", "ops"}, call)
        self.assertEqual(tree_digest(self.workspace), self.before, "observer must not change workspace files")

    def test_contract_drafts_pass_preview_per_family_and_a_missing_contract_is_named(self) -> None:
        # Семейства с контрактом, чей черновик на фикстуре предпросмотр не
        # проходит, с причиной. Закрытый пробел роняет тест: запись удаляется.
        known_gaps = {
            # Пример контракта связывает наборы Left и Right, а в схеме
            # фикстуры один набор данных: имена надо брать из узла.
            "dataSetLink": "contract example names data sets absent from the node",
        }
        root = self.view()
        pending = deque(f"{entry['name']}:Configuration" for entry in root["data"]["sourceSets"])
        seen: set[str] = set()
        passed: dict[str, str] = {}
        failed: dict[str, list] = {}
        unavailable_ops: set[str] = set()
        while pending:
            at = pending.popleft()
            if at in seen or len(seen) >= 600:
                continue
            seen.add(at)
            status, answer = self.post("/api/view", {"at": at, "extra": {"limit": 50}})
            self.assertEqual(status, 200, answer)
            result = answer["result"]
            if not result["ok"]:
                continue
            data = result["data"]
            for entry in data.get("can") or []:
                op = entry["op"]
                family = op.split(".", 1)[0]
                if family in passed or op in unavailable_ops:
                    continue
                status, draft = self.post("/api/draft", {"at": data["at"], "op": op})
                self.assertEqual(status, 200, draft)
                self.assertEqual(draft["draft"]["at"], data["at"])
                self.assertEqual(draft["draft"]["ops"][0]["op"], op)
                if draft["contract"] == "unavailable":
                    self.assertEqual(draft["note"], "контракт недоступен (#1468)", draft)
                    unavailable_ops.add(op)
                    continue
                if draft["contract"] != "ok":
                    continue
                self.assertIsNone(draft["note"], draft)
                self.assertEqual(draft["validation"], [], draft)
                status, preview = self.post("/api/apply", draft["draft"])
                self.assertEqual(status, 200, preview)
                if preview["result"]["ok"]:
                    self.assertIn("executionToken", preview["result"]["data"])
                    passed[family] = op
                    failed.pop(family, None)
                else:
                    failed.setdefault(family, []).append((data["at"], op, preview["result"]["summary"]))
            for slot in ("branches", "items"):
                pending.extend(
                    child["at"]
                    for child in data.get(slot) or []
                    if isinstance(child, dict) and "at" in child and not child["at"].endswith((".Caller", ".Callee"))
                )
        self.assertGreaterEqual(len(passed), 10, passed)
        self.assertEqual(set(failed), set(known_gaps), failed)
        self.assertTrue(unavailable_ops, "the fixture has implemented operations without a contract")
        self.assertEqual(tree_digest(self.workspace), self.before, "previews must not change workspace files")

    def test_foreign_origin_and_bad_length_are_refused(self) -> None:
        status, refusal = self.post("/api/view", {}, headers={"Origin": "http://evil.example"})
        self.assertEqual(status, 403, refusal)
        host, port = self.url.removeprefix("http://").split(":")
        for length in ("-1", "abc"):
            connection = http.client.HTTPConnection(host, int(port), timeout=30)
            try:
                connection.putrequest("POST", "/api/view")
                connection.putheader("Content-Type", "application/json")
                connection.putheader("Content-Length", length)
                connection.endheaders()
                response = connection.getresponse()
                self.assertEqual(response.status, 400, length)
                self.assertIn("Content-Length", json.load(response)["error"])
            finally:
                connection.close()


if __name__ == "__main__":
    unittest.main()
