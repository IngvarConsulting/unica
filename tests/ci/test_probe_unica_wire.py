from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[2]
PROBE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "probe-unica-wire.py"
BASELINE_FIXTURE = (
    REPO_ROOT / "tests" / "fixtures" / "migration" / "v0.12.3-baseline.json"
)
# Предохранитель от зависания пробы, а не проверяемый срок: фейки ниже
# отвечают сразу либо завершают пробу событием, так что этот срок
# не определяет длительность зелёного прогона.
PROBE_FUSE_SECONDS = 30.0
# Сколько живёт держатель трубы, если его никто не убил. Граница
# «очистка не ждала держателя» выражена долей этой величины, а не
# скоростью машины: ожидание EOF длилось бы не меньше всего срока жизни.
HOLDER_LIFETIME_SECONDS = 60.0
HOLDER_BOUND_SECONDS = HOLDER_LIFETIME_SECONDS / 2
# Бюджет старта фейка для проверки истечения общего срока: срок должен
# истечь уже после того, как фейк породил потомка и записал pids.txt.
# Сам срок здесь не контракт — контракт «по истечении срока дерево убрано».
FAKE_START_BUDGET_SECONDS = 2.0
RUNTIME_JOB_NAMES = [
    "unica.runtime.job.cancel",
    "unica.runtime.job.list",
    "unica.runtime.job.logs",
    "unica.runtime.job.start",
    "unica.runtime.job.status",
    "unica.runtime.job.wait",
]


