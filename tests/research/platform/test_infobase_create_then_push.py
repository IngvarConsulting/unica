"""Создание базы продуктом, затем `push` без `force` на настоящей платформе.

Раннер записывает память рабочей копии о базе, которую создаёт
`infobase create` без `--from`, поэтому первый `push` в неё идёт без
`--force` (правило `INV.WIRE.INFOBASE-CREATE-RECREATES-BY-CHOICE`).
Тест проверяет это через проводной `unica.run`, как его видит агент:

1. `infobase.create` создаёт файловую базу из набора `main`;
2. `push` сразу после создания не отказывает `untracked_base`, а пропускает
   неизменённый набор;
3. после правки исходников `push` без `force` загружает набор в базу.

Отдельный сценарий проверяет чтение итога `push` после переподключения
к демону с тем же каталогом состояния по протоколу MCP 2025-06-18.

Исследовательский тест на настоящей платформе. В обычных прогонах он
пропускается; вручную его запускает
`scripts/dev/run-research.py --profile platform`. Нужны переменные:

- `UNICA_BIN` — бинарь `unica` внутри собранного пакета плагина:
  рядом должен лежать `third-party/manifest.json` с закреплённым
  v8-runner. Пакет из набора инструментов собирает
  `scripts/ci/package-unica-plugin.py`;
- `UNICA_PLATFORM_BUILD` — установленная сборка платформы, например
  `8.3.27.2342`.

Демон запускается со своим каталогом состояния во временном каталоге.
"""
from __future__ import annotations

import hashlib
import json
import os
import queue
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "tests/fixtures/formats/platform_xml/2.20/empty_configuration/src"
BIN_ENV = "UNICA_BIN"
BUILD_ENV = "UNICA_PLATFORM_BUILD"
PROTOCOL_VERSION = "2025-11-25"
# Создание файловой базы и загрузка занимают десятки секунд.
CALL_TIMEOUT_SECONDS = 600
DAEMON_IDLE_GRACE_MS = 1000
DAEMON_SHUTDOWN_SECONDS = 15
HOST_PROJECT_VARIABLES = ("CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR")


class McpClient:
    """Минимальный MCP-клиент по stdio: запросы по очереди."""

    def __init__(self, binary: Path, workspace: Path, state: Path) -> None:
        env = dict(os.environ)
        for name in HOST_PROJECT_VARIABLES:
            env.pop(name, None)
        env["UNICA_PROVIDER_STATE_DIR"] = str(state)
        env["UNICA_DAEMON_IDLE_GRACE_MS"] = str(DAEMON_IDLE_GRACE_MS)
        self.stderr = tempfile.TemporaryFile()
        self.process = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            cwd=workspace,
            env=env,
        )
        self.messages: queue.Queue[dict[str, Any]] = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        self.next_id = 0

    def initialize(self, protocol_version: str) -> dict[str, Any]:
        result = self.request(
            "initialize",
            {
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {"name": "unica-research", "version": "0"},
            },
        )
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return result

    def _read(self) -> None:
        assert self.process.stdout is not None
        for raw in iter(self.process.stdout.readline, b""):
            raw = raw.strip()
            if raw:
                self.messages.put(json.loads(raw))

    def _send(self, payload: dict[str, Any]) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write((json.dumps(payload, ensure_ascii=False) + "\n").encode())
        self.process.stdin.flush()

    def request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        self.next_id += 1
        request_id = self.next_id
        self._send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        while True:
            try:
                message = self.messages.get(timeout=CALL_TIMEOUT_SECONDS)
            except queue.Empty:
                raise AssertionError(f"{method}: нет ответа; stderr: {self.stderr_tail()}") from None
            if message.get("id") == request_id and "method" not in message:
                if "error" in message:
                    raise AssertionError(f"{method}: {message['error']}")
                return message["result"]

    def call(self, tool: str, arguments: dict[str, Any]) -> dict[str, Any]:
        return self.request("tools/call", {"name": tool, "arguments": arguments})

    def run(self, op: str, args: dict[str, Any], *, dry_run: bool = False) -> dict[str, Any]:
        """Вызов до итога: долгая операция становится задачей, её ждём."""
        result = self.call("unica.run", {"op": op, "args": args, "dryRun": dry_run})
        task = task_snapshot(result)
        if task is None:
            return result
        return self.task_result(task["taskId"])

    def task_result(self, task_id: str) -> dict[str, Any]:
        """Читает существующее задание, в том числе из нового сеанса."""
        deadline = time.monotonic() + CALL_TIMEOUT_SECONDS
        while True:
            result = self.call("unica.task.result", {"taskId": task_id, "waitMs": 7000})
            task = task_snapshot(result)
            if task is None:
                return result
            assert task.get("taskId") == task_id, outcome(result)
            assert time.monotonic() < deadline, f"{task_id}: задача не завершилась; {outcome(result)}"

    def stderr_tail(self) -> str:
        self.stderr.seek(0)
        return self.stderr.read()[-4000:].decode("utf-8", "replace")

    def close(self) -> None:
        if self.process.stdin is not None:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        if self.process.stdout is not None:
            self.process.stdout.close()
        self.stderr.close()


