"""Opt-in Windows acceptance of the real pinned dbgs provider through MCP."""

from __future__ import annotations

from contextlib import ExitStack
import ctypes
from ctypes import wintypes
from copy import deepcopy
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import unittest
from urllib.parse import urlsplit

from tests.ci.acceptance_controls import load_acceptance_corpus, tool_result
from tests.ci.acceptance_platform import configure_workspace, run_operation
from tests.ci.acceptance_profiles import DBGS_PROFILE, completed_delivery_call, select_profile, stage_tools
from scripts.ci.temp_tree import temp_tree

BUILD_ENV = "UNICA_DBGS_PLATFORM_BUILD"
START = "debug-server.start"
STOP = "debug-server.stop"
PREPARE = "debug-server.prepare"
EXIT_SECONDS = 30


def bind_wire(scenario, owner_pid, occupied_port):
    if type(owner_pid) is not int or not 1 <= owner_pid <= 0xFFFFFFFF:
        raise ValueError("expected a live owner PID")
    if type(occupied_port) is not int or not 1 <= occupied_port <= 65535:
        raise ValueError("expected an occupied TCP port")
    wire = deepcopy(scenario["wire"])
    for step in wire:
        args = step["args"].get("args", {})
        if args.get("ownerPid") == "$dbgsOwnerPid":
            args["ownerPid"] = owner_pid
        if args.get("port") == "$dbgsOccupiedPort":
            args["port"] = occupied_port
        for assertion in step.get("assertions", []):
            if assertion.get("eq") == "$dbgsOwnerPid":
                assertion["eq"] = owner_pid
            if isinstance(assertion.get("contains"), str):
                assertion["contains"] = assertion["contains"].replace("$dbgsOccupiedPort", str(occupied_port))
    return {**scenario, "wire": wire}


def rdbg_answer(status, payload):
    return status == 400 and b"virtual-resource-system" in payload and b"Exception" in payload


def rdbg_ready(port, host="127.0.0.1"):
    # Same read-only RDBG probe as v8-runner 0.18.1. http.client does not use
    # shell proxy settings or follow redirects. The body is deliberately empty.
    connection = http.client.HTTPConnection(host, port, timeout=3)
    try:
        connection.request("POST", "/e1crdbg/rdbg?cmd=getDbgTargets", b"", {
            "Content-Type": "application/xml", "Accept": "application/json",
        })
        response = connection.getresponse()
        return rdbg_answer(response.status, response.read(64 * 1024))
    except (OSError, http.client.HTTPException):
        return False
    finally:
        connection.close()


def acquired_server(result, owner_pid):
    data = result.get("data") or {}
    pid, port, session = (data.get(key) for key in ("pid", "port", "session"))
    if not (
        result.get("ok") is True and data.get("op") == START and data.get("contractVersion") == 1
        and data.get("action") == "started" and data.get("owner") == "runner"
        and data.get("ownerPid") == owner_pid and type(pid) is int and 0 < pid <= 0xFFFFFFFF
        and pid != owner_pid and type(port) is int and 1 <= port <= 65535
        and data.get("url") == f"http://127.0.0.1:{port}" and data.get("infoBaseAlias") == "DefAlias"
        and data.get("providerDispatched") is True
        and isinstance(session, str) and re.fullmatch(r"[0-9a-f]{32}", session)
    ):
        raise AssertionError("expected matching owned debug-server.start receipt")
    return pid, port, session


def tcp_listening(port, host="127.0.0.1"):
    try:
        with socket.create_connection((host, port), timeout=2):
            return True
    except OSError:
        return False


