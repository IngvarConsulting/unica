"""Наблюдатель адресов на приёмочной фикстуре: дерево, `can`, контракт, предпросмотр.

Тест поднимает `scripts/dev/unica-observer.py` с `unica` и ходит только через
HTTP-API наблюдателя — тем же путём, что страница. Бинарь собирается через
`cargo build`, как в соседних тестах `medium`; готовый можно передать в
`UNICA_OBSERVER_BINARY`. Без Cargo и без готового бинаря тест пропускается
с названной причиной.
"""
from __future__ import annotations

import hashlib
import http.client
import json
import os
import queue
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

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "unica-observer.py"
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "acceptance" / "workspace"
# Скелет аргументов по ключу словаря `can` — как его строит реестр операций.
SKELETON = {"from": "", "items": [], "values": {}, "text": "", "at": ""}


def build_unica() -> Path:
    """Бинарь `unica`: из `UNICA_OBSERVER_BINARY` или собранный Cargo."""
    explicit = os.environ.get("UNICA_OBSERVER_BINARY")
    if explicit:
        return Path(explicit)
    if shutil.which("cargo") is None:
        raise unittest.SkipTest("нет cargo и не задан UNICA_OBSERVER_BINARY: unica собрать нечем")
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica",
         "--message-format=json-render-diagnostics"],
        cwd=REPO_ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return next(
        Path(message["executable"])
        for line in build.stdout.splitlines()
        if (message := json.loads(line)).get("reason") == "compiler-artifact"
        and message["target"]["name"] == "unica"
        and message.get("executable")
    )


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

    def sent_tool_calls(self) -> list[dict]:
        with urllib.request.urlopen(self.url + "/api/log?since=0", timeout=30) as response:
            entries = json.load(response)["entries"]
        return [
            entry["payload"]["params"]
            for entry in entries
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
                if not entry["implemented"]:
                    continue
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
