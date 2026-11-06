"""The controlled supplier seam cannot accept arbitrary commands or CF bytes."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tests.ci.support_restore_fixture import NATIVE, PRODUCER


class SupplierProducerTests(unittest.TestCase):
    def refused(self, *, command="convert", change_cf=False):
        with tempfile.TemporaryDirectory(prefix="unica-supplier-test-") as raw:
            root = Path(raw).resolve()
            script = root / "producer.py"
            shutil.copyfile(PRODUCER, script)
            shutil.copytree(NATIVE, root / "baseline")
            project = root / "project"
            project.mkdir()
            config = project / "v8project.yaml"
            config.write_text("format: DESIGNER\n", encoding="utf-8")
            cf = root / "supplier.cf"
            shutil.copyfile(NATIVE / "Ext/ParentConfigurations/РеактивныйHTML.cf", cf)
            if change_cf:
                cf.write_bytes(cf.read_bytes() + b"changed")
            output = root / "export"
            result = subprocess.run([sys.executable, str(script), "--config", str(config),
                                     "--json-message", command, str(cf), "--to", "xml",
                                     "--output", str(output)], capture_output=True, timeout=15)
            self.assertEqual(result.returncode, 4)
            self.assertFalse(json.loads(result.stdout)["ok"])
            self.assertFalse(output.exists())

    def test_modified_cf_is_not_an_immutable_supplier(self):
        self.refused(change_cf=True)

    def test_unknown_command_never_falls_back_to_the_platform(self):
        self.refused(command="push")
