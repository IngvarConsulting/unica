"""A real core advertises its complete wire surface without delivering engines."""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import unittest

from scripts.ci.prepared_binary import ensure_binary
from tests.ci.acceptance_profiles import host_target, isolated_environment
from tests.ci.test_probe_unica_wire import load_module
from tests.ci.test_tool_surface_ledger import NATIVE_V13, TASK_COMPATIBILITY

REPO_ROOT = Path(__file__).resolve().parents[2]
STARTUP_FUSE_SECONDS = 30


def sha256(data):
    return hashlib.sha256(data).hexdigest()


class ProxyTrap:
    """Count every accepted connection, reply locally, and join the observer."""
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(.1)
        self.address = self.listener.getsockname()
        self.stopping = threading.Event()
        self.lock = threading.Lock()
        self.connections = 0
        self.requests = []
        self.failure = None
        self.active = None
        self.worker = threading.Thread(target=self._serve, name="engine-free-proxy-observer")
        self.worker.start()

    @property
    def url(self):
        return f"http://{self.address[0]}:{self.address[1]}"

    def _serve(self):
        try:
            while not self.stopping.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with connection:
                    with self.lock:
                        self.connections += 1
                        self.active = connection
                    connection.settimeout(2)
                    request = bytearray()
                    while b"\r\n\r\n" not in request:
                        block = connection.recv(1024)
                        if not block:
                            break
                        request.extend(block)
                        if len(request) > 65536:
                            raise AssertionError("unexpected unbounded proxy request")
                    with self.lock:
                        self.requests.append(bytes(request).split(b"\r\n", 1)[0])
                    connection.sendall(b"HTTP/1.1 502 Controlled local refusal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    with self.lock:
                        self.active = None
        except BaseException as error:
            self.failure = error

    def close(self):
        self.stopping.set()
        with self.lock:
            active = self.active
        if active is not None:
            active.shutdown(socket.SHUT_RDWR)
        self.worker.join(timeout=5)
        self.listener.close()
        if self.worker.is_alive():
            raise AssertionError("proxy observer did not stop")
        if self.failure is not None:
            raise AssertionError("proxy observer failed") from self.failure

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def write_missing_engine_package(plugin, binary):
    """Declare approved release assets, with no engine binaries materialized."""
    lock = json.loads((REPO_ROOT / "plugins/unica/third-party/tools.lock.json").read_text())
    package = json.loads((REPO_ROOT / "plugins/unica/.codex-plugin/plugin.json").read_text())
    version = package["version"]
    target = host_target(lock, platform.system(), platform.machine())
    artifacts = {}
    descriptors = []
    with binary.open("rb") as stream:
        core_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    core_targets = {}
    for name, config in lock["targets"].items():
        executable = f'bin/{name}/unica{config["exe"]}'
        asset = f"unica-runtime-{name}.tar.gz"
        core_targets[name] = {
            "asset": {"name": asset,
                      "url": f"https://github.com/IngvarConsulting/unica/releases/download/v{version}/{asset}",
                      "mediaType": "application/gzip", "sha256": core_hash},
            "files": [{"path": executable, "sha256": core_hash, "executable": True}],
            "entrypoint": executable,
        }
    artifacts["unica"] = {"version": version, "role": "core", "targets": core_targets}
    for tool in lock["tools"]:
        if tool["name"] == "unica":
            continue
        artifact = "rlm-tools-bsl" if tool["name"].startswith("rlm-bsl-") else tool["name"]
        artifact_row = artifacts.setdefault(artifact, {"version": tool["version"], "role": "engine", "targets": {}})
        binaries = {}
        for name, config in lock["targets"].items():
            asset = tool["assets"][name]
            delivered = asset.get("archiveBinary") or f'{tool["binaryName"]}{config["exe"]}'
            file_hash = asset["sha256"] if "archiveBinary" not in asset else sha256(f"absent:{tool['name']}:{name}".encode())
            binary_path = f'bin/{name}/{tool["binaryName"]}{config["exe"]}'
            binaries[name] = {"targetTriple": config["targetTriple"], "binaryPath": binary_path,
                              "deliveredPath": delivered, "sha256": file_hash}
            suffix = asset["assetName"]
            media_type = ("application/gzip" if suffix.endswith(".tar.gz") else
                          "application/zip" if suffix.endswith(".zip") else
                          "application/octet-stream")
            row = artifact_row["targets"].setdefault(name, {
                "asset": {"name": suffix,
                          "url": f"{tool['assetRepository'].rstrip('/')}/releases/download/{tool['assetTag']}/{suffix}",
                          "mediaType": media_type, "sha256": asset["sha256"]},
                "files": [],
            })
            row["files"].append({"path": delivered, "sha256": file_hash, "executable": True})
        descriptors.append({"name": tool["name"], "version": tool["version"],
                            "artifact": artifact, "binaries": binaries})
    runtime = {"schemaVersion": 2, "pluginVersion": version, "development": False,
               "source": {"repository": "https://github.com/IngvarConsulting/unica",
                          "commit": "0123456789abcdef0123456789abcdef01234567"},
               "release": {"repository": "https://github.com/IngvarConsulting/unica", "tag": f"v{version}"},
               "artifacts": artifacts}
    (plugin / "third-party").mkdir(parents=True)
    (plugin / ".codex-plugin").mkdir()
    (plugin / "runtime-manifest.json").write_text(json.dumps(runtime), encoding="utf-8")
    (plugin / "third-party/tools.lock.json").write_text(json.dumps(lock), encoding="utf-8")
    (plugin / ".codex-plugin/plugin.json").write_text(json.dumps(package), encoding="utf-8")
    (plugin / "third-party/manifest.json").write_text(json.dumps({
        "schemaVersion": 2, "targetTriple": lock["targets"][target]["targetTriple"],
        "artifactAssets": {key: value["targets"][target]["asset"] for key, value in artifacts.items() if key != "unica"},
        "tools": descriptors,
    }), encoding="utf-8")
    return target


@contextmanager
def owned_daemon(binary, state, workspace, environment):
    identity = subprocess.check_output([str(binary), "--print-core-identity"], env=environment,
                                       cwd=workspace, text=True, timeout=STARTUP_FUSE_SECONDS).strip()
    stderr_path = state / "owned-daemon-stderr.log"
    with stderr_path.open("wb") as stderr:
        process = subprocess.Popen([str(binary), "--daemon", "--state-root", str(state),
                                    "--core-identity", identity, "--idle-grace-ms", "30000"],
                                   cwd=workspace, env=environment, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=stderr)
        try:
            deadline = time.monotonic() + STARTUP_FUSE_SECONDS
            checkpoint = threading.Event()
            while True:
                endpoints = list(state.glob("daemon-*/endpoint.json"))
                if len(endpoints) == 1:
                    endpoint = json.loads(endpoints[0].read_text(encoding="utf-8"))
                    if endpoint.get("pid") == process.pid:
                        break
                if process.poll() is not None:
                    raise AssertionError(f"owned daemon exited: {stderr_path.read_text(errors='replace')}")
                if time.monotonic() >= deadline:
                    raise AssertionError("owned daemon never published its endpoint")
                checkpoint.wait(.01)
            yield process, endpoints[0], endpoint
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
                raise AssertionError("owned daemon required forced cleanup")


class EngineFreeHandshakeTests(unittest.TestCase):
    """rule: INV.WIRE.ENGINE-FREE-HANDSHAKE
    rule: INV.PKG.CORE-FIRST-ACQUISITION
    """
    @classmethod
    def setUpClass(cls):
        cls.binary = ensure_binary(REPO_ROOT)

    def _exercise(self, protocol, expected, profile):
        # A separate real TCP control proves that this observer sees CONNECT.
        with ProxyTrap() as control:
            with socket.create_connection(control.address, timeout=5) as client:
                client.sendall(b"CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n")
                self.assertTrue(client.recv(1024).startswith(b"HTTP/1.1 502"))
        self.assertEqual(control.connections, 1)
        self.assertEqual(control.requests, [b"CONNECT github.com:443 HTTP/1.1"])
        with tempfile.TemporaryDirectory(prefix="unica-engine-free-handshake-") as raw:
            root = Path(raw).resolve()
            workspace = root / "workspace"
            workspace.mkdir()
            executable = root / self.binary.name
            shutil.copy2(self.binary, executable)
            plugin = root / "plugin"
            target = write_missing_engine_package(plugin, executable)
            before_plugin = {path.relative_to(plugin).as_posix(): path.read_bytes()
                             for path in plugin.rglob("*") if path.is_file()}
            state = root / "state"
            state.mkdir()
            cache = root / "empty-artifact-cache"
            cache.mkdir()
            with ProxyTrap() as trap:
                environment = isolated_environment(plugin, state)
                environment.update({"UNICA_PROVIDER_STATE_DIR": str(state),
                                    "UNICA_RUNTIME_MANIFEST": str(plugin / "runtime-manifest.json"),
                                    "UNICA_ARTIFACT_CACHE": str(cache),
                                    "UNICA_CACHE_DIR": str(root / "workspace-cache"),
                                    "HTTPS_PROXY": trap.url, "HTTP_PROXY": trap.url,
                                    "ALL_PROXY": trap.url, "NO_PROXY": ""})
                for key in ["https_proxy", "http_proxy", "all_proxy", "no_proxy"]:
                    environment.pop(key, None)
                with owned_daemon(executable, state, workspace, environment) as (daemon, endpoint_path, initial):
                    probe = load_module()
                    result = probe.WireProbe([str(executable)], protocol_version=protocol,
                                             tasks_capability="on" if profile == "native" else "off",
                                             profile=profile, target=target,
                                             timeout_seconds=STARTUP_FUSE_SECONDS,
                                             environment=environment, cwd=workspace).run()
                    self.assertEqual(result["toolNames"], sorted(expected), result)
                    self.assertEqual(result["toolCount"], len(expected), result)
                    self.assertIsNone(daemon.poll(), "the pre-owned daemon must serve the whole probe")
                    after_paths = list(state.glob("daemon-*/endpoint.json"))
                    self.assertEqual(after_paths, [endpoint_path], "frontend spawned another daemon")
                    final = json.loads(endpoint_path.read_text(encoding="utf-8"))
                    self.assertEqual((final["pid"], final["instanceId"]),
                                     (initial["pid"], initial["instanceId"]))
                self.assertIsNotNone(daemon.returncode, "counter is checked only after daemon exit")
            self.assertEqual(trap.connections, 0, trap.requests)
            self.assertEqual(trap.requests, [])
            self.assertEqual(list(cache.iterdir()), [], "handshake left delivery state or an engine")
            self.assertEqual({path.relative_to(plugin).as_posix(): path.read_bytes()
                              for path in plugin.rglob("*") if path.is_file()}, before_plugin)
            self.assertEqual(list(workspace.iterdir()), [], "handshake wrote workspace files")

    def test_compatibility_initialize_and_all_tools_work_without_engines(self):
        self._exercise("2025-11-25", NATIVE_V13 + TASK_COMPATIBILITY, "compatibility")

    def test_native_tools_list_works_without_engines(self):
        self._exercise("2026-07-28", NATIVE_V13, "native")
