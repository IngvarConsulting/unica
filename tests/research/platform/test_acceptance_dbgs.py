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
EXIT_SECONDS = 30


def bind_wire(scenario, owner_pid, supported):
    if type(owner_pid) is not int or not 1 <= owner_pid <= 0xFFFFFFFF:
        raise ValueError("expected a live owner PID")
    wire = deepcopy(scenario["wire"])
    if not supported:
        # On the old package the driver checks both refusals before creating any IB.
        return {**scenario, "wire": wire[:1]}
    for step in wire[1:]:
        step["args"]["args"]["ownerPid"] = owner_pid
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


def owned_endpoint(data):
    release = data.get("release") or {}
    session = release.get("args", {}).get("session")
    endpoint = urlsplit(data.get("url", ""))
    if not (
        data.get("op") == START and data.get("contractVersion") == 2
        and data.get("action") == "prepared" and data.get("owner") == "runner"
        and data.get("scope") == "clientAndServer" and data.get("infoBaseAlias") == "DefAlias"
        and endpoint.scheme == "http" and endpoint.port and endpoint.hostname
        and (endpoint.hostname == "localhost" or ipaddress.ip_address(endpoint.hostname).is_loopback)
        and isinstance(session, str) and re.fullmatch(r"[0-9a-f]{32}", session)
        and release == {"op": STOP, "args": {"session": session}}
    ):
        raise AssertionError("expected a v2 file debugger endpoint with an individual stop lease")
    return endpoint, session


def listening_pid(endpoint):
    # Obtain the identity independently of the MCP receipt, which grants a lease.
    response = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
         f"@(Get-NetTCPConnection -State Listen -LocalPort {endpoint.port} -ErrorAction Stop | "
         "Select-Object -Property LocalAddress,OwningProcess) | ConvertTo-Json -Compress"],
        check=True, capture_output=True, text=True, timeout=20,
    )
    addresses = {endpoint.hostname} if endpoint.hostname != "localhost" else {"127.0.0.1", "::1"}
    listeners = json.loads(response.stdout)
    if isinstance(listeners, dict):
        listeners = [listeners]
    processes = {row["OwningProcess"] for row in listeners if row["LocalAddress"] in addresses}
    if len(processes) != 1:
        raise AssertionError("expected one process listening on the prepared loopback endpoint")
    return processes.pop()


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


def call_debug(server, op, args, dry_run=False):
    return tool_result(completed_delivery_call(
        server, "unica.run", {"op": op, "args": args, "dryRun": dry_run}, op, retry_pending_index=False,
    ))