def outcome(result: dict[str, Any]) -> str:
    """Ответ целиком для сообщения об ошибке."""
    return json.dumps(result, ensure_ascii=False, indent=1)[:6000]


def task_snapshot(result: dict[str, Any]) -> dict[str, Any] | None:
    content = result.get("structuredContent") or {}
    if content.get("ok") is True:
        return (content.get("data") or {}).get("task")
    return None


def main_step(result: dict[str, Any]) -> dict[str, Any]:
    steps = result["structuredContent"]["data"]["steps"]
    found = [step for step in steps if step.get("sourceSet") == "main"]
    assert len(found) == 1, outcome(result)
    return found[0]


@unittest.skipUnless(
    os.environ.get(BIN_ENV) and os.environ.get(BUILD_ENV),
    f"нужна установленная платформа 1С: задайте {BIN_ENV} и {BUILD_ENV}",
)
class InfobaseCreateThenPush(unittest.TestCase):
    def setUp(self) -> None:
        self.binary = Path(os.environ[BIN_ENV]).resolve()
        self.assertTrue(self.binary.is_file(), f"{BIN_ENV}={self.binary}: нет файла")
        # Реальный путь: демон не открывает состояние через символические ссылки.
        self.root = Path(tempfile.mkdtemp(prefix="unica-create-push-")).resolve()
        self.state = self.root / "state"
        self.addCleanup(self.remove_workspace)
        self.workspace = self.root / "ws"
        shutil.copytree(FIXTURE, self.workspace / "src")
        (self.workspace / "v8project.yaml").write_text(
            "workPath: .work\n"
            "format: DESIGNER\n"
            "tools:\n"
            "  platform:\n"
            f"    version: '{os.environ[BUILD_ENV]}'\n"
            "  designer_agent:\n"
            "    live: false\n"
            "source-set:\n"
            "  - name: main\n"
            "    type: CONFIGURATION\n"
            "    path: src\n",
            encoding="utf-8",
        )
        (self.workspace / "v8project.local.yaml").write_text(
            "infobases:\n"
            "  origin:\n"
            f"""    connection: '/F "{self.root / 'ib'}"'\n""",
            encoding="utf-8",
        )
        self.state.mkdir()
        self.client = self.open_client()

    def remove_workspace(self) -> None:
        deadline = time.monotonic() + DAEMON_SHUTDOWN_SECONDS
        while True:
            if any(self.state.glob("daemon-p5-*/endpoint.json")):
                self.assertLess(time.monotonic(), deadline, "the isolated daemon did not stop")
            else:
                try:
                    shutil.rmtree(self.root)
                    return
                except PermissionError:
                    if time.monotonic() >= deadline:
                        raise
            time.sleep(0.05)

    def open_client(self, protocol_version: str = PROTOCOL_VERSION) -> McpClient:
        client = McpClient(self.binary, self.workspace, self.state)
        self.addCleanup(client.close)
        initialization = client.initialize(protocol_version)
        self.assertEqual(initialization["protocolVersion"], protocol_version)
        return client

    def assert_ok(self, step: str, result: dict[str, Any]) -> None:
        self.assertNotEqual(
            result.get("isError"), True, f"{step}: {outcome(result)}\nstderr: {self.client.stderr_tail()}"
        )

    def test_check_reads_the_selected_origin_without_changing_workspace_files(self) -> None:
        def workspace_files() -> dict[Path, str]:
            return {
                path.relative_to(self.workspace): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in self.workspace.rglob("*") if path.is_file()
            }

        project = self.workspace / "v8project.yaml"
        pinned = project.read_bytes()
        # The first lookup must not persist an inferred tools.platform.version.
        unpinned = "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
        project.write_text(unpinned, encoding="utf-8")
        initial = workspace_files()
        preview = self.client.call("unica.run", {"op": "infobase.check", "args": {}, "dryRun": True})
        self.assert_ok("preview missing origin", preview)
        missing = self.client.run("infobase.check", {})
        self.assertIs(missing["structuredContent"]["ok"], False)
        self.assertEqual(missing["structuredContent"]["data"]["runnerCode"], "infobase_not_found")
        self.assertEqual(workspace_files(), initial)

        project.write_bytes(pinned)
        self.assert_ok("create temporary origin", self.client.run("infobase.create", {}))
        bound = workspace_files()
        for _ in range(2):
            checked = self.client.run("infobase.check", {})["structuredContent"]
            self.assertIs(checked["ok"], True)
            self.assertEqual(checked["data"]["platformVersion"], os.environ[BUILD_ENV])
            self.assertEqual(workspace_files(), bound)

    def edit_configuration_comment(self) -> None:
        configuration = self.workspace / "src" / "Configuration.xml"
        text = configuration.read_text(encoding="utf-8-sig")
        self.assertEqual(text.count("<Comment/>"), 1)
        configuration.write_text(
            text.replace("<Comment/>", "<Comment>create then push</Comment>"), encoding="utf-8-sig"
        )

    def test_a_created_infobase_takes_a_push_without_force(self) -> None:
        created = self.client.run("infobase.create", {})
        self.assert_ok("infobase.create", created)
        data = created["structuredContent"]["data"]
        self.assertTrue(data.get("initializesSources"), outcome(created))
        self.assertEqual(data.get("sourceSet"), "main", outcome(created))

        unchanged = self.client.run("push", {"sourceSet": "main"})
        self.assert_ok("push after create", unchanged)
        self.assertEqual(main_step(unchanged).get("mode"), "skipped", outcome(unchanged))

        self.edit_configuration_comment()

        loaded = self.client.run("push", {"sourceSet": "main"})
        self.assert_ok("push after an edit", loaded)
        self.assertNotEqual(main_step(loaded).get("mode"), "skipped", outcome(loaded))
        changed = loaded["structuredContent"].get("changed") or []
        self.assertTrue(
            any(entry.get("infobase") and entry.get("sourceSet") == "main" for entry in changed),
            outcome(loaded),
        )

    def test_reconnected_client_reads_the_same_push_result(self) -> None:
        # rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
        created = self.client.run("infobase.create", {})
        self.assert_ok("infobase.create", created)
        self.assertTrue((self.root / "ib" / "1Cv8.1CD").is_file(), outcome(created))
        self.edit_configuration_comment()

        protocol_version = "2025-06-18"
        self.client.close()
        self.client = self.open_client(protocol_version)
        args = {"sourceSet": "main"}
        preview = self.client.run("push", args, dry_run=True)
        self.assert_ok("push preview", preview)
        plan = preview["structuredContent"]["data"]["plan"]
        self.assertEqual(plan["steps"], [{"sourceSet": "main", "files": 1, "mode": "partial"}])

        received = self.client.call("unica.run", {"op": "push", "args": args, "dryRun": False})
        task = task_snapshot(received)
        self.assertIsNotNone(task, outcome(received))
        task_id = task["taskId"]
        self.assertTrue(task_id, outcome(received))
        self.client.close()

        self.client = self.open_client(protocol_version)
        loaded = self.client.task_result(task_id)
        self.assert_ok("result of the original push", loaded)
        data = loaded["structuredContent"]["data"]
        self.assertTrue(data["providerDispatched"], outcome(loaded))
        self.assertTrue(data["appliesDatabaseConfiguration"], outcome(loaded))
        self.assertEqual(data["steps"], plan["steps"], outcome(loaded))
        self.assertEqual(self.client.task_result(task_id), loaded)


if __name__ == "__main__":
    unittest.main()