class ProcessWitness:
    def __init__(self, pid):
        win32 = ctypes.WinDLL("kernel32", use_last_error=True)
        win32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        win32.OpenProcess.restype = wintypes.HANDLE
        win32.QueryFullProcessImageNameW.argtypes = [
            wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD),
        ]
        win32.QueryFullProcessImageNameW.restype = wintypes.BOOL
        win32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        win32.WaitForSingleObject.restype = wintypes.DWORD
        win32.CloseHandle.argtypes = [wintypes.HANDLE]
        win32.CloseHandle.restype = wintypes.BOOL
        self.win32 = win32
        self.handle = win32.OpenProcess(0x101000, False, pid)  # wait + query limited information
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            name = ctypes.create_unicode_buffer(32768)
            length = wintypes.DWORD(len(name))
            if not win32.QueryFullProcessImageNameW(self.handle, 0, name, ctypes.byref(length)):
                raise ctypes.WinError(ctypes.get_last_error())
            self.image = Path(name.value)
        except BaseException:
            self.close()
            raise

    def exited(self, seconds=0):
        status = self.win32.WaitForSingleObject(self.handle, seconds * 1000)
        if status not in (0, 258):
            raise ctypes.WinError(ctypes.get_last_error())
        return status == 0

    def close(self):
        self.win32.CloseHandle(self.handle)


def close_owner(owner):
    if not owner.stdin.closed:
        owner.stdin.close()
    return owner.wait(timeout=10)


def check_public_prepare(test, server, operations, runner_binary, owner_pid):
    # The installed CLI provides an independent witness for discovery. Only
    # the acceptance test reads help; the product uses its pinned contract.
    help_text = subprocess.run(
        [str(runner_binary), "debug-server", "--help"], check=True, capture_output=True,
        text=True, encoding="utf-8", timeout=10,
    ).stdout
    supports_prepare = re.search(r"^\s*prepare(?:\s|$)", help_text, re.MULTILINE) is not None
    advertised = [entry for entry in operations if entry["op"] == PREPARE]
    test.assertEqual(len(advertised), int(supports_prepare))
    args = {"ownerPid": owner_pid}
    preview = tool_result(completed_delivery_call(
        server, "unica.run", {"op": PREPARE, "args": args, "dryRun": True},
        "preview debug-server.prepare", retry_pending_index=False,
    ))
    if not supports_prepare:
        test.assertFalse(preview["ok"], preview)
        test.assertEqual(preview["diagnostics"][0]["code"], "unsupported_operation")
        refused = tool_result(completed_delivery_call(
            server, "unica.run", {"op": PREPARE, "args": args, "dryRun": False},
            "request prepare with an older runner", retry_pending_index=False,
        ))
        test.assertFalse(refused["ok"], refused)
        test.assertEqual(refused["diagnostics"][0]["code"], "unsupported_operation")
        for result in (preview, refused):
            test.assertNotIn("release", result.get("data") or {})
            test.assertNotIn("url", result.get("data") or {})
        return
    test.assertTrue(advertised[0]["terminal"])
    test.assertFalse(advertised[0]["dryRunRequired"])
    test.assertEqual(set(advertised[0]["argsSchema"]["properties"]), {"ownerPid"})
    test.assertTrue(preview["ok"], preview)
    test.assertEqual(preview["data"], {"op": PREPARE, "contractVersion": 1, "action": "planned"})

    def release(session):
        result = run_operation(server, STOP, {"session": session}, "release the preparation lease")
        test.assertEqual(result["data"]["session"], session)
        return result["data"]

    def acquire():
        data = run_operation(server, PREPARE, args, "prepare debugger for dedicated file infobase")["data"]
        test.assertEqual(set(data["release"]), {"op", "args"})
        test.assertEqual(data["release"]["op"], STOP)
        test.assertEqual(set(data["release"]["args"]), {"session"})
        session = data["release"]["args"]["session"]
        test.assertRegex(session, r"^[0-9a-f]{32}$")
        return session, data

    first, prepared = acquire()
    pending = [first]
    try:
        second, reused = acquire()
        pending.append(second)
        test.assertNotEqual(first, second)
        for data in (prepared, reused):
            test.assertEqual(data["op"], PREPARE)
            test.assertEqual(data["action"], "prepared")
            test.assertEqual(data["contractVersion"], 1)
            test.assertEqual(data["scope"], "clientAndServer")
            test.assertEqual(data["infoBaseAlias"], "DefAlias")
        test.assertEqual(prepared["url"], reused["url"])
        endpoint = urlsplit(prepared["url"])
        test.assertEqual(endpoint.scheme, "http")
        test.assertIsNotNone(endpoint.port)
        if endpoint.hostname != "localhost":
            test.assertTrue(ipaddress.ip_address(endpoint.hostname).is_loopback)
        test.assertTrue(rdbg_ready(endpoint.port, endpoint.hostname))
        test.assertEqual(release(first)["action"], "released")
        pending.remove(first)
        test.assertTrue(rdbg_ready(endpoint.port, endpoint.hostname))
        test.assertEqual(release(second)["action"], "stopped")
        pending.remove(second)
        test.assertFalse(tcp_listening(endpoint.port, endpoint.hostname))
        test.assertEqual(release(second)["action"], "nothing_to_stop")
    finally:
        with ExitStack() as cleanup:
            for session in pending:
                cleanup.callback(release, session)