def load_module():
    spec = importlib.util.spec_from_file_location("unica_wire_probe", PROBE_SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class WireProbeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.assertTrue(PROBE_SCRIPT.is_file(), "wire probe script is missing")

    def write_server(
        self,
        root: Path,
        *,
        pages: list[list[str]],
        capture_path: Path,
    ) -> Path:
        server = root / "wire-server.py"
        server.write_text(
            textwrap.dedent(
                f"""
                import json
                import sys
                from pathlib import Path

                pages = {pages!r}
                capture_path = Path({str(capture_path)!r})
                captured = []

                for line in sys.stdin:
                    message = json.loads(line)
                    captured.append(message)
                    capture_path.write_text(
                        json.dumps(captured, sort_keys=True) + "\\n",
                        encoding="utf-8",
                    )
                    method = message.get("method")
                    request_id = message.get("id")
                    if method == "initialize":
                        result = {{
                            "protocolVersion": message["params"]["protocolVersion"],
                            "capabilities": {{}},
                            "serverInfo": {{"name": "unica", "version": "0.12.3"}},
                        }}
                        print(json.dumps({{"jsonrpc": "2.0", "id": request_id, "result": result}}), flush=True)
                    elif method == "tools/list":
                        cursor = message.get("params", {{}}).get("cursor")
                        page_index = 0 if cursor is None else int(cursor)
                        result = {{"tools": [{{"name": name}} for name in pages[page_index]]}}
                        if page_index + 1 < len(pages):
                            result["nextCursor"] = str(page_index + 1)
                        print(json.dumps({{"jsonrpc": "2.0", "id": request_id, "result": result}}), flush=True)
                """
            ),
            encoding="utf-8",
        )
        return server

    def run_probe(
        self,
        server: Path,
        output: Path,
        *,
        tasks_capability: str = "off",
        protocol_version: str = "2025-06-18",
        profile: str | None = None,
        target: str | None = None,
        timeout_seconds: float = PROBE_FUSE_SECONDS,
    ) -> subprocess.CompletedProcess[str]:
        command = [
                sys.executable,
                str(PROBE_SCRIPT),
                "--binary",
                sys.executable,
                "--binary-arg",
                str(server),
                "--protocol-version",
                protocol_version,
                "--tasks-capability",
                tasks_capability,
                "--output",
                str(output),
                "--timeout-seconds",
                str(timeout_seconds),
            ]
        if profile is not None:
            command.extend(["--profile", profile])
        if target is not None:
            command.extend(["--target", target])
        return subprocess.run(
            command,
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_seconds + PROBE_FUSE_SECONDS,
        )

    def probe_command(self, server: Path, output: Path, timeout_seconds: float) -> list[str]:
        return [
            sys.executable,
            str(PROBE_SCRIPT),
            "--binary",
            sys.executable,
            "--binary-arg",
            str(server),
            "--protocol-version",
            "2025-06-18",
            "--tasks-capability",
            "off",
            "--output",
            str(output),
            "--timeout-seconds",
            str(timeout_seconds),
        ]

    def read_pids(self, pid_path: Path) -> list[int]:
        self.assertTrue(
            pid_path.is_file(),
            "the fake server did not record its process tree before cleanup; "
            "the scenario under test never started",
        )
        return [int(value) for value in pid_path.read_text(encoding="utf-8").split()]

    def kill_leftovers(self, module, pid_path: Path) -> None:
        if not pid_path.exists():
            return
        pids = {int(value) for value in pid_path.read_text(encoding="utf-8").split()}
        for pid in module._running_pids(pids):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        module._wait_for_process_pids(pids, PROBE_FUSE_SECONDS)

    def test_initialize_uses_the_explicit_legacy_protocol(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = root / "requests.json"
            server = self.write_server(
                root,
                pages=[["unica.alpha"]],
                capture_path=capture,
            )
            result = self.run_probe(server, root / "wire.json", tasks_capability="off")
            requests = json.loads(capture.read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(requests[0]["method"], "initialize")
        self.assertEqual(requests[0]["params"]["protocolVersion"], "2025-06-18")
        self.assertEqual(requests[0]["params"]["capabilities"], {})

    def test_modern_protocol_is_direct_first_and_carries_tasks_on_every_page(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = root / "requests.json"
            server = root / "modern-wire-server.py"
            server.write_text(
                textwrap.dedent(
                    f"""
                    import json
                    import sys
                    from pathlib import Path

                    captured = []
                    capture = Path({str(capture)!r})
                    pages = [["unica.alpha"], ["unica.beta"]]
                    for line in sys.stdin:
                        message = json.loads(line)
                        captured.append(message)
                        capture.write_text(json.dumps(captured) + "\\n", encoding="utf-8")
                        cursor = message["params"].get("cursor")
                        page = 0 if cursor is None else int(cursor)
                        result = {{
                            "resultType": "complete",
                            "tools": [{{"name": name}} for name in pages[page]],
                            "_meta": {{
                                "io.modelcontextprotocol/serverInfo": {{
                                    "name": "unica", "version": "modern-test"
                                }}
                            }},
                        }}
                        if page == 0:
                            result["nextCursor"] = "1"
                        print(json.dumps({{
                            "jsonrpc": "2.0", "id": message["id"], "result": result
                        }}), flush=True)
                    """
                ),
                encoding="utf-8",
            )
            result = self.run_probe(
                server,
                root / "wire.json",
                tasks_capability="on",
                protocol_version="2026-07-28",
            )
            requests = json.loads(capture.read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([request["method"] for request in requests], ["tools/list", "tools/list"])
        expected_meta = {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {
                "name": "unica-wire-probe",
                "version": "1",
            },
            "io.modelcontextprotocol/clientCapabilities": {
                "extensions": {"io.modelcontextprotocol/tasks": {}}
            },
        }
        self.assertEqual(requests[0]["params"], {"_meta": expected_meta})
        self.assertEqual(
            requests[1]["params"],
            {"_meta": expected_meta, "cursor": "1"},
        )

    def test_profiled_probe_writes_a_versioned_p0_evidence_envelope(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = self.write_server(
                root,
                pages=[["unica.view"]],
                capture_path=root / "requests.json",
            )
            output = root / "wire.json"
            result = self.run_probe(
                server,
                output,
                profile="native",
                target="linux-x64",
                tasks_capability="on",
                protocol_version="2026-07-28",
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            evidence = json.loads(output.read_text(encoding="utf-8"))

        self.assertEqual(evidence["schemaVersion"], 1)
        self.assertEqual(evidence["profile"], "native")
        self.assertEqual(evidence["target"], "linux-x64")

    def test_exhausts_direct_first_pagination_and_writes_deterministic_json(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = root / "requests.json"
            server = self.write_server(
                root,
                pages=[["unica.zeta", "unica.alpha"], ["unica.beta"]],
                capture_path=capture,
            )
            first = root / "first.json"
            first_result = self.run_probe(server, first)
            first_bytes = first.read_bytes()
            first_requests = json.loads(capture.read_text(encoding="utf-8"))

            capture.unlink()
            second = root / "second.json"
            second_result = self.run_probe(server, second)

            self.assertEqual(first_result.returncode, 0, first_result.stderr)
            self.assertEqual(second_result.returncode, 0, second_result.stderr)
            second_bytes = second.read_bytes()

        self.assertEqual(first_bytes, second_bytes)
        self.assertEqual(
            [(request["method"], request.get("params")) for request in first_requests],
            [
                (
                    "initialize",
                    {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "unica-wire-probe", "version": "1"},
                    },
                ),
                ("notifications/initialized", {}),
                ("tools/list", {}),
                ("tools/list", {"cursor": "1"}),
            ],
        )
        self.assertEqual(
            json.loads(first_bytes),
            {
                "protocolVersion": "2025-06-18",
                "responseKinds": [
                    {"kind": "result", "method": "initialize"},
                    {"kind": "result", "method": "tools/list"},
                    {"kind": "result", "method": "tools/list"},
                ],
                "serverInfo": {"name": "unica", "version": "0.12.3"},
                "serverProtocolVersion": "2025-06-18",
                "tasksCapability": "off",
                "toolCount": 3,
                "toolNames": ["unica.alpha", "unica.beta", "unica.zeta"],
            },
        )

    def test_duplicate_tool_name_across_pages_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = self.write_server(
                root,
                pages=[["unica.alpha"], ["unica.alpha"]],
                capture_path=root / "requests.json",
            )
            result = self.run_probe(server, root / "wire.json")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("duplicate tool name", result.stderr)
        self.assertIn("unica.alpha", result.stderr)

    def test_repeated_next_cursor_is_rejected_before_a_third_list_request(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = root / "requests.json"
            server = root / "repeated-cursor-server.py"
            server.write_text(
                textwrap.dedent(
                    f"""
                    import json
                    import sys
                    from pathlib import Path

                    captured = []
                    capture = Path({str(capture)!r})
                    for line in sys.stdin:
                        message = json.loads(line)
                        captured.append(message)
                        capture.write_text(json.dumps(captured) + "\\n", encoding="utf-8")
                        if message.get("method") == "initialize":
                            result = {{
                                "protocolVersion": "2025-06-18",
                                "capabilities": {{}},
                                "serverInfo": {{"name": "unica", "version": "test"}},
                            }}
                        elif message.get("method") == "tools/list":
                            result = {{"tools": [], "nextCursor": "same-cursor"}}
                        else:
                            continue
                        print(json.dumps({{
                            "jsonrpc": "2.0", "id": message["id"], "result": result
                        }}), flush=True)
                    """
                ),
                encoding="utf-8",
            )
            result = self.run_probe(server, root / "wire.json")
            requests = json.loads(capture.read_text(encoding="utf-8"))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("repeated cursor", result.stderr)
        self.assertEqual(
            [request["method"] for request in requests].count("tools/list"),
            2,
        )

    def test_notifications_do_not_restart_the_aggregate_deadline(self) -> None:
        module = load_module()

        class TimedResponses:
            def __init__(self, responses):
                self.responses = responses
                self.now = 0.0
                self.timeouts = []

            def get(self, *, timeout):
                self.timeouts.append(timeout)
                wake_at = self.now + timeout
                if not self.responses or self.responses[0][0] > wake_at:
                    self.now = wake_at
                    raise module.queue.Empty
                arrival, response = self.responses.pop(0)
                self.now = max(self.now, arrival)
                return json.dumps(response) + "\n"

        first = {"jsonrpc": "2.0", "id": 1, "result": {"tools": [], "nextCursor": "next"}}
        second = {"jsonrpc": "2.0", "id": 2, "result": {"tools": []}}
        for second_arrival in (11.0, 9.5):
            with self.subTest(second_arrival=second_arrival):
                lines = TimedResponses([
                    (2.0, first),
                    *[(arrival, {"jsonrpc": "2.0", "method": "notifications/progress"})
                      for arrival in (4.0, 7.0, 9.0)],
                    (second_arrival, second),
                ])
                session = module.JsonRpcSession.__new__(module.JsonRpcSession)
                session.process = SimpleNamespace(stdin=io.StringIO())
                session.deadline = 10.0
                session.timeout_seconds = None
                session.lines = lines
                session.diagnostics = []
                session.response_kinds = []
                with patch.object(module, "time", SimpleNamespace(monotonic=lambda: lines.now)):
                    self.assertEqual(session.request({"id": 1, "method": "tools/list"}), first)
                    self.assertEqual(lines.now, 2.0)
                    request = {"id": 2, "method": "tools/list", "params": {"cursor": "next"}}
                    if second_arrival > session.deadline:
                        with self.assertRaisesRegex(SystemExit, "aggregate deadline"):
                            session.request(request)
                        self.assertEqual(lines.now, 10.0)
                        self.assertEqual(lines.responses, [(11.0, second)])
                        self.assertEqual(len(session.response_kinds), 1)
                    else:
                        self.assertEqual(session.request(request), second)
                        self.assertEqual(lines.now, second_arrival)
                        self.assertEqual(lines.responses, [])
                        self.assertEqual(len(session.response_kinds), 2)
                self.assertEqual(lines.timeouts, [10.0, 8.0, 6.0, 3.0, 1.0])

    def test_aggregate_timeout_reaps_the_detached_process_tree(self) -> None:
        # Общий срок — проверяемое свойство: фейк заведомо не отвечает на
        # tools/list и только шлёт уведомления. Его величина — бюджет старта
        # фейка, чтобы срок истёк после записи pids.txt.
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "pids.txt"
            server = root / "tree-server.py"
            server.write_text(
                textwrap.dedent(
                    f"""
                    import json
                    import os
                    import signal
                    import subprocess
                    import sys
                    import time
                    from pathlib import Path

                    child = subprocess.Popen(
                        [
                            sys.executable,
                            "-c",
                            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                            "time.sleep({HOLDER_LIFETIME_SECONDS!r})",
                        ],
                        start_new_session=True,
                    )
                    # Атомарно: срок может оборвать фейк посреди записи.
                    staged = Path({str(pid_path)!r} + ".tmp")
                    staged.write_text(f"{{os.getpid()}} {{child.pid}}", encoding="utf-8")
                    os.replace(staged, {str(pid_path)!r})
                    for line in sys.stdin:
                        message = json.loads(line)
                        if message.get("method") == "initialize":
                            print(json.dumps({{
                                "jsonrpc": "2.0",
                                "id": message["id"],
                                "result": {{
                                    "protocolVersion": "2025-06-18",
                                    "capabilities": {{}},
                                    "serverInfo": {{"name": "unica", "version": "test"}},
                                }},
                            }}), flush=True)
                        elif message.get("method") == "tools/list":
                            while True:
                                print(json.dumps({{
                                    "jsonrpc": "2.0",
                                    "method": "notifications/progress",
                                    "params": {{}},
                                }}), flush=True)
                                time.sleep(0.02)
                    """
                ),
                encoding="utf-8",
            )
            try:
                result = self.run_probe(
                    server,
                    root / "wire.json",
                    timeout_seconds=FAKE_START_BUDGET_SECONDS,
                )
                pids = self.read_pids(pid_path)
                # Добитый SIGKILL процесс исчезает асинхронно; без очистки
                # держатель прожил бы весь свой срок, а не долю его.
                module._wait_for_process_pids(set(pids), HOLDER_BOUND_SECONDS)
                running = sorted(module._running_pids(set(pids)))
            finally:
                self.kill_leftovers(module, pid_path)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aggregate deadline", result.stderr)
        self.assertEqual(running, [], f"probe process tree survived: {running}")

    @unittest.skipUnless(os.name == "posix", "POSIX session ownership regression")
    def test_parent_exit_before_snapshot_has_bounded_cleanup_and_reaps_orphan(self) -> None:
        # Порядок задают события, а не срок: сирота отвечает ошибкой на
        # initialize только после выхода родителя, и проба уходит в очистку
        # уже без него. Сирота держит stdout и игнорирует SIGTERM; если бы
        # очистка ждала EOF трубы, проба жила бы весь срок жизни сироты.
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "pids.txt"
            server = root / "parent-exits-first.py"
            orphan = textwrap.dedent(
                f"""
                import json, os, signal, sys, time
                signal.signal(signal.SIGTERM, signal.SIG_IGN)
                parent = int(sys.argv[1])
                while os.getppid() == parent:
                    time.sleep(0.005)
                print(json.dumps({{
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {{"code": -32000, "message": "parent exited"}},
                }}), flush=True)
                time.sleep({HOLDER_LIFETIME_SECONDS!r})
                """
            )
            server.write_text(
                textwrap.dedent(
                    f"""
                    import os
                    import subprocess
                    import sys
                    from pathlib import Path

                    child = subprocess.Popen(
                        [sys.executable, "-c", {orphan!r}, str(os.getpid())],
                        process_group=0,
                    )
                    Path({str(pid_path)!r}).write_text(
                        f"{{os.getpid()}} {{child.pid}}", encoding="utf-8"
                    )
                    """
                ),
                encoding="utf-8",
            )
            probe = subprocess.Popen(
                self.probe_command(server, root / "wire.json", PROBE_FUSE_SECONDS),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            bounded = True
            try:
                try:
                    stdout, stderr = probe.communicate(timeout=HOLDER_BOUND_SECONDS)
                except subprocess.TimeoutExpired:
                    bounded = False
                    probe.kill()
                    stdout, stderr = probe.communicate(timeout=PROBE_FUSE_SECONDS)
                pids = self.read_pids(pid_path)
                module._wait_for_process_pids(set(pids), HOLDER_BOUND_SECONDS)
                running = sorted(module._running_pids(set(pids)))
            finally:
                if probe.poll() is None:
                    probe.kill()
                    probe.wait(timeout=PROBE_FUSE_SECONDS)
                self.kill_leftovers(module, pid_path)

        self.assertTrue(bounded, "cleanup blocked on pipes inherited by an orphan")
        self.assertNotEqual(probe.returncode, 0, stdout)
        self.assertIn("initialize failed", stderr)
        self.assertEqual(running, [], f"orphaned probe processes survived: {running}")

    @unittest.skipUnless(os.name == "posix", "POSIX process table observation")
    def test_unreadable_process_table_is_never_read_as_absence(self) -> None:
        # Сбой `ps` (здесь — истёкший предохранитель) — неизвестность:
        # снимок не пуст, а ошибочен; живой процесс не объявляется мёртвым.
        module = load_module()
        hung_ps = subprocess.TimeoutExpired(["ps"], module._PROCESS_OBSERVATION_TIMEOUT_SECONDS)
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            with patch.object(module.subprocess, "run", side_effect=hung_ps):
                with self.assertRaises(module.ProcessObservationError):
                    module._posix_process_snapshot()
                self.assertTrue(module._process_is_running(process.pid))
                self.assertEqual(module._running_pids({process.pid}), {process.pid})
        finally:
            process.kill()
            process.wait(timeout=PROBE_FUSE_SECONDS)
        # Отсутствие ядро сообщает точно и без `ps`.
        with patch.object(module.subprocess, "run", side_effect=hung_ps):
            self.assertFalse(module._process_is_running(process.pid))

    @unittest.skipUnless(os.name == "posix", "POSIX process table observation")
    def test_cleanup_without_a_process_table_is_incomplete_and_stops_the_leader(self) -> None:
        module = load_module()
        leader = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            start_new_session=True,
        )
        signalled: list[int] = []
        try:
            ownership = module.ProcessOwnership(leader, None, leader.pid)
            identity = module.ProcessIdentity(leader.pid, os.getpid(), leader.pid, "start")
            failure = module.ProcessObservationError("process table is unreadable: hung")
            with patch.object(module, "_posix_process_snapshot", side_effect=failure):
                terminated = ownership.terminate()
                with patch.object(
                    module.os,
                    "kill",
                    side_effect=lambda pid, number: signalled.append(pid),
                ):
                    quiesced = ownership.quiesce({identity}, 0.0)
        finally:
            if leader.poll() is None:
                leader.kill()
            leader.wait(timeout=PROBE_FUSE_SECONDS)

        self.assertFalse(terminated.complete)
        self.assertTrue(
            any("snapshot failed" in detail for detail in terminated.incomplete),
            terminated.incomplete,
        )
        self.assertEqual(leader.returncode, -signal.SIGKILL)
        self.assertFalse(quiesced.complete)
        self.assertEqual(quiesced.survivors, {identity})
        self.assertEqual(signalled, [], "a PID was signalled without a readable table")

    @unittest.skipUnless(os.name == "posix", "POSIX process identity regression")
    def test_identity_mismatch_is_not_signalled_during_escalation(self) -> None:
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            module = load_module()
            self.assertTrue(
                hasattr(module, "ProcessOwnership"),
                "shared process ownership abstraction is missing",
            )
            ownership = module.ProcessOwnership.capture(process)
            identity = ownership.public_identity
            self.assertIsNotNone(identity)
            mismatched = module.ProcessIdentity(
                pid=identity.pid,
                parent_pid=identity.parent_pid,
                session_id=identity.session_id,
                start_identity=identity.start_identity + "-reused",
            )

            ownership.signal({mismatched}, signal.SIGKILL)

            self.assertIsNone(
                process.poll(),
                "a reused PID identity was signalled during escalation",
            )
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=1.0)

    @unittest.skipUnless(os.name == "posix", "POSIX session escape boundary")
    def test_unregistered_setsid_child_does_not_block_or_get_claimed(self) -> None:
        # Сбежавший через setsid потомок держит stdout пробы и сам запускает
        # её очистку ошибкой на initialize — уже после выхода родителя, так
        # что в снимке он не потомок владельца. Свойство — событие: проба
        # завершилась, пока держатель ещё жив, и он не получил SIGTERM.
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "pids.txt"
            ready_path = root / "escaped-ready"
            signal_path = root / "escaped-signalled"
            server = root / "unregistered-session-escape.py"
            escaped = textwrap.dedent(
                f"""
                import json, os, signal, sys, time
                from pathlib import Path
                parent = int(sys.argv[1])
                os.setsid()
                signal.signal(
                    signal.SIGTERM,
                    lambda *_: Path({str(signal_path)!r}).write_text("term", encoding="utf-8"),
                )
                Path({str(ready_path)!r}).write_text("ready", encoding="utf-8")
                while os.getppid() == parent:
                    time.sleep(0.005)
                print(json.dumps({{
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {{"code": -32000, "message": "parent exited"}},
                }}), flush=True)
                time.sleep({HOLDER_LIFETIME_SECONDS!r})
                """
            )
            server.write_text(
                textwrap.dedent(
                    f"""
                    import os
                    import subprocess
                    import sys
                    import time
                    from pathlib import Path

                    child = subprocess.Popen(
                        [sys.executable, "-c", {escaped!r}, str(os.getpid())]
                    )
                    while not Path({str(ready_path)!r}).exists():
                        time.sleep(0.005)
                    Path({str(pid_path)!r}).write_text(
                        f"{{os.getpid()}} {{child.pid}}", encoding="utf-8"
                    )
                    """
                ),
                encoding="utf-8",
            )
            probe = subprocess.Popen(
                self.probe_command(server, root / "wire.json", PROBE_FUSE_SECONDS),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            bounded = True
            try:
                try:
                    stdout, stderr = probe.communicate(timeout=HOLDER_BOUND_SECONDS)
                except subprocess.TimeoutExpired:
                    bounded = False
                    probe.kill()
                    stdout, stderr = probe.communicate(timeout=PROBE_FUSE_SECONDS)
                _, child_pid = self.read_pids(pid_path)
                child_alive_after_probe = module._process_is_running(child_pid)
                signalled = signal_path.exists()
            finally:
                if probe.poll() is None:
                    probe.kill()
                    probe.wait(timeout=PROBE_FUSE_SECONDS)
                self.kill_leftovers(module, pid_path)

        self.assertTrue(bounded, "escaped child kept probe cleanup blocked")
        self.assertNotEqual(probe.returncode, 0, stdout)
        self.assertIn("initialize failed", stderr)
        self.assertTrue(
            child_alive_after_probe,
            "the probe finished only after the escaped pipe holder was gone",
        )
        self.assertFalse(
            signalled,
            "cleanup sent SIGTERM to an unregistered escaped process",
        )

    def test_release_baseline_fixture_is_pinned_to_observed_v0123_evidence(self) -> None:
        self.assertTrue(BASELINE_FIXTURE.is_file(), "release baseline fixture is missing")
        self.assertEqual(
            hashlib.sha256(BASELINE_FIXTURE.read_bytes()).hexdigest(),
            "8e330877888f7760ee6b44eec001b86c2b0c81b246b34db28104082bb1d39fab",
            "the published baseline is immutable; capture a new versioned fixture instead",
        )
        fixture = json.loads(BASELINE_FIXTURE.read_text(encoding="utf-8"))

        self.assertEqual(fixture["schemaVersion"], 1)
        self.assertEqual(
            fixture["source"],
            {
                "assetName": "unica-runtime-darwin-arm64.tar.gz",
                "assetSize": 103410270,
                "assetSha256": "f257a154fe45fa9fe76a4f8ae456e3ddbfb8b0567fd88b5e67e55b1838171c9b",
                "descriptorEntrypoint": "bin/darwin-arm64/unica",
                "descriptorFileCount": 83,
                "descriptorName": "unica-runtime-darwin-arm64.json",
                "descriptorSha256": "5c29681a784f44175e45605560f2debea2d7e60643eb8fd0ae0972224b936b45",
                "descriptorSize": 15489,
                "releasePublishedAt": "2026-08-19T14:54:10Z",
                "releaseUrl": "https://github.com/IngvarConsulting/unica/releases/tag/v0.12.3",
                "tag": "v0.12.3",
                "tagCommit": "f6d23068c397cd85c540812de7627b2c3f434d68",
            },
        )
        self.assertEqual(fixture["target"], "darwin-arm64")
        self.assertEqual(fixture["packageForm"], "full-runtime")
        self.assertEqual(
            fixture["coldInstall"],
            {
                "mcpReachableBeforeFullArchiveInstall": False,
                "mcpReachableOnlyAfterFullArchiveInstall": True,
            },
        )
        wire = fixture["wire"]
        self.assertEqual(wire["toolCount"], 74)
        self.assertEqual(wire["toolNames"], sorted(set(wire["toolNames"])))
        self.assertEqual(
            [name for name in wire["toolNames"] if name.startswith("unica.runtime.job.")],
            RUNTIME_JOB_NAMES,
        )


if __name__ == "__main__":
    unittest.main()
