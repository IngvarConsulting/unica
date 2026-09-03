from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "scripts"
    / "dev"
    / "mcp-progress-probe-server.py"
)


def load_module():
    spec = importlib.util.spec_from_file_location("mcp_progress_probe_server", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ProgressProbeArgumentTests(unittest.TestCase):
    def test_invalid_call_arguments_return_json_rpc_error_without_starting_work(self) -> None:
        module = load_module()
        request = {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "probe_long_call",
                "arguments": {"seconds": "not-a-number", "progressEveryMs": "also-bad"},
            },
        }

        with mock.patch.object(module.threading, "Thread") as thread:
            response = module.handle(request)

        thread.assert_not_called()
        self.assertEqual(response["id"], 7)
        self.assertEqual(response["error"]["code"], -32602)
        self.assertIn("seconds", response["error"]["message"])


if __name__ == "__main__":
    unittest.main()