class NativeDbgsCase(unittest.TestCase):
    @unittest.skipUnless(
        sys.platform == "win32" and os.environ.get(BUILD_ENV),
        "opt-in Windows 1C: set UNICA_DBGS_PLATFORM_BUILD",
    )
    def test_corpus_debug_server(self):
        # Importing AcceptanceServer verifies the prepared product manifest;
        # this research test never invokes the implicit-build helper.
        self.assertTrue(os.environ.get("UNICA_PREPARED_BINARY_MANIFEST"), "prepare the exact product binary first")
        from tests.ci import test_acceptance_scenarios as acceptance

        build = os.environ[BUILD_ENV]
        self.assertRegex(build, r"^\d+\.\d+\.\d+\.\d+$")
        corpus = select_profile(load_acceptance_corpus(), DBGS_PROFILE)
        scenario, = corpus["scenarios"]
        witnesses = []
        with temp_tree("unica-dbgs-corpus-") as raw, ExitStack() as resources:
            plugin = stage_tools(Path(raw) / "plugin", ("v8-runner",))
            manifest = json.loads((plugin / "third-party/manifest.json").read_text(encoding="utf-8"))
            staged_version = manifest["tools"][0]["version"]
            listener = resources.enter_context(socket.socket(socket.AF_INET, socket.SOCK_STREAM))
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            listener.settimeout(3)
            listener.bind(("127.0.0.1", 0))
            listener.listen(4)
            occupied = listener.getsockname()[1]
            owner = subprocess.Popen(
                [sys.executable, "-I", "-B", "-c", "import sys; sys.stdin.buffer.read()"],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )

            def cleanup():
                try:
                    self.assertEqual(close_owner(owner), 0)
                    for witness in witnesses:
                        self.assertTrue(witness.exited(EXIT_SECONDS), "owned dbgs survived fixture cleanup")
                finally:
                    for witness in witnesses:
                        witness.close()
                    witnesses.clear()

            # run_corpus calls cleanup before removing the workspace. This
            # fallback also runs if it fails before entering its run loop.
            resources.callback(cleanup)

            def watch(result):
                pid, port, session = acquired_server(result, owner.pid)
                witness = ProcessWitness(pid)
                witnesses.append(witness)
                self.assertEqual(witness.image.name.lower(), "dbgs.exe")
                self.assertIn(build, witness.image.parts, "DBGS image must belong to the selected build")
                self.assertFalse(witness.exited(), "started receipt refers to an exited process")
                self.assertTrue(rdbg_ready(port), "server did not answer the pinned readiness protocol")
                return witness, port, session

            def prepare(server, _scenario):
                configure_workspace(server.workspace, build)
                connection = f'File="{(server.workspace / "ib").as_posix()}"'
                (server.workspace / "v8project.local.yaml").write_text(
                    "infobases:\n  origin:\n    connection: " + json.dumps(connection) + "\n",
                    encoding="utf-8",
                )
                run_operation(server, "infobase.create", {}, "prepare dedicated file infobase")
                ib = server.workspace / "ib/1Cv8.1CD"
                with ib.open("rb") as stream:
                    before = hashlib.file_digest(stream, "sha256").hexdigest()

                def finish(verification):
                    dictionary, _, start = [step["result"] for step in verification]
                    for operation in (START, STOP):
                        row, = [entry for entry in dictionary["data"]["operations"] if entry["op"] == operation]
                        self.assertTrue(row["terminal"])
                        self.assertEqual(row["support"]["adapter"], f"v8-runner/{staged_version}")
                    with socket.create_connection(("127.0.0.1", occupied), timeout=3):
                        connection, _ = listener.accept()
                        connection.close()
                    witness, port, session = watch(start)
                    released = run_operation(server, STOP, {"session": session}, "release own debug-server session")
                    data = released.get("data") or {}
                    self.assertTrue(data.get("session") == session, "stop returned a different session")
                    self.assertEqual(data.get("action"), "stopped")
                    self.assertIs(data.get("knownSession"), True)
                    self.assertTrue(witness.exited(EXIT_SECONDS), "own stop did not release dbgs")
                    self.assertFalse(tcp_listening(port))
                    self.assertIsNone(owner.poll(), "own stop killed the owner")

                    check_public_prepare(
                        self, server, dictionary["data"]["operations"],
                        plugin / manifest["tools"][0]["binaryPath"], owner.pid,
                    )
                    second = run_operation(server, START, {"ownerPid": owner.pid}, "start owner-exit control")
                    witness, port, session = watch(second)
                    self.assertEqual(close_owner(owner), 0)
                    self.assertTrue(witness.exited(EXIT_SECONDS), "dbgs survived its owner")
                    self.assertFalse(tcp_listening(port))
                    self.assertIsNone(server.process.poll(), "MCP exited during owner-exit control")
                    reaped = run_operation(server, STOP, {"session": session}, "clear expired session")
                    self.assertEqual(reaped["data"]["action"], "nothing_to_stop")
                    self.assertIs(reaped["data"]["staleRecord"], True)
                    with ib.open("rb") as stream:
                        self.assertEqual(hashlib.file_digest(stream, "sha256").hexdigest(), before)
                return finish

            acceptance.run_corpus(
                self, {**corpus, "scenarios": [bind_wire(scenario, owner.pid, occupied)]},
                plugin, scenario_driver=prepare, fixture_cleanup=cleanup,
            )


