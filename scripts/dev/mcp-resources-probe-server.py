#!/usr/bin/env python3
"""#336 Resources portability fixture server.

Modes:
  FIXTURE_MODE=stdio (default) | http   (Streamable HTTP on FIXTURE_PORT, path /mcp)
Env:
  FIXTURE_LOG=<path.jsonl>   journal of every JSON-RPC frame (in/out) + notes
  FIXTURE_PORT=8907
  FIXTURE_AUTONOTIFY=1       add Report resource + send list_changed 1.5s after initialized

Publishes:
  fixed resources:
    unica://meta/contracts/Catalog  application/json   marker CATALOG-MARKER-7A41
    unica://meta/contracts/guide    text/markdown      marker GUIDE-MARKER-55C2
    unica://meta/contracts/big      text/markdown      512x BIGMARK-93F1 (~208 KB)
  resource template:
    unica://meta/contracts/{kind}   -> generated JSON, marker TEMPLATE-<kind>-MARKER-E1F0
  tool:
    unica_add_contract {kind}       adds fixed resource for kind + notifications/resources/list_changed
"""
import json, os, sys, time, threading, queue

LOG_PATH = os.environ.get("FIXTURE_LOG")
MODE = os.environ.get("FIXTURE_MODE", "stdio")
PORT = int(os.environ.get("FIXTURE_PORT", "8907"))
AUTONOTIFY = os.environ.get("FIXTURE_AUTONOTIFY") == "1"

log_lock = threading.Lock()

def log(direction, message, **extra):
    if not LOG_PATH:
        return
    rec = {"ts": round(time.time(), 3), "transport": MODE, "direction": direction, "message": message}
    rec.update(extra)
    with log_lock:
        with open(LOG_PATH, "a") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

BIG_LINES = []
for i in range(512):
    BIG_LINES.append(f"- item {i:04d} BIGMARK-93F1 " + ("контрактное поле, тип, обязательность, ограничение значения; " * 4))
BIG_TEXT = "# Большой справочный контракт\n\n" + "\n".join(BIG_LINES) + "\n"

state_lock = threading.Lock()
RESOURCES = {
    "unica://meta/contracts/Catalog": {
        "name": "contract-Catalog",
        "title": "Контракт вида Catalog",
        "mimeType": "application/json",
        "text": json.dumps({
            "kind": "Catalog", "marker": "CATALOG-MARKER-7A41",
            "requiredProperties": ["Name", "Synonym"],
            "note": "Порядок создания: сначала Name, затем Synonym; кодовое слово ответа: АКВАМАРИН"
        }, ensure_ascii=False, indent=2),
    },
    "unica://meta/contracts/guide": {
        "name": "contracts-guide",
        "title": "Гид по контрактам метаданных",
        "mimeType": "text/markdown",
        "text": "# Гид\n\nGUIDE-MARKER-55C2\n\nДля каждого вида метаданных есть ресурс unica://meta/contracts/<Kind>.\n",
    },
    "unica://meta/contracts/big": {
        "name": "contract-big",
        "title": "Большой контракт",
        "mimeType": "text/markdown",
        "text": BIG_TEXT,
    },
}

notify_queue = queue.Queue()  # http mode: queued server->client notifications

def resource_list_entry(uri, r):
    return {"uri": uri, "name": r["name"], "title": r.get("title"), "mimeType": r["mimeType"]}

def make_notification(method, params=None):
    n = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        n["params"] = params
    return n

def add_contract(kind):
    uri = f"unica://meta/contracts/{kind}"
    with state_lock:
        RESOURCES[uri] = {
            "name": f"contract-{kind}",
            "title": f"Контракт вида {kind}",
            "mimeType": "application/json",
            "text": json.dumps({"kind": kind, "marker": f"ADDED-{kind}-MARKER-D2E8"}, ensure_ascii=False),
        }
    return uri

