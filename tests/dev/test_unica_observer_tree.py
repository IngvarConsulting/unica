"""Exercise the shipped tree renderer and events in a minimal DOM with Node."""
from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("observer_tree", ROOT / "scripts/dev/unica-observer.py")
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)


class ObserverTreeTests(unittest.TestCase):
    def test_interface_groups_reuse_returned_method_addresses_and_toggle_without_calls(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node.js is required to execute the observer tree renderer")
        result = subprocess.run(
            [node, str(ROOT / "tests/dev/fixtures/unica_observer_tree.js")],
            input=json.dumps({"source": observer.TREE_RENDERING}), text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