class NativeDbgsCaseLogic(unittest.TestCase):
    def test_declaration_binds_only_owned_values_and_keeps_the_wire_frozen(self):
        scenario, = select_profile(load_acceptance_corpus(), DBGS_PROFILE)["scenarios"]
        result = bind_wire(scenario, 71, 21177)
        self.assertEqual(result["wire"][1]["args"]["args"], {"ownerPid": 71, "port": 21177})
        self.assertIn("21177", result["wire"][1]["assertions"][1]["contains"])
        self.assertEqual(result["wire"][2]["assertions"][2]["eq"], 71)
        self.assertEqual(scenario["wire"][1]["args"]["args"]["ownerPid"], "$dbgsOwnerPid")
        with self.assertRaisesRegex(ValueError, "occupied TCP port"):
            bind_wire(scenario, 71, 0)

    def test_protocol_match_rejects_arbitrary_http_responses(self):
        witness = b'{"exception":"virtual-resource-system: Exception"}'
        self.assertTrue(rdbg_answer(400, witness))
        for status, body in ((200, witness), (404, witness), (406, witness),
                             (400, b"<html>bad request</html>"), (400, b"Exception")):
            with self.subTest(status=status, body=body):
                self.assertFalse(rdbg_answer(status, body))


if __name__ == "__main__":
    unittest.main()
