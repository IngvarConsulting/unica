from __future__ import annotations

import importlib.util
import io
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SMOKE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-mcp.py"
# Предохранитель от зависания процесса теста или ожидания события, а не
# проверяемый срок: зелёный прогон кончается событием задолго до него.
FUSE_SECONDS = 30.0
# Срок жизни фейковых держателей, если их никто не убил. Предохранители
# короче него, поэтому «дождались события» не путается с «держатель сам
# умер».
HOLDER_LIFETIME_SECONDS = 60.0
# Бюджет старта фейка для проверки жёсткого общего срока смоука: срок
# должен истечь уже после того, как фейк породил потомка. Его величина —
# не контракт; контракт в том, что общий срок обрывает смоук раньше
# заведомо большего срока запроса.
SMOKE_TOTAL_START_BUDGET_SECONDS = 3.0
SMOKE_REQUEST_TIMEOUT_SECONDS = 10 * SMOKE_TOTAL_START_BUDGET_SECONDS
# Колпак запроса в проверке трубы, удержанной потомком: он покрывает старт
# фейка и initialize, а держатель живёт много дольше колпака.
PIPE_HOLDER_REQUEST_CAP_SECONDS = 5.0


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp", SMOKE_SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeWindowsProcess:
    pid = 99

    def __init__(self) -> None:
        self._handle = "process"
        self.returncode = None
        self.stdin = io.StringIO()
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()


class FakeWindowsApi:
    """Fault-injectable handle ledger for Windows ownership tests."""

    def __init__(
        self,
        *failures: str,
        active_counts: tuple[int, ...] = (0,),
        wait_signaled: bool = True,
        member_pids: tuple[int, ...] = (),
        foreign_pids: tuple[int, ...] = (),
        unsignaled: tuple[str, ...] = (),
    ) -> None:
        self.failures = list(failures)
        self.active_counts = list(active_counts)
        self.wait_signaled = wait_signaled
        self.member_pids = member_pids
        self.foreign_pids = set(foreign_pids)
        self.unsignaled = set(unsignaled)
        self.waited: list[object] = []
        self.events: list[str] = []
        self.open_handles = {"process"}

    def _record(self, operation: str) -> None:
        self.events.append(operation)
        if operation in self.failures:
            self.failures.remove(operation)
            raise RuntimeError(f"{operation} injected failure")

    def create_job(self):
        self._record("CreateJobObjectW")
        self.open_handles.add("job")
        return "job"

    def set_kill_on_job_close(self, job) -> None:
        self._record("SetInformationJobObject")

    def assign_process(self, job, process_handle) -> None:
        self._record("AssignProcessToJobObject")

    def create_thread_snapshot(self):
        self._record("CreateToolhelp32Snapshot")
        self.open_handles.add("snapshot")
        return "snapshot"

    def open_primary_thread(self, snapshot, pid):
        self._record("OpenThread")
        self.open_handles.add("thread")
        return "thread"

    def resume_thread(self, thread) -> int:
        self._record("ResumeThread")
        return 1

    def terminate_process(self, process_handle) -> None:
        self._record("TerminateProcess")

    def terminate_job(self, job) -> None:
        self._record("TerminateJobObject")

    def wait_for_single_object(self, handle, timeout_seconds: float) -> bool:
        self._record("WaitForSingleObject")
        self.waited.append(handle)
        return self.wait_signaled and handle not in self.unsignaled

    def close_popen_handle(self, process) -> None:
        self._record("CloseHandle(process)")
        self.open_handles.remove("process")
        process._handle = None
        if process.returncode is None:
            process.returncode = 1

    def close_handle(self, handle) -> None:
        operation = f"CloseHandle({handle})"
        self._record(operation)
        self.open_handles.remove(handle)

    def active_process_count(self, job) -> int:
        self._record("QueryInformationJobObject")
        if len(self.active_counts) > 1:
            return self.active_counts.pop(0)
        return self.active_counts[0]

    def job_process_ids(self, job) -> tuple[int, ...]:
        self._record("QueryInformationJobObject(ProcessIdList)")
        return self.member_pids

    def open_job_member(self, job, pid: int):
        self._record(f"OpenProcess({pid})")
        if pid in self.foreign_pids:
            return None
        handle = f"member-{pid}"
        self.open_handles.add(handle)
        return handle


class FakeKernel32:
    """The kernel32 calls behind job membership, driven through real ctypes."""

    def __init__(
        self,
        *,
        member_pids: tuple[int, ...] = (),
        foreign_pids: tuple[int, ...] = (),
        open_errors: dict[int, int] | None = None,
        query_error: int | None = None,
        in_job_error: int | None = None,
        outgrow: bool = False,
    ) -> None:
        self.member_pids = member_pids
        self.outgrow = outgrow
        self.foreign_pids = set(foreign_pids)
        self.open_errors = dict(open_errors or {})
        self.query_error = query_error
        self.in_job_error = in_job_error
        self.last_error = 0
        self.list_capacities: list[int] = []
        self.open_handles: set[str] = set()
        self.closed: list[str] = []

    def QueryInformationJobObject(self, job, information_class, info, size, returned):
        assert information_class == 3, information_class
        structure = info._obj
        capacity = len(structure.ProcessIdList)
        self.list_capacities.append(capacity)
        if self.query_error is not None:
            self.last_error = self.query_error
            return False
        if self.outgrow:
            self.member_pids = tuple(range(1, capacity + 2))
        structure.NumberOfAssignedProcesses = len(self.member_pids)
        if len(self.member_pids) > capacity:
            self.last_error = 234
            return False
        structure.NumberOfProcessIdsInList = len(self.member_pids)
        for index, pid in enumerate(self.member_pids):
            structure.ProcessIdList[index] = pid
        return True

    def OpenProcess(self, access, inherit, pid):
        assert access == 0x00100000 | 0x1000, hex(access)
        if pid in self.open_errors:
            self.last_error = self.open_errors[pid]
            return None
        handle = f"process-{pid}"
        self.open_handles.add(handle)
        return handle

    def IsProcessInJob(self, handle, job, result):
        if self.in_job_error is not None:
            self.last_error = self.in_job_error
            return False
        pid = int(handle.removeprefix("process-"))
        result._obj.value = pid not in self.foreign_pids
        return True

    def CloseHandle(self, handle):
        self.open_handles.remove(handle)
        self.closed.append(handle)
        return True


def fake_kernel32_windows_api(module, kernel32: FakeKernel32):
    """A real ``_WindowsApi`` whose kernel32 and last-error source are fakes."""

    import ctypes
    import types
    from ctypes import wintypes

    api = object.__new__(module._WIRE_PROBE._WindowsApi)
    api.ctypes = types.SimpleNamespace(
        Structure=ctypes.Structure,
        c_size_t=ctypes.c_size_t,
        byref=ctypes.byref,
        sizeof=ctypes.sizeof,
        get_last_error=lambda: kernel32.last_error,
    )
    api.wintypes = wintypes
    api.kernel32 = kernel32
    return api


def assert_processes_exit(test: unittest.TestCase, module, pids, message: str) -> None:
    """Wait for the exit event, then check it; the fuse is not the property.

    A process killed by cleanup leaves the table asynchronously, and a
    reparented zombie lingers until its new parent reaps it. A process that
    cleanup never signalled outlives the fuse, so the check still fails.
    """
    pids = set(pids)
    module._wait_for_process_pids(pids, FUSE_SECONDS)
    for pid in sorted(pids):
        with test.subTest(pid=pid):
            test.assertFalse(module._process_is_running(pid), message)


