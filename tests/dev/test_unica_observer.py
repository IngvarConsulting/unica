"""Наблюдатель адресов на приёмочной фикстуре: дерево, `can`, контракт, предпросмотр.

Тест поднимает `scripts/dev/unica-observer.py` с собранным `unica` и ходит
только через HTTP-API наблюдателя — тем же путём, что страница. Бинарь
берётся из `UNICA_OBSERVER_BINARY`, `target/release/unica` или
`target/debug/unica`; без него тест пропускается с названной причиной.
"""
from __future__ import annotations

import hashlib
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


def unica_binary() -> Path:
    explicit = os.environ.get("UNICA_OBSERVER_BINARY")
    if explicit:
        return Path(explicit)
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = REPO_ROOT / target
    name = "unica.exe" if os.name == "nt" else "unica"
    # Release — умолчание наблюдателя; отладочная сборка тоже годится, её
    # собирают приёмочные наборы.
    for profile in ("release", "debug"):
        if (target / profile / name).is_file():
            return target / profile / name
    return target / "release" / name


def tree_digest(root: Path) -> dict[str, str]:
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


BINARY = unica_binary()


@unittest.skipUnless(BINARY.is_file(), f"нет собранного unica: {BINARY}; cargo build --release -p unica-coder --bin unica")
class UnicaObserverTests(unittest.TestCase):
    maxDiff = None

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
            [sys.executable, str(SCRIPT), "--workspace", str(self.workspace), "--unica", str(BINARY),
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

    def post(self, path: str, body: dict) -> tuple[int, dict]:
        request = urllib.request.Request(
            self.url + path, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}
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

    def test_tree_can_contract_and_preview_leave_the_workspace_unchanged(self) -> None:
        with urllib.request.urlopen(self.url + "/", timeout=30) as page:
            self.assertIn("Наблюдатель адресов", page.read().decode())

        root = self.view()
        self.assertTrue(root["ok"], root)
        source_sets = [entry["name"] for entry in root["data"]["sourceSets"]]
        self.assertIn("main", source_sets)

        # Раскрываем дерево по ветвям и элементам, как страница, до первого
        # узла объекта с непустым `can`, где есть исполнимая операция.
        pending = deque(f"{name}:Configuration" for name in source_sets)
        found = None
        visited = 0
        while pending and found is None and visited < 60:
            at = pending.popleft()
            visited += 1
            result = self.view(at)
            if not result["ok"]:
                continue
            data = result["data"]
            self.assertEqual(data["at"], at, "view answers the address it was asked")
            implemented = [entry for entry in data.get("can") or [] if entry["implemented"]]
            if implemented and "." in at.split(":", 1)[1]:
                found = (at, data, implemented)
                break
            for slot in ("branches", "items"):
                pending.extend(child["at"] for child in data.get(slot) or [] if "at" in child)
        self.assertIsNotNone(found, f"за {visited} узлов не нашлось объекта с can")
        at, data, implemented = found

        # Контракт операции по адресу `<узел>.Can.<op>`: берём первую
        # операцию, контракт которой опубликован.
        contracts = []
        for entry in implemented:
            contract = self.view(f"{at}.Can.{entry['op']}")
            if contract["ok"]:
                contracts.append((entry, contract))
                break
        self.assertTrue(contracts, f"ни у одной операции {at} нет контракта")
        entry, contract = contracts[0]
        self.assertEqual(contract["data"]["kind"], "Can")
        self.assertEqual(contract["data"]["props"]["op"], entry["op"])

        # Предпросмотр: план без токена строится и ничего не пишет.
        status, preview = self.post(
            "/api/apply",
            {"at": "main:Catalog.Валюты", "ops": [{"op": "props.set", "args": {"values": {"Comment": "observer"}}}]},
        )
        self.assertEqual(status, 200, preview)
        self.assertTrue(preview["result"]["ok"], preview)
        self.assertIn("executionToken", preview["result"]["data"], "a preview returns a plan token, not a write")

        # Исполнение токена API не пропускает ни отдельно, ни внутри плана.
        token = preview["result"]["data"]["executionToken"]
        for body in ({"executionToken": token},
                     {"at": "main:Catalog.Валюты", "ops": [], "executionToken": token}):
            status, refusal = self.post("/api/apply", body)
            self.assertEqual(status, 403, refusal)
        with urllib.request.urlopen(self.url + "/api/log?since=0", timeout=30) as response:
            sent = [entry["payload"] for entry in json.load(response)["entries"] if entry["dir"] == "out"]
        self.assertFalse(
            [payload for payload in sent if "executionToken" in json.dumps(payload)],
            "a token must never reach unica",
        )

        self.assertEqual(tree_digest(self.workspace), self.before, "observer must not change workspace files")


if __name__ == "__main__":
    unittest.main()
