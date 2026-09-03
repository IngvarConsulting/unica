from __future__ import annotations

import importlib.util
import json
import os
import queue
import subprocess
import tempfile
import threading
import time
import unittest
from contextlib import contextmanager
from pathlib import Path


MCP_HANDSHAKE_ID = "unica-ci-handshake"
MCP_INITIALIZE_PARAMS = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "unica-ci", "version": "1"},
}
MCP_HANDSHAKE = [
    {
        "jsonrpc": "2.0",
        "id": MCP_HANDSHAKE_ID,
        "method": "initialize",
        "params": MCP_INITIALIZE_PARAMS,
    },
    {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
]

MUTATION_TOOL_NAMES = frozenset(
    {
        "unica.cf.edit",
        "unica.cf.init",
        "unica.support.edit",
        "unica.cfe.borrow",
        "unica.cfe.init",
        "unica.epf.init",
        "unica.erf.init",
        "unica.cfe.patch_method",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.form.add",
        "unica.form.compile",
        "unica.form.edit",
        "unica.interface.edit",
        "unica.subsystem.compile",
        "unica.subsystem.edit",
        "unica.dcs.compile",
        "unica.dcs.edit",
        "unica.mxl.compile",
        "unica.role.compile",
        "unica.role.edit",
        "unica.build.dump",
        "unica.build.load",
        "unica.build.update",
        "unica.build.make",
        "unica.build.run",
        "unica.runtime.execute",
        "unica.runtime.job.start",
        "unica.runtime.job.cancel",
        "unica.code.patch",
    }
)


def source_smoke_oracle():
    script = Path(__file__).resolve().parents[2] / "scripts/ci/smoke-unica-mcp.py"
    spec = importlib.util.spec_from_file_location("smoke_unica_mcp_oracle", script)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def assert_no_physical_source_selectors(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"path", "sourceDir", "provider", "providerId", "providerRevision", "handle"}:
                raise AssertionError(f"source response leaked physical selector {key}")
            assert_no_physical_source_selectors(child)
    elif isinstance(value, list):
        for child in value:
            assert_no_physical_source_selectors(child)


def snapshot_workspace_files(root: Path) -> dict[str, bytes]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in root.rglob("*")
        if path.is_file()
    }


class UnicaMcpSmokeTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def call_mcp(self, messages: list[dict], *, cache_dir: Path | None = None) -> list[dict]:
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        process = subprocess.Popen(
            ["cargo", "run", "--quiet", "--bin", "unica", "--"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=self.repo_root(),
            env=env,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        deadline = time.monotonic() + 30
        lines: queue.Queue[str] = queue.Queue()

        def read_stdout() -> None:
            while True:
                line = process.stdout.readline()
                lines.put(line)
                if not line:
                    return

        reader = threading.Thread(target=read_stdout, daemon=True)
        reader.start()
        try:
            # The rmcp-based server requires the MCP handshake first; prepend it
            # unless the scenario drives initialize itself.
            injected_handshake = messages and messages[0].get("method") != "initialize"
            if injected_handshake:
                messages = MCP_HANDSHAKE + messages
            for message in messages:
                process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()

            expected_responses = sum("id" in message for message in messages)
            responses = []
            for _ in range(expected_responses):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail("timed out waiting for MCP response")
                try:
                    line = lines.get(timeout=remaining)
                except queue.Empty:
                    self.fail("timed out waiting for MCP response")
                if not line:
                    self.fail("MCP process exited before all responses arrived")
                responses.append(json.loads(line))

            process.stdin.close()
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self.fail("timed out waiting for MCP stdout EOF")
                try:
                    trailing = lines.get(timeout=remaining)
                except queue.Empty:
                    self.fail("timed out waiting for MCP stdout EOF")
                if not trailing:
                    break
                self.fail(f"unexpected MCP response after expected ids: {trailing.strip()}")
            return_code = process.wait(timeout=max(0.1, deadline - time.monotonic()))
            stderr = process.stderr.read()
            self.assertEqual(return_code, 0, stderr)
            return [r for r in responses if not injected_handshake or r.get("id") != MCP_HANDSHAKE_ID]
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            if process.poll() is None:
                process.kill()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            process.stdout.close()
            process.stderr.close()

    @contextmanager
    def mcp_session(
        self,
        *,
        cache_dir: Path | None = None,
        workdir: Path | None = None,
    ):
        env = os.environ.copy()
        if cache_dir is not None:
            env["UNICA_CACHE_DIR"] = str(cache_dir)
        process = subprocess.Popen(
            [
                "cargo",
                "run",
                "--quiet",
                "--manifest-path",
                str(self.repo_root() / "Cargo.toml"),
                "--bin",
                "unica",
                "--",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=workdir or self.repo_root(),
            env=env,
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        lines: queue.Queue[str] = queue.Queue()

        def read_stdout() -> None:
            for line in process.stdout:
                lines.put(line)
            lines.put("")

        threading.Thread(target=read_stdout, daemon=True).start()

        # The session outlives many calls, so stderr has to be drained too:
        # a full pipe buffer would stall the server and read as a timeout.
        diagnostics: list[str] = []

        def read_stderr() -> None:
            for line in process.stderr:
                diagnostics.append(line)

        threading.Thread(target=read_stderr, daemon=True).start()

        def request(message: dict) -> dict:
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            try:
                line = lines.get(timeout=30)
            except queue.Empty:
                self.fail(
                    "timed out waiting for interactive MCP response: "
                    + "".join(diagnostics)
                )
            if not line:
                self.fail(
                    "interactive MCP process exited before a response: "
                    + "".join(diagnostics)
                )
            response = json.loads(line)
            self.assertEqual(response.get("id"), message.get("id"), response)
            return response

        try:
            response = request({**MCP_HANDSHAKE[0], "id": 1})
            self.assertIn("result", response)
            process.stdin.write(json.dumps(MCP_HANDSHAKE[1]) + "\n")
            process.stdin.flush()
            yield request
        finally:
            try:
                if not process.stdin.closed:
                    try:
                        process.stdin.close()
                    except BrokenPipeError:
                        # The child may already be gone; reaping still has to run.
                        pass
                try:
                    return_code = process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    # Reap the killed child, otherwise it is left a zombie.
                    process.wait()
                    self.fail("interactive MCP process did not exit")
                self.assertEqual(return_code, 0, "".join(diagnostics))
            finally:
                # Closed on every exit path, including a failed assertion.
                for stream in (process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()

    def source_fixture(self, root: Path) -> dict[str, bytes]:
        (root / "src/CommonModules/Shared/Ext").mkdir(parents=True)
        (root / "ext/CommonModules/Shared/Ext").mkdir(parents=True)
        (root / "v8project.yaml").write_text(
            "format: DESIGNER\nsource-set:\n"
            "  - name: main\n    type: CONFIGURATION\n    path: src\n"
            "  - name: extension\n    type: EXTENSION\n    path: ext\n",
            encoding="utf-8",
        )
        xml = {}
        for source_set, name, method in [("src", "Main", "Run"), ("ext", "Extension", "RunExtension")]:
            descriptor = (
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
                f"<Configuration><Properties><Name>{name}</Name>"
                + ("<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>" if source_set == "ext" else "")
                + "</Properties><ChildObjects><CommonModule>Shared</CommonModule>"
                "</ChildObjects></Configuration></MetaDataObject>"
            ).encode()
            (root / source_set / "Configuration.xml").write_bytes(descriptor)
            module_descriptor = (
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
                "<CommonModule><Properties><Name>Shared</Name></Properties></CommonModule>"
                "</MetaDataObject>"
            ).encode()
            (root / source_set / "CommonModules/Shared.xml").write_bytes(module_descriptor)
            module_path = root / source_set / "CommonModules/Shared/Ext/Module.bsl"
            module_path.write_bytes((f"\ufeffProcedure {method}()\r\nEndProcedure\r\n").encode())
            xml[str(root / source_set / "Configuration.xml")] = descriptor
            xml[str(root / source_set / "CommonModules/Shared.xml")] = module_descriptor
        return xml

    def test_notifications_do_not_count_as_responses(self) -> None:
        responses = self.call_mcp(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": MCP_INITIALIZE_PARAMS},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": "already-complete", "reason": "smoke"},
                },
                {"jsonrpc": "2.0", "id": 2, "method": "ping"},
            ]
        )

        self.assertEqual([response["id"] for response in responses], [1, 2])

