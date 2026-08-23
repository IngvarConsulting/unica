from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PROBE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "probe-unica-wire.py"
BASELINE_FIXTURE = (
    REPO_ROOT / "tests" / "fixtures" / "migration" / "v0.12.3-baseline.json"
)
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
        notifications_only_on_last_page: bool = False,
    ) -> Path:
        server = root / "wire-server.py"
        server.write_text(
            textwrap.dedent(
                f"""
                import json
                import sys
                import time
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
                        if {notifications_only_on_last_page!r} and page_index == len(pages) - 1:
                            for sequence in range(20):
                                print(json.dumps({{
                                    "jsonrpc": "2.0",
                                    "method": "notifications/progress",
                                    "params": {{"progress": sequence}},
                                }}), flush=True)
                                time.sleep(0.02)
                            continue
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
        timeout_seconds: float = 2.0,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
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
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=4.0,
        )

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
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = self.write_server(
                root,
                pages=[["unica.alpha"], ["unica.beta"]],
                capture_path=root / "requests.json",
                notifications_only_on_last_page=True,
            )
            started = time.monotonic()
            result = self.run_probe(
                server,
                root / "wire.json",
                timeout_seconds=0.08,
            )
            elapsed = time.monotonic() - started

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aggregate deadline", result.stderr)
        self.assertLess(elapsed, 0.35, "notifications restarted the aggregate deadline")

    def test_aggregate_timeout_reaps_the_detached_process_tree(self) -> None:
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
                            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)",
                        ],
                        start_new_session=True,
                    )
                    Path({str(pid_path)!r}).write_text(
                        f"{{os.getpid()}} {{child.pid}}", encoding="utf-8"
                    )
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
            result = self.run_probe(
                server,
                root / "wire.json",
                timeout_seconds=0.1,
            )
            pids = [int(value) for value in pid_path.read_text(encoding="utf-8").split()]
            module = load_module()
            try:
                for pid in pids:
                    with self.subTest(pid=pid):
                        self.assertFalse(module._process_is_running(pid))
            finally:
                for pid in pids:
                    if module._process_is_running(pid):
                        os.kill(pid, signal.SIGKILL)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aggregate deadline", result.stderr)

    @unittest.skipUnless(os.name == "posix", "POSIX session ownership regression")
    def test_parent_exit_before_snapshot_has_bounded_cleanup_and_reaps_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "pids.txt"
            server = root / "parent-exits-first.py"
            server.write_text(
                textwrap.dedent(
                    f"""
                    import os
                    import signal
                    import subprocess
                    import sys
                    from pathlib import Path

                    child = subprocess.Popen(
                        [
                            sys.executable,
                            "-c",
                            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)",
                        ],
                        process_group=0,
                    )
                    Path({str(pid_path)!r}).write_text(
                        f"{{os.getpid()}} {{child.pid}}", encoding="utf-8"
                    )
                    """
                ),
                encoding="utf-8",
            )
            command = [
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
                str(root / "wire.json"),
                "--timeout-seconds",
                "0.15",
            ]
            started = time.monotonic()
            probe = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            bounded = True
            try:
                try:
                    stdout, stderr = probe.communicate(timeout=0.8)
                except subprocess.TimeoutExpired:
                    bounded = False
                    probe.kill()
                    stdout, stderr = probe.communicate(timeout=1.0)
                elapsed = time.monotonic() - started
                pids = [
                    int(value)
                    for value in pid_path.read_text(encoding="utf-8").split()
                ]
                module = load_module()
                running = [pid for pid in pids if module._process_is_running(pid)]
            finally:
                if probe.poll() is None:
                    probe.kill()
                    probe.wait(timeout=1.0)
                if pid_path.exists():
                    module = load_module()
                    for value in pid_path.read_text(encoding="utf-8").split():
                        pid = int(value)
                        if module._process_is_running(pid):
                            os.kill(pid, signal.SIGKILL)

        self.assertTrue(bounded, "cleanup blocked on pipes inherited by an orphan")
        self.assertLess(elapsed, 0.8)
        self.assertNotEqual(probe.returncode, 0, stdout)
        self.assertIn("aggregate deadline", stderr)
        self.assertEqual(running, [], f"orphaned probe processes survived: {running}")

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
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_path = root / "pids.txt"
            ready_path = root / "escaped-ready"
            signal_path = root / "escaped-signalled"
            server = root / "unregistered-session-escape.py"
            server.write_text(
                textwrap.dedent(
                    f"""
                    import os
                    import subprocess
                    import sys
                    import time
                    from pathlib import Path

                    child = subprocess.Popen(
                        [
                            sys.executable,
                            "-c",
                            "import os,signal,time; from pathlib import Path; "
                            "os.setsid(); "
                            "signal.signal(signal.SIGTERM, lambda *_: "
                            "Path({str(signal_path)!r}).write_text('term', encoding='utf-8')); "
                            "Path({str(ready_path)!r}).write_text('ready', encoding='utf-8'); "
                            "time.sleep(60)",
                        ]
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
            command = [
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
                str(root / "wire.json"),
                "--timeout-seconds",
                "0.15",
            ]
            started = time.monotonic()
            probe = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            child_pid = None
            bounded = True
            try:
                try:
                    stdout, stderr = probe.communicate(timeout=0.8)
                except subprocess.TimeoutExpired:
                    bounded = False
                    probe.kill()
                    stdout, stderr = probe.communicate(timeout=1.0)
                elapsed = time.monotonic() - started
                _, child_pid = [
                    int(value)
                    for value in pid_path.read_text(encoding="utf-8").split()
                ]
                module = load_module()
                child_was_not_claimed = module._process_is_running(child_pid)
            finally:
                if probe.poll() is None:
                    probe.kill()
                    probe.wait(timeout=1.0)
                if child_pid is None and pid_path.exists():
                    _, child_pid = [
                        int(value)
                        for value in pid_path.read_text(encoding="utf-8").split()
                    ]
                if child_pid is not None:
                    module = load_module()
                    if module._process_is_running(child_pid):
                        os.kill(child_pid, signal.SIGKILL)
                        module._wait_for_process_pids({child_pid}, 1.0)

        self.assertTrue(bounded, "escaped child kept probe cleanup blocked")
        self.assertLess(elapsed, 0.8)
        self.assertNotEqual(probe.returncode, 0, stdout)
        self.assertIn("aggregate deadline", stderr)
        self.assertTrue(
            child_was_not_claimed,
            "an unregistered process that escaped the owned session was signalled",
        )
        self.assertFalse(
            signal_path.exists(),
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
