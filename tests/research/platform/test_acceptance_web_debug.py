"""Exercise web preparation and ordinary launch preview through MCP and the packaged runner."""
from __future__ import annotations

import json
import os
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

from scripts.ci.temp_tree import temp_tree
from tests.ci.acceptance_controls import load_acceptance_corpus
from tests.ci.acceptance_profiles import WEB_DEBUG_PROFILE, select_profile, stage_tools


class RouteProbe(BaseHTTPRequestHandler):
    """The external route answers its first readiness probe and refuses the repeat."""

    def do_POST(self):
        self.server.probes.append(self.path)
        matched = self.path == "/e1crdbg/rdbg?cmd=getDbgTargets" and len(self.server.probes) == 1
        body = b'{"#exception":"{http://v8.1c.ru/8.2/virtual-resource-system}Exception"}' if matched else b""
        self.send_response(400 if matched else 503)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def run_route_case(test, plugin, binary):
    from tests.ci import test_acceptance_scenarios as acceptance
    from unittest.mock import patch

    corpus = select_profile(load_acceptance_corpus(), WEB_DEBUG_PROFILE)
    with HTTPServer(("127.0.0.1", 0), RouteProbe) as route:
        route.probes = []
        publisher = f"http://127.0.0.1:{route.server_port}"
        thread = threading.Thread(target=route.serve_forever, daemon=True)
        thread.start()
        try:
            def prepare(server, _scenario):
                config = server.workspace / "v8project.yaml"
                with config.open("a", encoding="utf-8") as stream:
                    stream.write("\nworkPath: work\ninfobase:\n  connection: 'File=ib'\n  web:\n")
                    stream.write("    url: 'http://localhost/demo/?L=ru'\n    dir: external\n")
                    stream.write("    debugger-url: " + json.dumps(publisher) + "\n")
                descriptor = server.workspace / "external/default.vrd"
                descriptor.parent.mkdir()
                contents = (
                    '<point xmlns="http://v8.1c.ru/8.2/virtual-resource-system" base="/demo">'
                    f'<debug enable="true" protocol="http" url="{publisher}"/></point>'
                ).encode()
                descriptor.write_bytes(contents)

                def finish(*_):
                    test.assertEqual(descriptor.read_bytes(), contents)
                    test.assertGreaterEqual(len(route.probes), 2)
                    test.assertTrue(all(path == "/e1crdbg/rdbg?cmd=getDbgTargets" for path in route.probes))

                return finish

            with patch.object(acceptance, "BINARY", binary):
                acceptance.run_corpus(test, corpus, plugin, scenario_driver=prepare)
        finally:
            route.shutdown()
            thread.join()


class ExternalWebDebugCase(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("UNICA_PREPARED_BINARY_MANIFEST"), "prepare an exact Unica product first")
    def test_packaged_runner_web_debug_route(self):
        from tests.ci import test_acceptance_scenarios as acceptance

        with temp_tree("unica-web-debug-corpus-") as temporary:
            plugin = stage_tools(Path(temporary) / "plugin", ("v8-runner",))
            run_route_case(self, plugin, acceptance.BINARY)