def handle(msg, send_notification):
    """Returns response dict or None (for notifications)."""
    method = msg.get("method")
    msg_id = msg.get("id")
    params = msg.get("params") or {}

    def ok(result):
        return {"jsonrpc": "2.0", "id": msg_id, "result": result}

    def err(code, message):
        return {"jsonrpc": "2.0", "id": msg_id, "error": {"code": code, "message": message}}

    if method == "initialize":
        offered = params.get("protocolVersion") or "2025-11-25"
        return ok({
            "protocolVersion": offered,
            "capabilities": {"resources": {"subscribe": False, "listChanged": True}, "tools": {}},
            "serverInfo": {"name": "res336-fixture", "version": "1.0.0"},
            "instructions": "Справочные контракты метаданных опубликованы как MCP Resources (unica://meta/contracts/...).",
        })
    if method == "notifications/initialized":
        if AUTONOTIFY:
            def later():
                time.sleep(1.5)
                add_contract("Report")
                send_notification(make_notification("notifications/resources/list_changed"))
            threading.Thread(target=later, daemon=True).start()
        return None
    if method == "ping":
        return ok({})
    if method == "tools/list":
        return ok({"tools": [{
            "name": "unica_add_contract",
            "description": "Добавить справочный контракт вида метаданных как новый MCP-ресурс (после добавления сервер шлёт notifications/resources/list_changed).",
            "inputSchema": {"type": "object", "properties": {"kind": {"type": "string", "description": "Вид метаданных, например Report"}}, "required": ["kind"]},
        }]})
    if method == "tools/call":
        name = params.get("name")
        if name == "unica_add_contract":
            kind = (params.get("arguments") or {}).get("kind", "Report")
            uri = add_contract(kind)
            send_notification(make_notification("notifications/resources/list_changed"))
            return ok({"content": [{"type": "text", "text": f"added {uri}"}], "isError": False})
        return err(-32602, f"unknown tool {name}")
    if method == "resources/list":
        with state_lock:
            entries = [resource_list_entry(u, r) for u, r in sorted(RESOURCES.items())]
        return ok({"resources": entries})
    if method == "resources/read":
        uri = params.get("uri") or ""
        with state_lock:
            r = RESOURCES.get(uri)
        if r:
            return ok({"contents": [{"uri": uri, "mimeType": r["mimeType"], "text": r["text"]}]})
        if uri.startswith("unica://meta/contracts/"):
            kind = uri.rsplit("/", 1)[-1]
            text = json.dumps({"kind": kind, "marker": f"TEMPLATE-{kind}-MARKER-E1F0",
                               "requiredProperties": ["Name", "Synonym"]}, ensure_ascii=False, indent=2)
            return ok({"contents": [{"uri": uri, "mimeType": "application/json", "text": text}]})
        return err(-32002, f"resource not found: {uri}")
    if method == "resources/templates/list":
        return ok({"resourceTemplates": [{
            "uriTemplate": "unica://meta/contracts/{kind}",
            "name": "metadata-contract",
            "title": "Контракт вида метаданных",
            "mimeType": "application/json",
            "description": "Справочный контракт создания вида метаданных: обязательные свойства и порядок операций.",
        }]})
    if method == "prompts/list":
        return ok({"prompts": []})
    if method and msg_id is not None:
        return err(-32601, f"Method not found: {method}")
    return None

# ---------------- stdio ----------------

def run_stdio():
    out_lock = threading.Lock()

    def send(obj):
        log("out", obj)
        with out_lock:
            sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
            sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            log("note", {"unparsed": line[:200]})
            continue
        log("in", msg)
        resp = handle(msg, send)
        if resp is not None:
            send(resp)
    log("note", {"event": "eof"})

# ---------------- Streamable HTTP ----------------

def run_http():
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    session_id = "res336-" + hex(int(time.time()))[2:]
    sse_clients = []
    sse_lock = threading.Lock()

    def send_notification(n):
        log("out", n, channel="sse-queued")
        with sse_lock:
            for q in sse_clients:
                q.put(n)

    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):
            pass

        def _headers_subset(self):
            keep = ("mcp-session-id", "mcp-protocol-version", "accept", "content-type", "last-event-id")
            return {k: v for k, v in self.headers.items() if k.lower() in keep}

        def do_POST(self):
            if self.path.rstrip("/") != "/mcp":
                self.send_response(404); self.end_headers(); return
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            try:
                msg = json.loads(body)
            except json.JSONDecodeError:
                self.send_response(400); self.end_headers(); return
            msgs = msg if isinstance(msg, list) else [msg]
            responses = []
            for m in msgs:
                log("in", m, http=self._headers_subset())
                r = handle(m, send_notification)
                if r is not None:
                    responses.append(r)
            if not responses:
                self.send_response(202)
                self.send_header("Mcp-Session-Id", session_id)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            out = responses[0] if len(responses) == 1 else responses
            for r in responses:
                log("out", r, channel="http-post")
            data = json.dumps(out, ensure_ascii=False).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Mcp-Session-Id", session_id)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path.rstrip("/") != "/mcp":
                self.send_response(404); self.end_headers(); return
            log("note", {"event": "sse-open", "http": self._headers_subset()})
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Mcp-Session-Id", session_id)
            self.end_headers()
            q = queue.Queue()
            with sse_lock:
                sse_clients.append(q)
            try:
                while True:
                    try:
                        n = q.get(timeout=15)
                        payload = f"event: message\ndata: {json.dumps(n, ensure_ascii=False)}\n\n"
                        self.wfile.write(payload.encode()); self.wfile.flush()
                        log("out", n, channel="sse-delivered")
                    except queue.Empty:
                        self.wfile.write(b": keepalive\n\n"); self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError, OSError):
                log("note", {"event": "sse-closed"})
            finally:
                with sse_lock:
                    if q in sse_clients:
                        sse_clients.remove(q)

        def do_DELETE(self):
            log("note", {"event": "session-delete", "http": self._headers_subset()})
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()

    srv = ThreadingHTTPServer(("127.0.0.1", PORT), H)
    log("note", {"event": "http-listening", "port": PORT})
    srv.serve_forever()

if __name__ == "__main__":
    if MODE == "http":
        run_http()
    else:
        run_stdio()