def runner_has_automatic_start(binary):
    # The old CLI permits a manual port. Examine the real staged provider,
    # independently of what the MCP server advertises or what its tag is called.
    help_text = subprocess.run(
        [str(binary), "debug-server", "start", "--help"], check=True, capture_output=True,
        text=True, encoding="utf-8", timeout=10,
    ).stdout
    return "--owner-pid" in help_text and "--port" not in help_text


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
            supported = runner_has_automatic_start(plugin / manifest["tools"][0]["binaryPath"])
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

            def watch(data):
                endpoint, session = owned_endpoint(data)
                pid = listening_pid(endpoint)
                self.assertNotEqual(pid, owner.pid)
                witness = ProcessWitness(pid)
                witnesses.append(witness)
                self.assertEqual(witness.image.name.lower(), "dbgs.exe")
                self.assertIn(build, witness.image.parts, "DBGS image must belong to the selected build")
                self.assertFalse(witness.exited())
                self.assertTrue(rdbg_ready(endpoint.port, endpoint.hostname))
                return endpoint, session, witness

            def prepare(server, _scenario):
                args = {"ownerPid": owner.pid}
                connection = f'File="{(server.workspace / "ib").as_posix()}"'
                local = server.workspace / "v8project.local.yaml"
                if supported:
                    configure_workspace(server.workspace, build)
                    local.write_text(
                        "infobases:\n  origin:\n    connection: " + json.dumps(connection) + "\n",
                        encoding="utf-8",
                    )
                    run_operation(server, "infobase.create", {}, "prepare dedicated file infobase")
                    ib = server.workspace / "ib/1Cv8.1CD"
                    with ib.open("rb") as stream:
                        before = hashlib.file_digest(stream, "sha256").hexdigest()

                def finish(verification):
                    operations = {row["op"]: row for row in verification[0]["result"]["data"]["operations"]}
                    self.assertNotIn("debug-server.prepare", operations)
                    if not supported:
                        for operation, inputs in [(START, args), (STOP, {"session": "0" * 32})]:
                            self.assertNotIn(operation, operations)
                            for preview in (True, False):
                                refused = call_debug(server, operation, inputs, preview)
                                self.assertFalse(refused["ok"], refused)
                                self.assertEqual(refused["diagnostics"][0]["code"], "unsupported_operation")
                                for field in ("url", "release"):
                                    self.assertNotIn(field, refused.get("data") or {})
                        return
                    for operation in (START, STOP):
                        self.assertTrue(operations[operation]["terminal"])
                        self.assertEqual(operations[operation]["support"]["adapter"], f"v8-runner/{staged_version}")
                    row = operations[START]
                    self.assertEqual(row["contractVersion"], 2)
                    self.assertEqual(set(row["argsSchema"]["properties"]), {"ownerPid"})
                    self.assertEqual(verification[1]["result"]["data"], {
                        "op": START, "contractVersion": 2, "action": "planned",
                    })
                    first = verification[2]["result"]["data"]
                    endpoint, first_lease, witness = watch(first)

                    def release(session, action):
                        response = run_operation(server, STOP, {"session": session}, "release the debugger lease")["data"]
                        self.assertEqual(response["session"], session)
                        self.assertEqual(response["action"], action, response)
                        return response

                    second = run_operation(server, START, args, "acquire a second file debugger lease")["data"]
                    second_endpoint, second_lease = owned_endpoint(second)
                    self.assertEqual(endpoint, second_endpoint)
                    self.assertNotEqual(first_lease, second_lease)
                    self.assertFalse(witness.exited())

                    # The owner can explicitly choose this running server for the same file IB.
                    profile = local.read_bytes()
                    def choose_url(url):
                        marker = b"  origin:"
                        self.assertEqual(profile.count(marker), 1)
                        local.write_bytes(profile.replace(marker, marker + f"\n    debug:\n      url: {json.dumps(url)}".encode(), 1))
                    try:
                        choose_url(first["url"])
                        borrowed = run_operation(server, START, args, "use the explicitly selected dbgs")["data"]
                        self.assertEqual(borrowed, {
                            "op": START, "contractVersion": 2, "action": "prepared", "owner": "external",
                            "scope": "clientAndServer", "url": first["url"], "infoBaseAlias": "DefAlias", "release": None,
                        })
                        # Bound without listen: a stable connection refusal at this exact local address.
                        with socket.socket() as unavailable:
                            unavailable.bind(("127.0.0.1", 0))
                            choose_url(f"http://127.0.0.1:{unavailable.getsockname()[1]}")
                            refusal = call_debug(server, START, args)
                        self.assertFalse(refusal["ok"], refusal)
                        self.assertEqual(refusal["data"]["runnerCode"], "debug_server_external_unavailable")
                        self.assertNotIn("release", refusal["data"])
                        self.assertNotIn("url", refusal["data"])
                    finally:
                        local.write_bytes(profile)
                    self.assertFalse(witness.exited(), "the configured external server was replaced")
                    self.assertIs(release(first_lease, "released")["knownSession"], True)
                    self.assertFalse(witness.exited())
                    self.assertTrue(rdbg_ready(endpoint.port, endpoint.hostname))
                    self.assertIs(release(second_lease, "stopped")["knownSession"], True)
                    self.assertTrue(witness.exited(EXIT_SECONDS))
                    self.assertFalse(tcp_listening(endpoint.port, endpoint.hostname))
                    self.assertIs(release(second_lease, "nothing_to_stop")["knownSession"], False)
                    self.assertIsNone(owner.poll())

                    owned = run_operation(server, START, args, "start owner-exit control")["data"]
                    endpoint, session, witness = watch(owned)
                    self.assertEqual(close_owner(owner), 0)
                    self.assertTrue(witness.exited(EXIT_SECONDS), "dbgs survived its owner")
                    self.assertFalse(tcp_listening(endpoint.port, endpoint.hostname))
                    self.assertIsNone(server.process.poll(), "MCP exited during owner-exit control")
                    release(session, "nothing_to_stop")
                    with ib.open("rb") as stream:
                        self.assertEqual(hashlib.file_digest(stream, "sha256").hexdigest(), before)
                return finish

            acceptance.run_corpus(
                self, {**corpus, "scenarios": [bind_wire(scenario, owner.pid, supported)]},
                plugin, scenario_driver=prepare, fixture_cleanup=cleanup,
            )


class NativeDbgsCaseLogic(unittest.TestCase):
    def test_declaration_binds_only_owned_values_and_keeps_the_wire_frozen(self):
        scenario, = select_profile(load_acceptance_corpus(), DBGS_PROFILE)["scenarios"]
        result = bind_wire(scenario, 71, True)
        for step in result["wire"][1:]:
            self.assertEqual(step["args"]["args"], {"ownerPid": 71})
        self.assertEqual([s["args"]["dryRun"] for s in result["wire"][1:]], [True, False])
        self.assertEqual(scenario["wire"][1]["args"]["args"]["ownerPid"], "$dbgsOwnerPid")
        self.assertEqual(bind_wire(scenario, 71, False)["wire"], scenario["wire"][:1])
        with self.assertRaisesRegex(ValueError, "owner PID"):
            bind_wire(scenario, 0, True)

    def test_file_endpoint_requires_an_individual_lease_and_the_v2_scope(self):
        token = "f" * 32
        receipt = {
            "op": START, "action": "prepared", "contractVersion": 2, "owner": "runner",
            "scope": "clientAndServer", "infoBaseAlias": "DefAlias", "url": "http://127.0.0.1:1550",
            "release": {"op": STOP, "args": {"session": token}},
        }
        endpoint, session = owned_endpoint(receipt)
        self.assertEqual((endpoint.port, session), (1550, token))
        for field, value in [
            ("owner", "external"), ("contractVersion", 1), ("scope", "client"), ("release", None),
            ("release", {"op": STOP, "args": {"session": "not-a-token"}}),
            ("url", "http://192.0.2.1:1550"),
        ]:
            with self.subTest(field=field), self.assertRaises(AssertionError):
                owned_endpoint({**receipt, field: value})

    def test_protocol_match_rejects_arbitrary_http_responses(self):
        witness = b'{"exception":"virtual-resource-system: Exception"}'
        self.assertTrue(rdbg_answer(400, witness))
        for status, body in ((200, witness), (404, witness), (406, witness),
                             (400, b"<html>bad request</html>"), (400, b"Exception")):
            with self.subTest(status=status, body=body):
                self.assertFalse(rdbg_answer(status, body))


if __name__ == "__main__":
    unittest.main()
