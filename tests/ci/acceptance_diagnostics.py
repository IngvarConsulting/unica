"""Real pinned analyzer drivers for diagnostic acceptance, with private Git state."""
from __future__ import annotations

import json
import os
import queue
import subprocess
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


def prepare_diagnostic_case(server, scenario):
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
