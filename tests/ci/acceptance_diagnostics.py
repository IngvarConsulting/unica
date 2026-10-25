"""Real pinned analyzer drivers for diagnostic acceptance, with private Git state."""
from __future__ import annotations

from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
import platform
import queue
import subprocess
import tempfile
import threading
import time
from pathlib import Path

CONFIG_NAMES = ("bsl-analyzer.toml", ".bsl-analyzer.json", ".bsl-language-server.json")
AUTHOR = "vendor@example.invalid"
MODULE = "CommonModules/Пример/Ext/Module.bsl"


def initialize_vendor_source(source: Path) -> None:
    if (source / ".git").exists():
        return
    subprocess.run(["git", "init", "--quiet", str(source)], check=True, capture_output=True)
    subprocess.run(["git", "-C", str(source), "config", "core.autocrlf", "false"], check=True)
    subprocess.run(["git", "-C", str(source), "add", "."], check=True)
    subprocess.run(["git", "-C", str(source), "-c", "commit.gpgsign=false",
                    "-c", "user.name=Vendor", "-c", f"user.email={AUTHOR}",
                    "commit", "--quiet", "-m", "synthetic vendor baseline"], check=True,
                   capture_output=True)


def configure_case(source: Path, case: str) -> None:
    for name in CONFIG_NAMES:
        (source / name).unlink(missing_ok=True)
    if case == "plain":
        return
    if case == "authors-toml":
        (source / CONFIG_NAMES[0]).write_text(f'[analysis]\nignored_authors=["{AUTHOR}"]\n', encoding="utf-8")
    elif case == "authors-json":
        (source / CONFIG_NAMES[1]).write_text(json.dumps({"analysis": {"ignoredAuthors": [AUTHOR]}}), encoding="utf-8")
    elif case == "diff-base":
        (source / CONFIG_NAMES[0]).write_text('[analysis]\ndiff_base="HEAD"\n', encoding="utf-8")
    elif case == "precedence":
        (source / CONFIG_NAMES[0]).write_text("[analysis]\n", encoding="utf-8")
        (source / CONFIG_NAMES[1]).write_text(json.dumps({"analysis": {"ignoredAuthors": [AUTHOR]}}), encoding="utf-8")
    else:
        raise ValueError(f"unsupported diagnostic case {case!r}")


def prepare_diagnostic_case(server, scenario, *, plugin=None, fixtures=None):
    if scenario.get("driver") == "bsl-analyzer-invalidation":
        if plugin is None or fixtures is None:
            raise ValueError("helper invalidation requires the staged analyzer and owned fixtures")
        assert_held = fixtures.enter_context(legacy_broker_for_invalidation(plugin, server.workspace))
        return lambda verification: assert_held()
    if scenario.get("driver") == "bsl-analyzer-form-diagnostics":
        from tests.ci.form_runtime_diagnostics import prepare_case
        return prepare_case(server, scenario)
    if scenario.get("driver") != "bsl-analyzer-diagnostics":
        return None
    source = server.workspace / "src"
    initialize_vendor_source(source)
    configure_case(source, scenario["evaluation"])
    return None