class SmokeUnicaMcpTests(unittest.TestCase):
    def test_v13_contract_allows_path_only_as_the_resolve_bridge_input(self) -> None:
        # `unica.resolve` — аварийный мост между адресом и раскладкой, и путь на
        # входе разрешён только ему (#801, решение зонтика #871). Остальные схемы
        # ключ `path` по-прежнему не несут; дым обязан различать эти два случая.
        module = load_module()

        def schema(properties: dict) -> dict:
            return {
                "type": "object",
                "properties": properties,
                "required": [],
                "additionalProperties": False,
            }

        def tools(extra: dict[str, dict]) -> list[dict]:
            listed = []
            for name in sorted(module.V13_COMPATIBILITY_TOOL_NAMES):
                properties = {"at": {"type": "string"}}
                properties.update(extra.get(name, {}))
                listed.append({"name": name, "inputSchema": schema(properties)})
            return listed

        module._stable_v13_tool_contract(
            tools({"unica.resolve": {"path": {"type": "string"}}}),
            module.V13_COMPATIBILITY_TOOL_NAMES,
        )
        with self.assertRaisesRegex(SystemExit, "leaks path"):
            module._stable_v13_tool_contract(
                tools({"unica.view": {"path": {"type": "string"}}}),
                module.V13_COMPATIBILITY_TOOL_NAMES,
            )

    def expected_tools(self) -> set[str]:
        module = load_module()
        return module.expected_tool_names(REPO_ROOT / "plugins" / "unica")

    def test_request_notifications_cannot_restart_the_aggregate_deadline(self) -> None:
        # Часы управляемые: уведомления приходят каждые 0.02 с бесконечно,
        # срок запроса 0.05 с. Перезапуск срока на уведомлении не дал бы
        # запросу кончиться вовсе, а не «кончиться медленнее».
        module = load_module()
        wire = module._WIRE_PROBE

        class Notifications:
            def __init__(self) -> None:
                self.now = 0.0
                self.delivered = 0

            def monotonic(self) -> float:
                return self.now

            def get(self, *, timeout: float) -> str:
                next_arrival = (self.delivered + 1) * 0.02
                if next_arrival > self.now + timeout:
                    self.now += timeout
                    raise wire.queue.Empty
                if next_arrival > 1.0:
                    raise AssertionError("notifications restarted the request deadline")
                self.now = next_arrival
                self.delivered += 1
                return '{"jsonrpc":"2.0","method":"notifications/progress"}\n'

        class Process:
            stdin = io.StringIO()

        lines = Notifications()
        session = module.McpSession.__new__(module.McpSession)
        session.process = Process()
        session.timeout_seconds = 0.05
        session.lines = lines
        session.diagnostics = []

        with mock.patch.object(wire, "time", mock.Mock(monotonic=lines.monotonic)):
            with self.assertRaisesRegex(SystemExit, "timed out after 0.05s"):
                session.request({"jsonrpc": "2.0", "id": 41, "method": "tools/list"})

        self.assertEqual(lines.delivered, 2)
        self.assertAlmostEqual(lines.now, 0.05)

    def test_request_records_the_response_kind_for_shared_wire_diagnostics(self) -> None:
        module = load_module()

        class Process:
            stdin = io.StringIO()

        session = module.McpSession.__new__(module.McpSession)
        session.process = Process()
        session.timeout_seconds = 0.05
        session.deadline = time.monotonic() + 0.05
        session.lines = module.queue.Queue()
        session.diagnostics = []
        session.response_kinds = []
        session.lines.put('{"jsonrpc":"2.0","id":41,"result":{"tools":[]}}\n')

        response = session.request(
            {"jsonrpc": "2.0", "id": 41, "method": "tools/list", "params": {}}
        )

        self.assertEqual(response["result"], {"tools": []})
        self.assertEqual(
            session.response_kinds,
            [{"kind": "result", "method": "tools/list"}],
        )

    def test_whole_smoke_has_a_hard_aggregate_deadline(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            child_pid_path = Path(directory) / "child.pid"
            server = Path(directory) / "never-answers.py"
            server.write_text(
                textwrap.dedent(
                    """
                    import os
                    import sys
                    import subprocess
                    import time
                    from pathlib import Path

                    options = {}
                    if os.name == "posix":
                        options["process_group"] = 0
                    elif hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
                        options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
                    child = subprocess.Popen(
                        [
                            sys.executable,
                            "-c",
                            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)",
                        ],
                        **options,
                    )
                    # Атомарно: тест читает файл, не дожидаясь писателя.
                    staged = Path(sys.argv[1] + ".tmp")
                    staged.write_text(f"{os.getpid()} {child.pid}", encoding="utf-8")
                    os.replace(staged, sys.argv[1])
                    time.sleep(60)
                    """
                ),
                encoding="utf-8",
            )
            # Общий срок покрывает старт фейка; срок запроса заведомо больше,
            # так что оборвать смоук раньше него может только общий срок.
            started = time.monotonic()
            result = subprocess.run(
                [
                    sys.executable,
                    str(SMOKE_SCRIPT),
                    "--binary",
                    sys.executable,
                    "--binary-arg",
                    str(server),
                    "--binary-arg",
                    str(child_pid_path),
                    "--plugin-root",
                    str(REPO_ROOT / "plugins" / "unica"),
                    "--timeout-seconds",
                    str(SMOKE_REQUEST_TIMEOUT_SECONDS),
                    "--total-timeout-seconds",
                    str(SMOKE_TOTAL_START_BUDGET_SECONDS),
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=SMOKE_REQUEST_TIMEOUT_SECONDS + FUSE_SECONDS,
            )
            elapsed = time.monotonic() - started
            self.assertTrue(
                child_pid_path.is_file(),
                "the fake server did not start within the smoke start budget",
            )
            child_pids = [
                int(value)
                for value in child_pid_path.read_text(encoding="utf-8").split()
            ]

        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertIn("aggregate deadline", result.stderr)
        # A silent 124 tells nothing; the watchdog names the phase it
        # interrupted and copies out what the child had said so far.
        self.assertIn("last phase:", result.stderr)
        self.assertIn("initialize", result.stderr)
        self.assertEqual(result.stderr.count("---- smoke diagnostics"), 1, result.stderr)
        self.assertLess(elapsed, SMOKE_REQUEST_TIMEOUT_SECONDS)
        assert_processes_exit(
            self, load_module(), child_pids, "the deadline left the fake tree alive"
        )

    def test_close_reports_pipes_held_by_a_detached_descendant_within_the_request_cap(
        self,
    ) -> None:
        """A descendant that inherited the pipes must not eat the aggregate deadline.

        This is the Windows shape of the packaged smoke hang: the MCP process
        exits, a detached daemon keeps copies of its stdout/stderr handles, and
        EOF never arrives. `close()` reports that within the request cap.
        """
        module = load_module()
        grandchild_pid: int | None = None
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                pid_path = root / "pipe-holder.pid"
                server = root / "leave-pipe-holder.py"
                server.write_text(
                    textwrap.dedent(
                        """
                        import os
                        import subprocess
                        import sys
                        from pathlib import Path

                        # stdout is inherited on purpose: the descendant holds
                        # the smoke's pipe the way a detached daemon does.
                        child = subprocess.Popen(
                            [sys.executable, "-c", "import time; time.sleep(60)"]
                        )
                        # Atomic: the test polls for the file, not the writer.
                        staged = Path(sys.argv[1] + ".tmp")
                        staged.write_text(str(child.pid), encoding="utf-8")
                        os.replace(staged, sys.argv[1])
                        """
                    ),
                    encoding="utf-8",
                )
                session = module.McpSession(
                    [sys.executable, str(server), str(pid_path)],
                    os.environ.copy(),
                    1.0,
                    cwd=root,
                    deadline=time.monotonic() + 30.0,
                )
                deadline = time.monotonic() + FUSE_SECONDS
                while not pid_path.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                grandchild_pid = int(pid_path.read_text(encoding="utf-8"))
                started = time.monotonic()
                with self.assertRaisesRegex(
                    SystemExit, "reader threads did not stop within 1s"
                ):
                    session.close()
                elapsed = time.monotonic() - started
            # Ожидание EOF длилось бы весь срок жизни держателя (60 с).
            self.assertLess(elapsed, HOLDER_LIFETIME_SECONDS / 2)
            assert_processes_exit(
                self, module, {grandchild_pid}, "cleanup left the pipe holder alive"
            )
        finally:
            if grandchild_pid is not None and module._process_is_running(grandchild_pid):
                if os.name == "posix":
                    os.kill(grandchild_pid, signal.SIGKILL)

    def test_diagnostics_dump_copies_child_stderr_and_workspace_service_logs(
        self,
    ) -> None:
        module = load_module()

        class Session:
            diagnostics = ["first line\n", "второй: кириллица\n"]

        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            service_dir = cache_root / "services" / "svc-1"
            service_dir.mkdir(parents=True)
            (service_dir / "service.json").write_text(
                '{"pid": 4242, "port": 1}', encoding="utf-8"
            )
            (service_dir / "service.stderr.log").write_text(
                "\n".join(f"stderr {index}" for index in range(50)),
                encoding="utf-8",
            )
            module._trace("phase under test")
            rendered = module._dump_session_diagnostics(Session(), cache_root)

        self.assertIn("last phase: phase under test", rendered)
        self.assertIn("второй: кириллица", rendered)
        self.assertIn('{"pid": 4242, "port": 1}', rendered)
        self.assertNotIn("stderr 9\n", rendered)
        self.assertIn("stderr 49", rendered)

    def test_expired_watchdog_cannot_race_a_success_exit(self) -> None:
        runner = textwrap.dedent(
            f"""
            import importlib.util
            import sys
            import time
            from pathlib import Path

            spec = importlib.util.spec_from_file_location("smoke_under_test", {str(SMOKE_SCRIPT)!r})
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)

            import threading

            # Порядок задают события: срок истёк и очистка началась до того,
            # как передний план вернул успех. Сон в очистке лишь даёт
            # сломанному коду время выйти с 0; верный код ждёт её при любой
            # длительности.
            release_watchdog = threading.Event()
            cleanup_entered = threading.Event()
            foreground_returned = threading.Event()

            class PausedTimer:
                daemon = True

                def __init__(self, interval, callback):
                    self.callback = callback

                def start(self):
                    def run():
                        release_watchdog.wait()
                        self.callback()
                    threading.Thread(target=run, daemon=True).start()

                def cancel(self):
                    pass

            class SlowCleanupSession:
                def terminate_tree(self, cache_root):
                    cleanup_entered.set()
                    foreground_returned.wait({FUSE_SECONDS!r})
                    time.sleep(0.3)

            def fake_smoke(command, plugin_root, timeout_seconds, deadline, session_started, admission_lock):
                session_started(SlowCleanupSession(), Path("cache"))
                release_watchdog.set()
                cleanup_entered.wait({FUSE_SECONDS!r})
                foreground_returned.set()

            module.threading.Timer = PausedTimer
            module.smoke = fake_smoke
            sys.argv = [
                "smoke-unica-mcp.py",
                "--binary", "unused",
                "--plugin-root", {str(REPO_ROOT / "plugins" / "unica")!r},
                "--total-timeout-seconds", "0.05",
            ]
            module.main()
            """
        )

        result = subprocess.run(
            [sys.executable, "-c", runner],
            capture_output=True,
            text=True,
            check=False,
            timeout=FUSE_SECONDS,
        )

        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertIn("aggregate deadline", result.stderr)
        self.assertNotIn("verified packaged", result.stdout)

    def test_deadline_during_session_admission_kills_spawned_process(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            child_pid_path = Path(directory) / "admission-child.pid"
            runner = textwrap.dedent(
                f"""
                import contextlib
                import importlib.util
                import os
                import subprocess
                import sys
                import time
                from pathlib import Path

                import threading

                spec = importlib.util.spec_from_file_location("smoke_under_test", {str(SMOKE_SCRIPT)!r})
                module = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(module)

                # Сторож отпускается, только когда допуск уже взят и потомок
                # порождён: срок истекает строго внутри допуска. Сон после
                # входа сторожа лишь даёт сломанному коду прочесть пустой
                # список сессий; верный код ждёт замок при любой длительности.
                release_watchdog = threading.Event()
                watchdog_fired = threading.Event()
                real_last_phase = module._last_phase

                def observed_last_phase():
                    watchdog_fired.set()
                    return real_last_phase()

                class PausedTimer:
                    daemon = True

                    def __init__(self, interval, callback):
                        self.callback = callback

                    def start(self):
                        def run():
                            release_watchdog.wait()
                            self.callback()
                        threading.Thread(target=run, daemon=True).start()

                    def cancel(self):
                        pass

                class Session:
                    def __init__(self, process):
                        self.process = process

                    def terminate_tree(self, cache_root):
                        if self.process.poll() is None:
                            self.process.kill()
                        self.process.wait(timeout={FUSE_SECONDS!r})

                def fake_smoke(
                    command,
                    plugin_root,
                    timeout_seconds,
                    deadline,
                    session_started,
                    admission_lock=None,
                ):
                    guard = admission_lock or contextlib.nullcontext()
                    with guard:
                        options = {{"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}}
                        if os.name == "posix":
                            options["start_new_session"] = True
                        child = subprocess.Popen(
                            [sys.executable, "-c", "import time; time.sleep(60)"],
                            **options,
                        )
                        Path({str(child_pid_path)!r}).write_text(str(child.pid), encoding="utf-8")
                        release_watchdog.set()
                        watchdog_fired.wait({FUSE_SECONDS!r})
                        time.sleep(0.2)
                        session_started(Session(child), Path("cache"))
                    time.sleep(60)

                module._last_phase = observed_last_phase
                module.threading.Timer = PausedTimer
                module.smoke = fake_smoke
                sys.argv = [
                    "smoke-unica-mcp.py",
                    "--binary", "unused",
                    "--plugin-root", {str(REPO_ROOT / "plugins" / "unica")!r},
                    "--total-timeout-seconds", "0.05",
                ]
                module.main()
                """
            )
            result = subprocess.run(
                [sys.executable, "-c", runner],
                capture_output=True,
                text=True,
                check=False,
                timeout=FUSE_SECONDS,
            )
            child_pid = int(child_pid_path.read_text(encoding="utf-8"))
            try:
                self.assertEqual(result.returncode, 124, result.stderr)
                assert_processes_exit(
                    self,
                    load_module(),
                    {child_pid},
                    "the watchdog missed the child spawned during admission",
                )
            finally:
                if load_module()._process_is_running(child_pid):
                    os.kill(child_pid, signal.SIGKILL)

    def test_committed_success_cancels_a_late_watchdog_callback(self) -> None:
        runner = textwrap.dedent(
            f"""
            import importlib.util
            import sys
            import threading
            import time

            spec = importlib.util.spec_from_file_location("smoke_under_test", {str(SMOKE_SCRIPT)!r})
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)

            callback_entered = threading.Event()
            release_callback = threading.Event()
            callback_finished = threading.Event()

            class PausedTimer:
                daemon = True

                def __init__(self, interval, callback):
                    self.callback = callback

                def start(self):
                    def run():
                        callback_entered.set()
                        release_callback.wait()
                        self.callback()
                        callback_finished.set()
                    threading.Thread(target=run, daemon=True).start()

                def cancel(self):
                    # Поздний обратный вызов обязан отработать до выхода:
                    # иначе проверка была бы пустой.
                    release_callback.set()
                    if not callback_finished.wait({FUSE_SECONDS!r}):
                        raise SystemExit("late watchdog callback never finished")

            def fake_smoke(command, plugin_root, timeout_seconds, deadline, session_started, admission_lock):
                callback_entered.wait()

            module.threading.Timer = PausedTimer
            module.smoke = fake_smoke
            sys.argv = [
                "smoke-unica-mcp.py",
                "--binary", "unused",
                "--plugin-root", {str(REPO_ROOT / "plugins" / "unica")!r},
            ]
            module.main()
            """
        )

        result = subprocess.run(
            [sys.executable, "-c", runner],
            capture_output=True,
            text=True,
            check=False,
            timeout=FUSE_SECONDS,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified packaged", result.stdout)
        self.assertNotIn("aggregate deadline", result.stderr)

    def test_committed_failure_cancels_a_late_watchdog_callback(self) -> None:
        runner = textwrap.dedent(
            f"""
            import importlib.util
            import sys
            import threading

            spec = importlib.util.spec_from_file_location("smoke_under_test", {str(SMOKE_SCRIPT)!r})
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)

            callback_entered = threading.Event()
            release_callback = threading.Event()
            callback_finished = threading.Event()

            class PausedTimer:
                daemon = True

                def __init__(self, interval, callback):
                    self.callback = callback

                def start(self):
                    def run():
                        callback_entered.set()
                        release_callback.wait()
                        self.callback()
                        callback_finished.set()
                    threading.Thread(target=run, daemon=True).start()

                def cancel(self):
                    release_callback.set()
                    if not callback_finished.wait({FUSE_SECONDS!r}):
                        raise SystemExit("late watchdog callback never finished")

            def fake_smoke(command, plugin_root, timeout_seconds, deadline, session_started, admission_lock):
                callback_entered.wait()
                raise SystemExit("early real failure")

            module.threading.Timer = PausedTimer
            module.smoke = fake_smoke
            sys.argv = [
                "smoke-unica-mcp.py",
                "--binary", "unused",
                "--plugin-root", {str(REPO_ROOT / "plugins" / "unica")!r},
            ]
            module.main()
            """
        )

        result = subprocess.run(
            [sys.executable, "-c", runner],
            capture_output=True,
            text=True,
            check=False,
            timeout=FUSE_SECONDS,
        )

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("early real failure", result.stderr)
        self.assertNotIn("aggregate deadline", result.stderr)

    def test_close_never_blocks_on_stream_owned_by_a_live_reader(self) -> None:
        module = load_module()

        class Input:
            closed = False

            def close(self) -> None:
                self.closed = True

        class Output:
            closed = False

            def close(self) -> None:
                raise AssertionError("close waited on a reader-owned stream lock")

        class Process:
            stdin = Input()
            stdout = Output()
            stderr = Output()

            def wait(self, timeout: float) -> int:
                return 0

        class LiveReader:
            def join(self, timeout: float) -> None:
                pass

            def is_alive(self) -> bool:
                return True

        session = module.McpSession.__new__(module.McpSession)
        session.process = Process()
        session.timeout_seconds = 0.05
        session.deadline = time.monotonic() + 1.0
        session.reader = LiveReader()
        session.error_reader = LiveReader()
        session.diagnostics = []

        with self.assertRaisesRegex(SystemExit, "reader threads did not stop"):
            session.close()

    def test_constructor_failure_after_popen_reaps_the_child(self) -> None:
        module = load_module()
        spawned: list[subprocess.Popen[str]] = []
        real_popen = module.subprocess.Popen
        real_start = module.threading.Thread.start
        starts = 0

        def recording_popen(*args, **kwargs):
            process = real_popen(*args, **kwargs)
            spawned.append(process)
            return process

        child_pid_path: Path

        def fail_second_start(thread):
            nonlocal starts
            starts += 1
            if starts == 2:
                deadline = time.monotonic() + FUSE_SECONDS
                while not child_pid_path.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                raise RuntimeError("reader start failed")
            return real_start(thread)

        with tempfile.TemporaryDirectory() as directory:
            child_pid_path = Path(directory) / "constructor-child.pid"
            server = Path(directory) / "spawn-child.py"
            server.write_text(
                textwrap.dedent(
                    """
                    import os
                    import signal
                    import subprocess
                    import sys
                    import time
                    from pathlib import Path

                    options = {}
                    if os.name == "posix":
                        options["process_group"] = 0
                    child = subprocess.Popen(
                        [
                            sys.executable,
                            "-c",
                            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)",
                        ],
                        **options,
                    )
                    # Atomic: the test polls for the file, not the writer.
                    staged = Path(sys.argv[1] + ".tmp")
                    staged.write_text(str(child.pid), encoding="utf-8")
                    os.replace(staged, sys.argv[1])
                    time.sleep(60)
                    """
                ),
                encoding="utf-8",
            )
            with mock.patch.object(module.subprocess, "Popen", recording_popen), mock.patch.object(
                module.threading.Thread, "start", fail_second_start
            ):
                with self.assertRaisesRegex(RuntimeError, "reader start failed"):
                    module.McpSession(
                        [sys.executable, str(server), str(child_pid_path)],
                        os.environ.copy(),
                        1.0,
                        cwd=Path(directory),
                    )
            child_pid = int(child_pid_path.read_text(encoding="utf-8"))

        self.assertGreaterEqual(len(spawned), 1)
        public = spawned[0]
        # Без очистки корень спал бы 60 с: ожидание с предохранителем
        # отличает «добит» от «жив», не замеряя скорость машины.
        public.wait(timeout=FUSE_SECONDS)
        assert_processes_exit(
            self,
            module,
            {public.pid, child_pid},
            "constructor cleanup left the spawned tree alive",
        )

    def test_smoke_uses_the_shared_wire_process_ownership_boundary(self) -> None:
        module = load_module()

        self.assertIs(module.ProcessOwnership, module._WIRE_PROBE.ProcessOwnership)
        self.assertTrue(issubclass(module.McpSession, module._WIRE_PROBE.JsonRpcSession))

    def test_windows_liveness_probe_never_calls_os_kill(self) -> None:
        module = load_module()
        identity = module.ProcessIdentity(41, None, None, "start-41")

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_windows_process_state",
            return_value=(identity, True, None),
            create=True,
        ), mock.patch.object(
            module._WIRE_PROBE.os,
            "kill",
            side_effect=AssertionError("Windows liveness probe called os.kill"),
        ):
            running = module._WIRE_PROBE._process_is_running(identity.pid)

        self.assertTrue(running)

    def test_windows_session_starts_suspended_before_ownership_capture(self) -> None:
        module = load_module()
        creation_flags: list[int] = []

        class Process:
            pid = 99
            stdin = io.StringIO()
            stdout = io.StringIO()
            stderr = io.StringIO()

        class Reader:
            def __init__(self, *args, **kwargs):
                pass

            def start(self) -> None:
                pass

        process = Process()

        def popen(*args, **kwargs):
            creation_flags.append(kwargs["creationflags"])
            return process

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE.subprocess,
            "CREATE_NEW_PROCESS_GROUP",
            0x200,
            create=True,
        ), mock.patch.object(
            module._WIRE_PROBE.subprocess, "Popen", side_effect=popen
        ), mock.patch.object(
            module._WIRE_PROBE.ProcessOwnership,
            "capture",
            return_value=object(),
        ), mock.patch.object(
            module._WIRE_PROBE.threading, "Thread", Reader
        ):
            module._WIRE_PROBE.JsonRpcSession(
                ["unica"], {}, cwd=Path("."), deadline=1.0
            )

        self.assertEqual(creation_flags, [0x204])

    def test_windows_capture_attaches_the_suspended_process_to_a_job_handle(self) -> None:
        module = load_module()
        identity = module.ProcessIdentity(99, None, None, "start-99")
        job = object()

        class Process:
            pid = 99
            _handle = object()

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=identity,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_attach_windows_process_job",
            return_value=job,
            create=True,
        ) as attach:
            ownership = module.ProcessOwnership.capture(Process())

        attach.assert_called_once()
        self.assertIs(ownership.windows_job, job)

    def test_windows_job_attach_rolls_back_every_acquired_handle(self) -> None:
        module = load_module()
        failure_cases = (
            "CreateJobObjectW",
            "SetInformationJobObject",
            "AssignProcessToJobObject",
            "CreateToolhelp32Snapshot",
            "OpenThread",
            "ResumeThread",
        )

        for failed_operation in failure_cases:
            with self.subTest(failed_operation=failed_operation):
                process = FakeWindowsProcess()
                api = FakeWindowsApi(failed_operation)

                with self.assertRaisesRegex(RuntimeError, failed_operation):
                    module._WIRE_PROBE._attach_windows_process_job(
                        process,
                        deadline=time.monotonic() + 0.5,
                        api=api,
                    )

                self.assertEqual(api.open_handles, set(), api.events)
                self.assertLess(
                    api.events.index("TerminateProcess"),
                    api.events.index("WaitForSingleObject"),
                    api.events,
                )
                self.assertLess(
                    api.events.index("WaitForSingleObject"),
                    api.events.index("CloseHandle(process)"),
                    api.events,
                )
                for handle in ("thread", "snapshot", "job"):
                    if handle in {
                        event.removeprefix("CloseHandle(").removesuffix(")")
                        for event in api.events
                        if event.startswith("CloseHandle(")
                    }:
                        self.assertNotIn(handle, api.open_handles)

    def test_windows_job_attach_reports_cleanup_failures_without_skipping_later_cleanup(
        self,
    ) -> None:
        module = load_module()
        cleanup_failures = (
            "TerminateProcess",
            "TerminateJobObject",
            "WaitForSingleObject",
            "CloseHandle(process)",
            "CloseHandle(thread)",
            "CloseHandle(snapshot)",
            "CloseHandle(job)",
        )

        for cleanup_failure in cleanup_failures:
            with self.subTest(cleanup_failure=cleanup_failure):
                process = FakeWindowsProcess()
                primary_failure = (
                    "ResumeThread"
                    if cleanup_failure == "CloseHandle(thread)"
                    else "OpenThread"
                )
                api = FakeWindowsApi(primary_failure, cleanup_failure)

                with self.assertRaises(RuntimeError) as raised:
                    module._WIRE_PROBE._attach_windows_process_job(
                        process,
                        deadline=time.monotonic() + 0.5,
                        api=api,
                    )

                self.assertIn(cleanup_failure, str(raised.exception))

                self.assertIn("TerminateProcess", api.events)
                self.assertIn("WaitForSingleObject", api.events)
                self.assertIn("CloseHandle(process)", api.events)
                self.assertIn("CloseHandle(snapshot)", api.events)
                self.assertIn("CloseHandle(job)", api.events)
                expected_leaks = {
                    event.removeprefix("CloseHandle(").removesuffix(")")
                    for event in (cleanup_failure,)
                    if event.startswith("CloseHandle(")
                }
                self.assertEqual(api.open_handles, expected_leaks, api.events)

    def test_windows_job_attach_cleanup_uses_the_passed_deadline(self) -> None:
        module = load_module()

        class ClockedApi(FakeWindowsApi):
            def wait_for_single_object(self, handle, timeout_seconds: float) -> bool:
                self._record("WaitForSingleObject")
                Clock.now += timeout_seconds
                return False

        class Clock:
            now = 10.0

            @classmethod
            def monotonic(cls) -> float:
                return cls.now

        process = FakeWindowsProcess()
        api = ClockedApi("OpenThread")
        with mock.patch.object(
            module._WIRE_PROBE.time, "monotonic", side_effect=Clock.monotonic
        ):
            with self.assertRaisesRegex(RuntimeError, "WaitForSingleObject timed out"):
                module._WIRE_PROBE._attach_windows_process_job(
                    process,
                    deadline=10.125,
                    api=api,
                )

        self.assertEqual(Clock.now, 10.125)
        self.assertEqual(api.open_handles, set())

    def test_windows_job_attach_failure_closes_suspended_process_pipes(self) -> None:
        module = load_module()

        class Process:
            pid = 99
            _handle = object()

            def __init__(self) -> None:
                self.stdin = io.StringIO()
                self.stdout = io.StringIO()
                self.stderr = io.StringIO()
                self.terminated = False

            def terminate(self) -> None:
                self.terminated = True

            @staticmethod
            def wait(timeout):
                return 1

        process = Process()
        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE.subprocess, "Popen", return_value=process
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_attach_windows_process_job",
            side_effect=RuntimeError("job attach failed"),
        ):
            with self.assertRaisesRegex(RuntimeError, "job attach failed"):
                module._WIRE_PROBE.JsonRpcSession(
                    ["unica"], {}, cwd=Path("."), deadline=1.0
                )

        self.assertTrue(process.stdin.closed)
        self.assertTrue(process.stdout.closed)
        self.assertTrue(process.stderr.closed)
        self.assertTrue(process.terminated)

    def test_windows_job_cleanup_waits_and_closes_exact_root_before_job_zero(self) -> None:
        module = load_module()
        process = FakeWindowsProcess()
        api = FakeWindowsApi(active_counts=(0,))
        job = module._WIRE_PROBE._WindowsProcessJob("job", api, process)
        api.open_handles.add("job")
        identity = module.ProcessIdentity(99, None, None, "start-99")
        ownership = module.ProcessOwnership(process, identity, None, job)

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=None,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_process_is_running",
            return_value=False,
        ):
            result = ownership.quiesce({identity}, 0.5)

        self.assertTrue(result.complete, result.incomplete)
        self.assertEqual(result.active_processes, 0)
        self.assertEqual(api.open_handles, set())
        self.assertLess(
            api.events.index("TerminateJobObject"),
            api.events.index("WaitForSingleObject"),
            api.events,
        )
        self.assertLess(
            api.events.index("WaitForSingleObject"),
            api.events.index("CloseHandle(process)"),
            api.events,
        )
        self.assertLess(
            api.events.index("CloseHandle(process)"),
            api.events.index("QueryInformationJobObject"),
            api.events,
        )
        self.assertLess(
            api.events.index("QueryInformationJobObject"),
            api.events.index("CloseHandle(job)"),
            api.events,
        )

    def test_windows_job_zero_is_not_cleanup_until_held_members_are_signaled(
        self,
    ) -> None:
        # ActiveProcesses уже 0, а потомок ещё не signaled: TerminateJobObject
        # асинхронен, счётчик задания снимается раньше сигнала объекта процесса.
        module = load_module()
        process = FakeWindowsProcess()
        api = FakeWindowsApi(
            active_counts=(0,),
            member_pids=(99, 41, 77),
            foreign_pids=(77,),
            unsignaled=("member-41",),
        )
        api.open_handles.add("job")
        job = module._WIRE_PROBE._WindowsProcessJob("job", api, process)
        identity = module.ProcessIdentity(99, None, None, "start-99")
        ownership = module.ProcessOwnership(process, identity, None, job)

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=None,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_process_is_running",
            return_value=False,
        ):
            result = ownership.quiesce({identity}, 0.5)

        self.assertFalse(result.complete)
        self.assertEqual(result.active_processes, 0)
        self.assertEqual(
            result.incomplete,
            (
                "Windows Job Object member 41 was not signaled "
                "before the cleanup deadline",
            ),
        )
        self.assertLess(
            api.events.index("OpenProcess(41)"),
            api.events.index("TerminateJobObject"),
            api.events,
        )
        self.assertIn("member-41", api.waited)
        self.assertIn("member-99", api.waited)
        self.assertNotIn("member-77", api.waited)
        self.assertEqual(api.open_handles, set())

    def test_windows_job_member_hold_failure_keeps_cleanup_incomplete(self) -> None:
        module = load_module()
        for injected in ("QueryInformationJobObject(ProcessIdList)", "OpenProcess(41)"):
            with self.subTest(injected=injected):
                process = FakeWindowsProcess()
                api = FakeWindowsApi(injected, member_pids=(41,))
                api.open_handles.add("job")
                job = module._WIRE_PROBE._WindowsProcessJob("job", api, process)
                identity = module.ProcessIdentity(99, None, None, "start-99")
                ownership = module.ProcessOwnership(process, identity, None, job)

                with mock.patch.object(
                    module._WIRE_PROBE.os, "name", "nt"
                ), mock.patch.object(
                    module._WIRE_PROBE,
                    "_current_process_identity",
                    return_value=None,
                ), mock.patch.object(
                    module._WIRE_PROBE,
                    "_process_is_running",
                    return_value=False,
                ):
                    result = ownership.quiesce({identity}, 0.5)

                self.assertFalse(result.complete)
                self.assertTrue(
                    any(injected in detail for detail in result.incomplete),
                    result.incomplete,
                )
                self.assertEqual(api.open_handles, set())

    def test_windows_api_job_process_ids_grows_buffer_on_more_data(self) -> None:
        module = load_module()
        for count, capacities in ((100, [64, 128]), (1000, [64, 1016])):
            with self.subTest(count=count):
                pids = tuple(range(1000, 1000 + count))
                kernel32 = FakeKernel32(member_pids=pids)
                api = fake_kernel32_windows_api(module, kernel32)

                self.assertEqual(api.job_process_ids("job"), pids)
                self.assertEqual(kernel32.list_capacities, capacities)

    def test_windows_api_job_process_ids_stops_growing_at_the_limit(self) -> None:
        module = load_module()
        # Задание растёт быстрее буфера: перезапросы ограничены, а не вечны.
        kernel32 = FakeKernel32(outgrow=True)
        api = fake_kernel32_windows_api(module, kernel32)

        with self.assertRaisesRegex(
            RuntimeError, r"QueryInformationJobObject\(ProcessIdList\).*234"
        ):
            api.job_process_ids("job")
        self.assertEqual(
            kernel32.list_capacities, [64 * 2**step for step in range(11)]
        )

    def test_windows_api_job_process_ids_reports_other_query_errors(self) -> None:
        module = load_module()
        kernel32 = FakeKernel32(query_error=6)
        api = fake_kernel32_windows_api(module, kernel32)

        with self.assertRaisesRegex(
            RuntimeError, r"QueryInformationJobObject\(ProcessIdList\).*error 6"
        ):
            api.job_process_ids("job")
        self.assertEqual(kernel32.list_capacities, [64])

    def test_windows_api_open_job_member_rejects_processes_outside_the_job(
        self,
    ) -> None:
        module = load_module()
        kernel32 = FakeKernel32(member_pids=(41,), foreign_pids=(42,))
        api = fake_kernel32_windows_api(module, kernel32)

        self.assertEqual(api.open_job_member("job", 41), "process-41")
        self.assertIsNone(api.open_job_member("job", 42))
        self.assertEqual(kernel32.open_handles, {"process-41"})
        self.assertEqual(kernel32.closed, ["process-42"])

    def test_windows_api_open_job_member_treats_gone_pids_as_absent(self) -> None:
        module = load_module()
        for error in (0, 5, 87, 1168):
            with self.subTest(error=error):
                kernel32 = FakeKernel32(open_errors={41: error})
                api = fake_kernel32_windows_api(module, kernel32)

                self.assertIsNone(api.open_job_member("job", 41))
                self.assertEqual(kernel32.open_handles, set())

        kernel32 = FakeKernel32(open_errors={41: 6})
        api = fake_kernel32_windows_api(module, kernel32)
        with self.assertRaisesRegex(RuntimeError, r"OpenProcess\(41\).*error 6"):
            api.open_job_member("job", 41)

    def test_windows_api_open_job_member_closes_handle_when_job_check_fails(
        self,
    ) -> None:
        module = load_module()
        kernel32 = FakeKernel32(member_pids=(41,), in_job_error=6)
        api = fake_kernel32_windows_api(module, kernel32)

        with self.assertRaisesRegex(
            RuntimeError, r"IsProcessInJob\(41\).*error 6"
        ):
            api.open_job_member("job", 41)
        self.assertEqual(kernel32.open_handles, set())
        self.assertEqual(kernel32.closed, ["process-41"])

    def test_windows_job_cleanup_is_handle_bound_and_uses_one_deadline(self) -> None:
        module = load_module()

        class Clock:
            now = 0.0

            @classmethod
            def monotonic(cls) -> float:
                return cls.now

            @classmethod
            def sleep(cls, duration: float) -> None:
                cls.now += duration

        class Api(FakeWindowsApi):
            def terminate_job(self, job) -> None:
                super().terminate_job(job)
                Clock.sleep(0.3)

            def wait_for_single_object(
                self, handle, timeout_seconds: float
            ) -> bool:
                self._record("WaitForSingleObject")
                Clock.sleep(timeout_seconds)
                return False

            def active_process_count(self, job) -> int:
                self._record("QueryInformationJobObject")
                return 0 if Clock.now >= 0.58 else 1

        original = module.ProcessIdentity(99, None, None, "original-start")
        replacement = module.ProcessIdentity(99, None, None, "replacement-start")
        process = FakeWindowsProcess()
        api = Api()
        api.open_handles.add("job")
        job = module._WIRE_PROBE._WindowsProcessJob("job", api, process)
        ownership = module.ProcessOwnership(process, original, None, job)

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=replacement,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_process_is_running",
            return_value=True,
        ), mock.patch.object(
            module._WIRE_PROBE.time, "monotonic", side_effect=Clock.monotonic
        ), mock.patch.object(
            module._WIRE_PROBE.time, "sleep", side_effect=Clock.sleep
        ):
            result = ownership.quiesce({original}, 0.5)

        self.assertIn("TerminateJobObject", api.events)
        self.assertFalse(result.complete)
        self.assertEqual(result.survivors, set())
        self.assertEqual(result.active_processes, 1)
        self.assertTrue(
            any("1 active process" in detail for detail in result.incomplete),
            result.incomplete,
        )
        self.assertLessEqual(Clock.now, 0.5)

    def test_windows_job_timeout_reports_incomplete_descendant_evidence(self) -> None:
        module = load_module()

        class Clock:
            now = 0.0

            @classmethod
            def monotonic(cls) -> float:
                return cls.now

            @classmethod
            def sleep(cls, duration: float) -> None:
                cls.now += duration

        original = module.ProcessIdentity(99, None, None, "original-start")
        process = FakeWindowsProcess()
        api = FakeWindowsApi(active_counts=(1,))
        api.open_handles.add("job")
        job = module._WIRE_PROBE._WindowsProcessJob("job", api, process)
        ownership = module.ProcessOwnership(process, original, None, job)

        with mock.patch.object(module._WIRE_PROBE.os, "name", "nt"), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=None,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_process_is_running",
            return_value=False,
        ), mock.patch.object(
            module._WIRE_PROBE.time, "monotonic", side_effect=Clock.monotonic
        ), mock.patch.object(
            module._WIRE_PROBE.time, "sleep", side_effect=Clock.sleep
        ):
            result = ownership.quiesce({original}, 0.5)

        self.assertFalse(result.complete)
        self.assertEqual(result.survivors, set())
        self.assertEqual(result.active_processes, 1)
        self.assertTrue(
            any("1 active process" in detail for detail in result.incomplete),
            result.incomplete,
        )
        self.assertLessEqual(Clock.now, 0.5)

    def test_windows_job_api_errors_remain_incomplete_after_root_exit(self) -> None:
        module = load_module()

        original = module.ProcessIdentity(99, None, None, "original-start")
        for expected in ("TerminateJobObject", "QueryInformationJobObject"):
            with self.subTest(expected=expected):
                process = FakeWindowsProcess()
                api = FakeWindowsApi(expected)
                api.open_handles.add("job")
                job = module._WIRE_PROBE._WindowsProcessJob(
                    "job", api, process
                )
                ownership = module.ProcessOwnership(
                    process, original, None, job
                )
                with mock.patch.object(
                    module._WIRE_PROBE.os, "name", "nt"
                ), mock.patch.object(
                    module._WIRE_PROBE,
                    "_current_process_identity",
                    return_value=None,
                ), mock.patch.object(
                    module._WIRE_PROBE,
                    "_process_is_running",
                    return_value=False,
                ):
                    result = ownership.quiesce({original}, 0.5)

                self.assertFalse(result.complete)
                self.assertTrue(
                    any(expected in detail for detail in result.incomplete),
                    result.incomplete,
                )

    def test_smoke_rejects_incomplete_job_cleanup_evidence(self) -> None:
        module = load_module()

        class Result:
            survivors = set()
            incomplete = ("Windows Job Object still owns 1 active process",)
            active_processes = 1

        class Ownership:
            @staticmethod
            def quiesce(identities, timeout):
                return Result()

        with self.assertRaisesRegex(
            SystemExit, "cleanup evidence is incomplete.*1 active process"
        ):
            module._quiesce_owned_processes(
                Ownership(),
                {module.ProcessIdentity(99, None, None, "start-99")},
                0.5,
            )

    @unittest.skipUnless(os.name == "nt", "Windows Job Object integration")
    def test_windows_job_object_reaps_root_and_child(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "job-pids.txt"
            server = root / "job-root.py"
            server.write_text(
                textwrap.dedent(
                    """
                    import os
                    import subprocess
                    import sys
                    import time
                    from pathlib import Path

                    child = subprocess.Popen(
                        [sys.executable, "-c", "import time; time.sleep(60)"]
                    )
                    Path(sys.argv[1]).write_text(
                        f"{os.getpid()} {child.pid}", encoding="utf-8"
                    )
                    time.sleep(60)
                    """
                ),
                encoding="utf-8",
            )
            session = module.McpSession(
                [sys.executable, str(server), str(pid_path)],
                os.environ.copy(),
                2.0,
                cwd=root,
            )
            try:
                deadline = time.monotonic() + 2.0
                while not pid_path.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(pid_path.exists(), "job fixture did not start")
                pids = {
                    int(value)
                    for value in pid_path.read_text(encoding="utf-8").split()
                }
                ownership = session.process_ownership
                # Голый PID после очистки может достаться чужому процессу,
                # поэтому выживших ищем по идентичности со временем создания.
                identities = ownership.capture_identities(pids)
                self.assertEqual({identity.pid for identity in identities}, pids)

                result = ownership.terminate(timeout_seconds=1.0)

                self.assertTrue(result.complete, result.incomplete)
                self.assertEqual(result.active_processes, 0)
                self.assertEqual(ownership.survivors(identities), set())
            finally:
                session.terminate_tree(root)

    def test_posix_tree_cleanup_continues_after_one_signal_error(self) -> None:
        module = load_module()
        calls: list[int] = []
        identities = {
            module.ProcessIdentity(pid, 1, 99, f"start-{pid}")
            for pid in (41, 42, 99)
        }

        class Process:
            pid = 99

        ownership = module.ProcessOwnership(Process(), None, 99)

        def signal_process(pid, signal_number):
            calls.append(pid)
            if len(calls) == 1:
                raise PermissionError("signal denied")

        with mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identities",
            side_effect=lambda pids: {
                identity.pid: identity
                for identity in identities
                if identity.pid in pids
            },
        ), mock.patch.object(
            module._WIRE_PROBE.os, "kill", side_effect=signal_process
        ):
            ownership.signal(identities, signal.SIGTERM)

        self.assertEqual(calls, [99, 42, 41])

    def test_workspace_service_observation_preserves_the_first_identity(self) -> None:
        module = load_module()
        original = module.ProcessIdentity(41, 1, 99, "original-start")
        replacement = module.ProcessIdentity(41, 1, 99, "replacement-start")

        class Process:
            pid = 99

        session = module.McpSession.__new__(module.McpSession)
        session.process = Process()
        session.process_ownership = module.ProcessOwnership(
            session.process, None, None
        )
        session._service_identities = {}
        self.assertTrue(
            hasattr(session, "observe_workspace_services"),
            "smoke session does not capture a service identity when first observed",
        )

        with mock.patch.object(
            module, "_workspace_service_pids", return_value={41}
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identities",
            return_value={41: original},
        ):
            session.observe_workspace_services(Path("cache"))
        with mock.patch.object(
            module, "_workspace_service_pids", return_value={41}
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identities",
            return_value={41: replacement},
        ):
            session.observe_workspace_services(Path("cache"))

        self.assertEqual(session.observed_service_identities(), {original})

    def test_stale_recorded_service_pid_is_not_promoted_at_cleanup(self) -> None:
        module = load_module()
        original = module.ProcessIdentity(41, 1, 99, "original-start")
        replacement = module.ProcessIdentity(41, 1, 99, "replacement-start")

        class Process:
            pid = 99
            stdin = None
            stdout = None
            stderr = None

            @staticmethod
            def wait(timeout):
                return 0

        session = module.McpSession.__new__(module.McpSession)
        session.process = Process()
        session.process_ownership = module.ProcessOwnership(
            session.process, None, None
        )
        session._service_identities = {41: original}
        signalled: list[int] = []

        with mock.patch.object(
            module, "_workspace_service_pids", return_value={41}
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_posix_process_snapshot",
            return_value={41: replacement},
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_current_process_identity",
            return_value=replacement,
        ), mock.patch.object(
            module._WIRE_PROBE,
            "_process_is_running",
            return_value=True,
        ), mock.patch.object(
            module._WIRE_PROBE.os,
            "kill",
            side_effect=lambda pid, signal_number: signalled.append(pid),
        ):
            session.terminate_tree(Path("cache"))

        self.assertNotIn(
            41,
            signalled,
            "cleanup promoted a stale recorded PID into a replacement identity",
        )

    def test_waits_for_short_lived_workspace_service_before_temp_cleanup(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            record = cache_root / "services" / "svc-test" / "service.json"
            record.parent.mkdir(parents=True)
            record.write_text("{}\n", encoding="utf-8")

            def retire_service() -> None:
                time.sleep(0.05)
                record.unlink()

            worker = threading.Thread(target=retire_service)
            worker.start()
            try:
                module._wait_for_workspace_services(cache_root, FUSE_SECONDS)
            finally:
                worker.join(timeout=FUSE_SECONDS)

            self.assertFalse(worker.is_alive())
            self.assertFalse(record.exists())

    def test_requests_workspace_service_shutdown_before_temp_cleanup(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            record = cache_root / "services" / "svc-test" / "service.json"
            record.parent.mkdir(parents=True)
            listener = socket.socket()
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(FUSE_SECONDS)
            record.write_text(
                json.dumps(
                    {
                        "pid": os.getpid(),
                        "port": listener.getsockname()[1],
                        "token": "smoke-secret",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            received: list[dict] = []

            def serve_shutdown() -> None:
                try:
                    connection, _ = listener.accept()
                except (OSError, TimeoutError):
                    return
                with listener, connection:
                    payload = connection.makefile("rb").readline()
                    received.append(json.loads(payload))
                    connection.sendall(
                        b'{"ok":true,"status":"shutdown","shutdown":true}\n'
                    )
                record.unlink()

            worker = threading.Thread(target=serve_shutdown)
            worker.start()
            try:
                service_pids = module._shutdown_workspace_services(
                    cache_root, FUSE_SECONDS
                )
                module._wait_for_workspace_services(cache_root, FUSE_SECONDS)
            finally:
                listener.close()
                worker.join(timeout=FUSE_SECONDS)

            self.assertFalse(worker.is_alive())
            self.assertEqual(service_pids, {os.getpid()})

            self.assertEqual(
                received,
                [
                    {
                        "token": "smoke-secret",
                        "kind": {"type": "shutdown"},
                    }
                ],
            )

    def test_waits_for_workspace_service_process_after_record_is_removed(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            process = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(0.15)"]
            )
            try:
                module._wait_for_workspace_services(
                    cache_root,
                    FUSE_SECONDS,
                    {process.pid},
                )
                self.assertFalse(module._process_is_running(process.pid))
            finally:
                if process.poll() is None:
                    process.terminate()
                process.wait(timeout=1.0)

    def test_close_failure_still_shuts_down_and_waits_for_workspace_services(self) -> None:
        module = load_module()
        events: list[object] = []

        class FailingSession:
            def close(self) -> None:
                events.append("close")
                raise SystemExit("MCP close failed")

            def terminate_tree(
                self, root: Path, known_service_identities: set[object]
            ) -> None:
                events.append(("terminate", root, known_service_identities))

        cache_root = Path("cache")
        module._shutdown_workspace_services = lambda root, timeout: (
            events.append(("shutdown", root, timeout)) or {41}
        )
        module._wait_for_workspace_services = lambda root, timeout, pids: events.append(
            ("wait", root, timeout, pids)
        )

        with self.assertRaisesRegex(SystemExit, "MCP close failed"):
            module._close_session_and_workspace_services(
                FailingSession(), cache_root, 7.0
            )

        self.assertEqual(
            events,
            [
                ("shutdown", cache_root, 7.0),
                "close",
                ("wait", cache_root, 7.0, {41}),
                ("terminate", cache_root, set()),
            ],
        )

    def test_workspace_shutdown_precedes_close_when_service_holds_mcp_pipes(self) -> None:
        module = load_module()
        service_released = threading.Event()
        events: list[object] = []

        class PipeHoldingSession:
            def close(self) -> None:
                events.append("close-started")
                if not service_released.wait(0.1):
                    raise AssertionError(
                        "session close waited on pipes still held by the workspace service"
                    )
                events.append("close-finished")

        cache_root = Path("cache")

        def shutdown(root: Path, timeout: float) -> set[int]:
            events.append(("shutdown", root, timeout))
            service_released.set()
            return {41}

        module._shutdown_workspace_services = shutdown
        module._wait_for_workspace_services = lambda root, timeout, pids: events.append(
            ("wait", root, timeout, pids)
        )

        module._close_session_and_workspace_services(
            PipeHoldingSession(), cache_root, 7.0
        )

        self.assertEqual(
            events,
            [
                ("shutdown", cache_root, 7.0),
                "close-started",
                "close-finished",
                ("wait", cache_root, 7.0, {41}),
            ],
        )

    def test_successful_close_reaps_captured_provider_descendants(self) -> None:
        module = load_module()
        events: list[object] = []
        identities = {
            module.ProcessIdentity(pid, 1, 99, f"start-{pid}")
            for pid in (41, 42, 99)
        }

        class Ownership:
            def snapshot(self, service_pids: set[int]):
                events.append(("capture", service_pids))
                return identities

            def quiesce(self, owned, timeout):
                events.append(("quiesce", owned, timeout))
                return module.ProcessCleanupResult(set())

        ownership = Ownership()

        class Session:
            @staticmethod
            def _process_ownership():
                return ownership

            @staticmethod
            def close() -> None:
                events.append("close")

            @staticmethod
            def observed_service_identities():
                return {
                    identity for identity in identities if identity.pid == 41
                }

        cache_root = Path("cache")

        with mock.patch.object(
            module, "_workspace_service_pids", return_value={41}
        ), mock.patch.object(
            module,
            "_shutdown_workspace_services",
            side_effect=lambda root, timeout: events.append(
                ("shutdown", root, timeout)
            )
            or {41},
        ), mock.patch.object(
            module,
            "_wait_for_workspace_services",
            side_effect=lambda root, timeout, pids: events.append(
                ("wait-services", root, timeout, pids)
            ),
        ):
            module._close_session_and_workspace_services(
                Session(), cache_root, 7.0
            )

        self.assertEqual(
            events[0],
            ("capture", {identity for identity in identities if identity.pid == 41}),
        )
        self.assertEqual(
            events[-1],
            ("quiesce", identities, module._WIRE_PROBE._CLEANUP_GRACE_SECONDS),
        )

    def test_shutdown_failure_does_not_claim_unobserved_recorded_service_pid(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            cache_root = Path(directory) / "cache"
            record = cache_root / "services" / "svc-test" / "service.json"
            record.parent.mkdir(parents=True)
            public = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                start_new_session=os.name == "posix",
            )
            service = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)",
                ],
            )
            record.write_text(json.dumps({"pid": service.pid}), encoding="utf-8")
            session = module.McpSession.__new__(module.McpSession)
            session.process = public
            session.close = lambda: None
            module._shutdown_workspace_services = lambda root, timeout: (_ for _ in ()).throw(
                SystemExit("shutdown failed")
            )
            module._wait_for_workspace_services = lambda root, timeout, pids: (_ for _ in ()).throw(
                SystemExit("wait failed")
            )
            try:
                with self.assertRaisesRegex(SystemExit, "wait failed"):
                    module._close_session_and_workspace_services(
                        session, cache_root, 0.1
                )
                assert_processes_exit(
                    self, module, {public.pid}, "cleanup left the owned public process alive"
                )
                self.assertTrue(module._process_is_running(service.pid))
            finally:
                for process in (service, public):
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=2.0)

    def test_review_ledger_resolution_handles_source_and_packaged_plugin_roots(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            manifest = root / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            review_path = root / "tests/fixtures/v013/tool-surface-review.json"
            review_path.parent.mkdir(parents=True)
            review_path.write_text(
                json.dumps({"unica.xdto.info": {}, "unica.xdto.edit": {}}),
                encoding="utf-8",
            )
            source_plugin = root / "plugins/unica"
            packaged_plugin = root / ".build/thin/marketplace/plugins/unica"
            source_plugin.mkdir(parents=True, exist_ok=True)
            packaged_plugin.mkdir(parents=True)

            for plugin_root in (source_plugin, packaged_plugin):
                with self.subTest(plugin_root=plugin_root):
                    self.assertEqual(
                        module.expected_tool_names(plugin_root),
                        {"unica.xdto.info", "unica.xdto.edit"},
                    )

    def test_review_ledger_resolution_rejects_ledger_outside_checkout(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            outer = Path(directory)
            unrelated = outer / "tests/fixtures/v013/tool-surface-review.json"
            unrelated.parent.mkdir(parents=True)
            unrelated.write_text(
                json.dumps({"unica.source.read": {}}),
                encoding="utf-8",
            )
            checkout = outer / "checkout"
            (checkout / "Cargo.toml").parent.mkdir(parents=True)
            (checkout / "Cargo.toml").write_text(
                "[workspace]\n", encoding="utf-8"
            )
            manifest = checkout / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            packaged_plugin = checkout / ".build/thin/marketplace/plugins/unica"
            packaged_plugin.mkdir(parents=True)

            with self.assertRaisesRegex(
                SystemExit,
                "tool-surface-review.json.*checkout root",
            ):
                module.expected_tool_names(packaged_plugin)

    def tool_entries(self, names: set[str] | None = None) -> list[object]:
        module = load_module()
        stable_schemas = json.loads(
            json.dumps(
                {
                    **module.EXPECTED_SOURCE_INPUT_SCHEMAS,
                    **module.EXPECTED_XDTO_INPUT_SCHEMAS,
                }
            )
        )
        selected_names = self.expected_tools() if names is None else names
        return [
            {
                "name": name,
                "inputSchema": stable_schemas.get(
                    name,
                    {
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": False,
                    },
                ),
                **(
                    {"outputSchema": module.EXPECTED_META_OUTPUT_SCHEMA}
                    if name in module.META_TOOL_NAMES
                    else {}
                ),
            }
            for name in sorted(selected_names)
        ]

    def run_smoke(
        self,
        tool_entries: list[object],
        *,
        server_name: str = "unica",
        instructions: str = "",
        result_drift: bool = False,
        provider_revision: bool = False,
        read_writes: bool = False,
        code_search_ok: bool = True,
        code_search_status: str = "ok",
        code_search_root_field: str | None = None,
        leave_pipe_holder: bool = False,
        request_timeout_seconds: float = FUSE_SECONDS,
    ) -> subprocess.CompletedProcess[str]:
        expected_tools = self.expected_tools()
        module = load_module()
        source_flows = json.loads(
            json.dumps(module.EXPECTED_SOURCE_FLOW_PROJECTIONS)
        )
        if result_drift:
            source_flows["main"]["resolve"]["summary"] = (
                "source.resolve returned a drifted result"
            )
        if provider_revision:
            source_flows["main"]["resolve"]["data"]["candidates"][0][
                "providerRevision"
            ] = "private"
        server_source = textwrap.dedent("""
            import json
            import sys
            from pathlib import Path

            tools = json.loads(r'''__TOOLS__''')
            source_flows = json.loads(r'''__SOURCE_FLOWS__''')
            if __LEAVE_PIPE_HOLDER__:
                import subprocess
                # Inherits stdout on purpose: a detached descendant keeps the
                # smoke's pipes open after this server exits.
                subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
            read_writes = __READ_WRITES__
            code_search_ok = __CODE_SEARCH_OK__
            code_search_status = __CODE_SEARCH_STATUS__
            code_search_root_field = __CODE_SEARCH_ROOT_FIELD__

            def materialize(value, cwd):
                if isinstance(value, dict):
                    return {
                        key: materialize(child, cwd)
                        for key, child in value.items()
                    }
                if isinstance(value, list):
                    return [materialize(child, cwd) for child in value]
                if value == "<cache-root>":
                    return str(Path(cwd).parent / "cache")
                if value == "<workspace-epoch>":
                    return 1
                return value

            def source_payload(name, args):
                if name in {"unica.source.resolve", "unica.source.children"}:
                    source_set = args["sourceSet"]
                    operation = name.rsplit(".", 1)[1]
                elif name == "unica.source.resources":
                    source_set = args["sourceSet"]
                    operation = "resources"
                else:
                    source_set = args["snapshotId"].rsplit("-", 1)[0]
                    operation = "read"

                payload = materialize(
                    source_flows[source_set][operation],
                    args["cwd"],
                )
                if name == "unica.source.resources":
                    payload["data"]["snapshotId"] = source_set + "-old"
                    payload["data"]["resources"][0]["resourceId"] = (
                        source_set + "-resource-old"
                    )
                elif name == "unica.source.read":
                    payload["data"]["snapshotId"] = args["snapshotId"]
                    payload["data"]["resourceId"] = args["resourceId"]
                    if read_writes:
                        relative = "src" if source_set == "main" else "ext"
                        Path(
                            args["cwd"],
                            relative,
                            "CommonModules/Shared/Ext/Module.bsl",
                        ).write_bytes(b"a read must never write")
                return payload

            def operation_result(ok, summary, diagnostics=None):
                return {
                    "ok": ok,
                    "summary": summary,
                    "changes": [],
                    "warnings": [],
                    "errors": [] if ok else [summary],
                    "artifacts": [],
                    "cache": {
                        "mode": "read", "root": "", "workspace_epoch": 0,
                        "events": [], "invalidated": [], "refreshed": [],
                        "lazy_rebuilt": [], "stale": [], "fresh": [],
                    },
                    **({"diagnostics": diagnostics} if diagnostics is not None else {}),
                }

            for line in sys.stdin:
                message = json.loads(line)
                request_id = message.get("id")
                if message.get("method") == "initialize":
                    initialized = {"jsonrpc": "2.0", "id": request_id, "result": {"serverInfo": {"name": __NAME__}, "instructions": __INSTRUCTIONS__}}
                    # Written as bytes rather than printed: the point of the
                    # UTF-8 case is that real non-ASCII bytes cross the pipe,
                    # and `print` would re-encode them in the console codec.
                    sys.stdout.buffer.write(json.dumps(initialized, ensure_ascii=False).encode("utf-8") + b"\\n")
                    sys.stdout.buffer.flush()
                elif message.get("method") == "tools/list":
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": {"tools": tools}}), flush=True)
                elif message.get("method") == "tools/call":
                    name = message["params"]["name"]
                    args = message["params"]["arguments"]
                    if name == "unica.role.info":
                        # The stub answers the three
                        # outcomes the smoke asserts and nothing else.
                        if "sourceSet" in args and "RightsPath" in args:
                            payload = operation_result(
                                False,
                                "selector_conflict: unica.role.info accepts either"
                                " `sourceSet` + `metadataPath` or `RightsPath`,"
                                " not both",
                            )
                        elif args.get("metadataPath") == "Role.SmokeRole":
                            payload = operation_result(True, "role inspected")
                            payload["data"] = {"name": "SmokeRole"}
                        else:
                            payload = operation_result(
                                False, "target_not_found: the logical target was not found"
                            )
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}]
                        }
                    elif (
                        name == "unica.source.resolve"
                        and args.get("query") == "Role.SmokeRole"
                    ):
                        payload = operation_result(True, "source.resolve returned 1 canonical candidate(s)")
                        payload["data"] = {
                            "candidates": [
                                {
                                    "displayName": "SmokeRole",
                                    "matchKind": "exact",
                                    "metadataPath": "Role.SmokeRole",
                                    "targetKind": "metadataObject",
                                }
                            ],
                            "completeness": "complete",
                        }
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}]
                        }
                    elif name == "unica.meta.info":
                        if args:
                            payload = operation_result(True, "metadata information inspected")
                        else:
                            payload = operation_result(False, "metadata arguments are invalid", [{"code": "invalid_arguments"}])
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}],
                            "structuredContent": payload,
                            "isError": not payload["ok"],
                        }
                    elif name == "unica.code.search":
                        payload = operation_result(code_search_ok, "code search completed")
                        bsl_section = {
                            "provider": "bsl-analyzer",
                            "status": code_search_status,
                            "hits": [] if code_search_status != "ok" else [
                                {
                                    "rank": 1,
                                    "path": "CommonModules/Shared/Ext/Module.bsl",
                                    "line": 1,
                                    "symbol": "Run",
                                    "snippet": "Procedure Run()",
                                    "attributes": {},
                                }
                            ],
                            "diagnostics": (
                                ["provider failed"]
                                if code_search_status == "failed"
                                else []
                            ),
                            "artifacts": [],
                        }
                        if code_search_root_field is not None:
                            bsl_section[code_search_root_field] = "private-root"
                        payload["data"] = {
                            "sections": [
                                {
                                    "provider": "rlm", "status": "empty", "hits": [],
                                    "diagnostics": [], "artifacts": [],
                                },
                                bsl_section,
                                {
                                    "provider": "git-grep", "status": "empty", "hits": [],
                                    "diagnostics": [], "artifacts": [],
                                },
                            ]
                        }
                        result = {
                            "content": [{"type": "text", "text": json.dumps(payload)}],
                            "isError": False,
                        }
                    else:
                        payload = source_payload(name, args)
                        result = {"content": [{"type": "text", "text": json.dumps(payload)}]}
                    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)
        """)
        server_source = (
            server_source.replace(
                "__TOOLS__",
                json.dumps(tool_entries, ensure_ascii=False, indent=2),
            )
            .replace(
                "__SOURCE_FLOWS__",
                json.dumps(source_flows, ensure_ascii=False, indent=2),
            )
            .replace("__READ_WRITES__", repr(read_writes))
            .replace("__LEAVE_PIPE_HOLDER__", repr(leave_pipe_holder))
            .replace("__CODE_SEARCH_OK__", repr(code_search_ok))
            .replace("__CODE_SEARCH_STATUS__", repr(code_search_status))
            .replace("__CODE_SEARCH_ROOT_FIELD__", repr(code_search_root_field))
            .replace("__NAME__", repr(server_name))
            .replace("__INSTRUCTIONS__", repr(instructions))
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / "server.py"
            server.write_text(server_source, encoding="utf-8")
            review_path = root / "tests/fixtures/v013/tool-surface-review.json"
            review_path.parent.mkdir(parents=True)
            review_path.write_text(
                json.dumps({name: {} for name in sorted(expected_tools)}),
                encoding="utf-8",
            )
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            manifest = root / "plugins/unica/.codex-plugin/plugin.json"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("{}\n", encoding="utf-8")
            plugin_root = root / "packaged/plugins/unica"
            plugin_root.mkdir(parents=True)
            return subprocess.run(
                [
                    sys.executable,
                    str(SMOKE_SCRIPT),
                    "--binary",
                    sys.executable,
                    "--binary-arg",
                    str(server),
                    "--plugin-root",
                    str(plugin_root),
                    "--timeout-seconds",
                    str(request_timeout_seconds),
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=request_timeout_seconds + FUSE_SECONDS,
            )

    def test_accepts_exact_v13_compatibility_surface(self) -> None:
        module = load_module()
        expected = module.V13_COMPATIBILITY_TOOL_NAMES

        module._stable_tool_contract(self.tool_entries(expected), expected)

    def test_rejects_v13_surface_missing_a_canonical_tool(self) -> None:
        module = load_module()
        expected = module.V13_COMPATIBILITY_TOOL_NAMES

        with self.assertRaisesRegex(SystemExit, "missing: unica.search"):
            module._stable_tool_contract(
                self.tool_entries(expected - {"unica.search"}),
                expected,
            )

    def test_v13_payload_uses_structured_content_without_text_duplication(self) -> None:
        module = load_module()
        payload = {
            "ok": True,
            "summary": "workspace is ready",
            "data": {"status": "passed", "ready": True},
            "rev": "unica-read-set-sha256-v1:test",
        }

        self.assertEqual(
            module._v13_tool_payload(
                {
                    "result": {
                        "content": [],
                        "structuredContent": payload,
                        "isError": False,
                    }
                }
            ),
            payload,
        )

    def test_source_snapshot_excludes_only_the_canonical_internal_cache(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".build/unica/source-revisions").mkdir(parents=True)
            (root / ".build/other").mkdir(parents=True)
            (root / "src").mkdir()
            (root / ".build/unica/source-revisions/revision.json").write_text(
                "{}", encoding="utf-8"
            )
            (root / ".build/other/evidence.txt").write_text(
                "visible", encoding="utf-8"
            )
            (root / "src/Configuration.xml").write_text(
                "<Configuration/>", encoding="utf-8"
            )

            self.assertEqual(
                set(module._source_snapshot(root)),
                {".build/other/evidence.txt", "src/Configuration.xml"},
            )

    def test_cleanup_failure_does_not_replace_the_failure_that_caused_it(self) -> None:
        result = self.run_smoke(
            self.tool_entries(),
            server_name="other",
            leave_pipe_holder=True,
            request_timeout_seconds=PIPE_HOLDER_REQUEST_CAP_SECONDS,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertNotEqual(result.returncode, 124, result.stderr)
        self.assertIn("serverInfo.name must be 'unica'", result.stderr)
        self.assertIn("cleanup after that failure also failed", result.stderr)
        self.assertIn("reader threads did not stop", result.stderr)

    def test_rejects_a_server_name_other_than_unica(self) -> None:
        # INV-MCP-SERVER-NAME fixes the published identity of the server. A
        # release smoke that accepts any name cannot prove the invariant holds
        # in the artifact it is smoking.
        result = self.run_smoke(self.tool_entries(), server_name="Уника")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("serverInfo", result.stderr)
        self.assertIn("Уника", result.stderr)

    @staticmethod
    def typed_code_search_output_schema() -> dict[str, object]:
        return {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "data": {
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {
                        "coverage": {"type": "string"},
                        "elapsedMs": {"type": "integer"},
                        "sections": {
                            "type": "array",
                            "minItems": 3,
                            "maxItems": 3,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "role": {"type": "string"},
                                    "provider": {"type": "string"},
                                    "status": {"type": "string"},
                                    "termination": {
                                        "oneOf": [
                                            {"type": "null"},
                                            {
                                                "type": "object",
                                                "additionalProperties": False,
                                                "properties": {
                                                    "code": {
                                                        "type": "string",
                                                        "enum": [
                                                            "limitReached",
                                                            "deadlineExceeded",
                                                            "dependencyPending",
                                                            "unsupportedScope",
                                                            "capacityExhausted",
                                                            "providerUnavailable",
                                                            "providerFailed",
                                                        ],
                                                    },
                                                    "retryable": {"type": "boolean"},
                                                    "detailCode": {
                                                        "type": "string",
                                                        "minLength": 1,
                                                    },
                                                },
                                                "required": ["code", "retryable"],
                                            },
                                        ]
                                    },
                                    "searchComplete": {"type": "boolean"},
                                    "ranking": {"type": "string"},
                                    "ordering": {"type": "string"},
                                    "matches": {
                                        "type": "object",
                                        "properties": {
                                            "returned": {"type": "integer"},
                                            "total": {"type": "integer"},
                                            "relation": {"type": "string"},
                                        },
                                        "required": ["returned", "relation"],
                                    },
                                    "hits": {"type": "array"},
                                    "diagnostics": {"type": "array"},
                                },
                                "required": [
                                    "role",
                                    "provider",
                                    "status",
                                    "termination",
                                    "searchComplete",
                                    "ranking",
                                    "ordering",
                                    "matches",
                                    "hits",
                                    "diagnostics",
                                ],
                            },
                        },
                    },
                    "required": ["coverage", "elapsedMs", "sections"],
                }
            },
            "required": ["data"],
        }

    def test_unavailable_bsl_analyzer_is_terminal_even_with_building_prose(self) -> None:
        """`unavailable` is permanent; prose in the diagnostics cannot soften it.

        The building state now arrives as `timedOut`/`dependencyPending`, so an
        `unavailable` carrying the old sentence is a provider defect to surface,
        not a wait to keep taking.
        """
        payload = {
            "data": {
                "sections": [
                    {"provider": "rlm", "status": "empty", "hits": []},
                    {
                        "provider": "bsl-analyzer",
                        "status": "unavailable",
                        "hits": [],
                        "diagnostics": [
                            "Search index is being built, please try again in a moment."
                        ],
                    },
                    {"provider": "git-grep", "status": "empty", "hits": []},
                ]
            }
        }

        with self.assertRaises(SystemExit):
            load_module()._bsl_search_is_ready(payload)

    def test_retries_bsl_analyzer_while_search_index_is_being_built(self) -> None:
        payload = {
            "data": {
                "sections": [
                    {
                        "provider": "rlm",
                        "status": "empty",
                        "hits": [],
                        "diagnostics": [],
                        "artifacts": [],
                    },
                    {
                        "provider": "bsl-analyzer",
                        "status": "timedOut",
                        "termination": {
                            "code": "dependencyPending",
                            "retryable": True,
                            "detailCode": "buildingIndex",
                        },
                        "hits": [],
                        "diagnostics": [
                            "Search index is being built, please try again in a moment."
                        ],
                        "artifacts": [],
                    },
                    {
                        "provider": "git-grep",
                        "status": "empty",
                        "hits": [],
                        "diagnostics": [],
                        "artifacts": [],
                    },
                ]
            }
        }

        self.assertFalse(load_module()._bsl_search_is_ready(payload))

    def test_bsl_search_retries_the_real_request_loop_until_ready(self) -> None:
        module = load_module()
        unavailable = {
            "ok": False,
            "data": {
                "sections": [
                    {"provider": "rlm", "status": "empty", "hits": []},
                    {
                        "provider": "bsl-analyzer",
                        "status": "timedOut",
                        "termination": {
                            "code": "dependencyPending",
                            "retryable": True,
                            "detailCode": "buildingIndex",
                        },
                        "hits": [],
                        "diagnostics": ["Search index is being built"],
                    },
                    {"provider": "git-grep", "status": "empty", "hits": []},
                ]
            },
        }
        ready = json.loads(json.dumps(unavailable))
        ready["ok"] = True
        ready["data"]["sections"][1] = {
            "provider": "bsl-analyzer",
            "status": "ok",
            "hits": [
                {
                    "path": "CommonModules/Shared/Ext/Module.bsl",
                    "symbol": "Run",
                }
            ],
            "diagnostics": [],
        }

        class Session:
            def __init__(self) -> None:
                self.payloads = [unavailable, ready]
                self.request_ids: list[int] = []

            def request(self, message: dict) -> dict:
                self.request_ids.append(message["id"])
                payload = self.payloads.pop(0)
                return {
                    "result": {
                        "content": [{"text": json.dumps(payload)}],
                        "isError": not payload["ok"],
                    }
                }

        session = Session()
        original_sleep = module.time.sleep
        sleeps: list[float] = []
        module.time.sleep = sleeps.append
        try:
            next_id = module._exercise_bsl_search(session, 17, 1.0)
        finally:
            module.time.sleep = original_sleep

        self.assertEqual(next_id, 19)
        self.assertEqual(session.request_ids, [17, 18])
        self.assertEqual(sleeps, [0.5])


if __name__ == "__main__":
    unittest.main()