class ResidentSession:
    """Actual analyzer MCP; bounded observation time and owned process cleanup."""
    def __init__(self, binary, source, cache):
        self.records = []
        self.cache = cache
        self.next_id = 0
        self.stderr_file = open(cache.with_suffix(".stderr.log"), "wb")
        try:
            environment = dict(os.environ)
            environment["BSL_MCP_IDLE_TTL_SECS"]="1"
            environment["BSL_MCP_ORPHAN_GRACE_SECS"]="1"
            self.process = subprocess.Popen([
                str(binary), "mcp", "serve", "--profile", "workspace",
                "--source-dir", str(source), "--cache-dir", str(cache), "--mode", "stdio",
            ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr_file, cwd=source, env=environment)
        except BaseException:
            self.stderr_file.close()
            raise
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()

    def _pump(self):
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def send(self, payload):
        self.process.stdin.write((json.dumps(payload) + "\n").encode())
        self.process.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("resident analyzer did not answer")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError("resident analyzer did not answer") from error
            if line is None:
                raise RuntimeError("resident analyzer exited before its answer")
            response = json.loads(line)
            if response.get("id") == self.next_id:
                self.records.append({"method": method, "params": params, "response": response})
                if "error" in response:
                    raise RuntimeError(response["error"])
                return response["result"]

    def diagnostics(self, arguments):
        result = self.request("tools/call", {"name": "diagnostics", "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(result)
        return result.get("structuredContent") or json.loads(next(
            item["text"] for item in result["content"] if item["type"] == "text"))

    def findings(self, module):
        self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                    "clientInfo": {"name": "unica-acceptance", "version": "0"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        self.request("tools/list", {})
        deadline = time.monotonic() + 90
        while True:
            status = self.diagnostics({"action": "status"})
            state = status.get("result", status)
            if state.get("state") == "failed" or state.get("reload") == "failed":
                raise RuntimeError(state)
            if state.get("state") == "ready" and state.get("reload") not in {"running", "failed"}:
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("resident analysis never became ready")
            time.sleep(.2)  # Observe the upstream building state, not retry a failed run.
        return self.diagnostics({"action": "file", "path": str(module), "min_severity": "hint", "max_findings": 5000})

    def close(self):
        try:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
            self.reader.join(timeout=5)
            self.process.stdout.close()
            if self.reader.is_alive():
                raise TimeoutError("resident stdout reader did not stop")
            # The distributed analyzer launches a warm background daemon even
            # for this stdio connection. Observe its own lease PID; a finished
            # stdio client alone does not release the cache directory.
            lease=self.cache / "writer.lease"
            if lease.exists():
                pid=json.loads(lease.read_text(encoding="utf-8"))["pid"]
                deadline=time.monotonic()+15
                while True:
                    if os.name=="nt":
                        state=subprocess.run(["tasklist","/FI",f"PID eq {pid}","/FO","CSV","/NH"],
                                             capture_output=True,text=True,check=True).stdout
                        alive=f'"{pid}"' in state
                    else:
                        state=subprocess.run(["ps","-p",str(pid),"-o","stat="],
                                             capture_output=True,text=True).stdout.strip()
                        alive=bool(state) and not state.startswith("Z")
                    if not alive:
                        break
                    if time.monotonic()>=deadline:
                        raise TimeoutError("owned resident daemon did not idle out before cache cleanup")
                    time.sleep(.1)
        finally:
            self.stderr_file.close()


def legacy_analyzer_cache_key(workspace: str, source: str) -> str:
    digest = hashlib.sha256(b"unica-provider-state-v1\0")
    digest.update(workspace.encode("utf-8"))
    digest.update(b"\0")
    digest.update(source.encode("utf-8"))
    return f"source-{digest.hexdigest()}"


def legacy_analyzer_cache(workspace: Path, source: Path) -> Path:
    """The cache layout used by the core that allowed an implicit broker."""
    key = legacy_analyzer_cache_key(str(workspace.resolve()), str(source.resolve()))
    return workspace / ".build/unica/providers/bsl-analyzer" / key


def stop_owned_process(process) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


class LegacyBrokerProxy(ResidentSession):
    """A public broker client; its fixture separately owns the daemon process."""
    def __init__(self, binary, source, cache, runtime, environment):
        self.records = []
        self.cache = cache
        self.next_id = 0
        self._closed = False
        self.stderr_path = cache.parent / "legacy-proxy.stderr.log"
        self.stderr_file = open(self.stderr_path, "wb")
        try:
            self.process = subprocess.Popen([
                str(binary), "mcp", "serve", "--profile", "workspace",
                "--source-dir", str(source), "--cache-dir", str(cache), "--mode", "broker",
            ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr_file,
                cwd=runtime, env=environment)
        except BaseException:
            self.stderr_file.close()
            raise
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()

    def close(self):
        if self._closed:
            return
        try:
            try:
                self.process.stdin.close()
            except OSError:
                pass
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                stop_owned_process(self.process)
            self.reader.join(timeout=5)
            self.process.stdout.close()
            if self.reader.is_alive():
                raise TimeoutError("legacy proxy reader did not stop after EOF")
        finally:
            self.stderr_file.close()
            self._closed = True


def assert_owned_legacy_broker(backend, cache: Path, runtime: Path):
    if backend.poll() is not None:
        raise AssertionError("owned legacy backend exited during the upgrade scenario")
    record = json.loads((cache / "writer.lease").read_text(encoding="utf-8"))
    if record.get("pid") != backend.pid:
        raise AssertionError("the legacy lease belongs to another process")
    if os.name != "nt":
        sockets = [path for path in (runtime / "bsl-mcp").glob("*.sock") if path.is_socket()]
        if not sockets:
            raise FileNotFoundError("the owned legacy endpoint is not ready")
        if len(sockets) != 1:
            raise AssertionError("the owned legacy endpoint is ambiguous")


@contextmanager
def legacy_broker_for_invalidation(plugin: Path, workspace: Path):
    from tests.ci.acceptance_profiles import host_target

    lock = json.loads((plugin / "third-party/tools.lock.json").read_text())
    target = host_target(lock, platform.system(), platform.machine())
    tool = next(tool for tool in lock["tools"] if tool["name"] == "bsl-analyzer")
    binary = plugin / "bin" / target / (tool["binaryName"] + lock["targets"][target]["exe"])
    source = workspace / "src"
    cache = legacy_analyzer_cache(workspace, source)
    cache.mkdir(parents=True, exist_ok=True)
    with ExitStack() as resources:
        # dirs::runtime_dir() is None on macOS; the pinned backend then uses TMPDIR.
        raw = resources.enter_context(tempfile.TemporaryDirectory(
            prefix="u978-", dir="/tmp" if platform.system() == "Darwin" else None))
        runtime = Path(raw)
        runtime.chmod(0o700)
        environment = dict(os.environ)
        environment.update(XDG_RUNTIME_DIR=str(runtime), TMPDIR=str(runtime))
        for key in ["BSL_MCP_BROKER", "BSL_MCP_IDLE_TTL_SECS", "BSL_MCP_ORPHAN_GRACE_SECS",
                    "EMBEDDING_URL", "EMBEDDING_API_KEY"]:
            environment.pop(key, None)
        stderr = resources.enter_context(open(runtime / "legacy-backend.stderr.log", "wb"))
        backend = subprocess.Popen([
            str(binary), "mcp", "serve", "--profile", "workspace", "--source-dir", str(source),
            "--cache-dir", str(cache), "--mode", "daemon",
        ], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=stderr,
            cwd=runtime, env=environment)
        resources.callback(stop_owned_process, backend)
        deadline = time.monotonic() + 30
        while True:
            try:
                assert_owned_legacy_broker(backend, cache, runtime)
                break
            except (FileNotFoundError, json.JSONDecodeError):
                pass
            if time.monotonic() >= deadline:
                raise TimeoutError("owned legacy backend did not publish its endpoint and lease")
            time.sleep(.01)
        proxy = LegacyBrokerProxy(binary, source, cache, runtime, environment)
        resources.callback(proxy.close)
        proxy.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                    "clientInfo": {"name": "unica-upgrade-acceptance", "version": "0"}})
        proxy.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        deadline = time.monotonic() + 30
        while True:
            result = proxy.request("tools/call", {"name": "search", "arguments": {
                "action": "search_code", "query": "AcceptanceExistingSymbol1306", "limit": 20}})
            text = "\n".join(item.get("text", "") for item in result.get("content", []))
            if ":: AcceptanceExistingSymbol1306 (procedure)" in text:
                break
            if "not_ready" not in json.dumps(result):
                raise AssertionError("legacy broker did not return its initial symbol or pending state")
            if time.monotonic() >= deadline:
                raise TimeoutError("legacy broker search did not become ready")
            time.sleep(.1)
        assert_owned_legacy_broker(backend, cache, runtime)
        proxy.close()
        assert_owned_legacy_broker(backend, cache, runtime)
        yield lambda: assert_owned_legacy_broker(backend, cache, runtime)


def resident_filter_evidence(binary: Path, workspace: Path, root: Path):
    source = workspace / "src"
    initialize_vendor_source(source)
    evidence = {}
    for name, case in [("plain", "plain"), ("authors", "authors-toml"), ("scope", "diff-base"), ("both", "plain")]:
        configure_case(source, case)
        if name == "both":
            baseline_config = '[diagnostics.baseline]\npath="baseline.json"\n'
            (source / CONFIG_NAMES[0]).write_text(baseline_config, encoding="utf-8")
            created = subprocess.run([str(binary), "diagnostics", "baseline", "create",
                                      "--source-dir", str(source), "--format", "json"],
                                     capture_output=True, text=True, timeout=90, check=True)
            (root / "baseline-create.stdout").write_text(created.stdout, encoding="utf-8")
            baseline_path = source / "baseline.json"
            baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
            baseline["diagnostics"] = [item for item in baseline["diagnostics"]
                                       if item["path"] == MODULE and item["code"] == "UnusedLocalVariable"]
            if len(baseline["diagnostics"]) != 1:
                raise AssertionError("pinned CLI did not create the unused-variable baseline record")
            baseline_path.write_text(json.dumps(baseline, ensure_ascii=False), encoding="utf-8")
            (source / CONFIG_NAMES[0]).write_text(baseline_config + f'\n[analysis]\nignored_authors=["{AUTHOR}"]\n', encoding="utf-8")
        session = ResidentSession(binary, source, root / (name + "-cache"))
        try:
            evidence[name] = session.findings(source / MODULE)
        finally:
            session.close()
            (root / (name + "-protocol.json")).write_text(json.dumps(session.records, ensure_ascii=False, indent=2), encoding="utf-8")
    return evidence
