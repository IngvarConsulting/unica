"""Временные каталоги тестов удаляются, даже если их ещё держит процесс.

Демон Unica держит файлы своего состояния до конца паузы простоя, и Windows
не удаляет открытый файл. Удаление ждёт, а не оставляет каталог; оставленное
тестом прогон называет поимённо.
"""
from __future__ import annotations

import importlib.util
import io
import os
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts.ci import temp_tree

REPO_ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("run_unittest_for_temp", REPO_ROOT / "scripts/ci/run-unittest.py")
run_unittest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run_unittest)


class RemoveTreeTests(unittest.TestCase):
    def test_removal_waits_for_the_holder_and_then_succeeds(self):
        calls = []

        def busy_twice(path, **_options):
            calls.append(path)
            if len(calls) < 3:
                raise PermissionError(32, "busy", str(path))

        with mock.patch.object(temp_tree.shutil, "rmtree", side_effect=busy_twice), \
             mock.patch.object(temp_tree, "_processes", return_value=[]), \
             mock.patch.object(temp_tree.time, "sleep"):
            temp_tree.remove_tree("held", timeout=30)
        self.assertEqual(len(calls), 3)

    def test_a_directory_that_stays_held_is_an_error_naming_it(self):
        clock = iter(range(0, 1000, 10))
        with mock.patch.object(temp_tree.shutil, "rmtree", side_effect=PermissionError(32, "busy")), \
             mock.patch.object(temp_tree, "_processes", return_value=[]), \
             mock.patch.object(temp_tree.time, "sleep"), \
             mock.patch.object(temp_tree.time, "monotonic", side_effect=lambda: next(clock)):
            with self.assertRaisesRegex(OSError, "held.*не удалён за 30 с"):
                temp_tree.remove_tree("held", timeout=30)

    def test_only_processes_naming_a_path_inside_the_tree_are_stopped(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            processes = [
                (4242, f"unica.exe --daemon --state-root {root / 'state'} --core-identity x"),
                (4343, f"bsl-analyzer serve {root / 'workspace' / 'src'}"),
                (4444, f"unica.exe --daemon --state-root {root.parent / 'elsewhere' / 'state'}"),
                (os.getpid(), f"python -m unittest {root}"),
            ]
            with mock.patch.object(temp_tree, "_processes", return_value=processes),                  mock.patch.object(temp_tree.os, "kill") as kill:
                self.assertEqual(temp_tree.stop_holders(raw), [4242, 4343])
            self.assertEqual([call.args[0] for call in kill.call_args_list], [4242, 4343])

    def test_a_read_only_file_is_removed(self):
        with tempfile.TemporaryDirectory() as raw:
            tree = Path(raw, "tree")
            tree.mkdir()
            locked = tree / "objects.pack"
            locked.write_bytes(b"pack")
            os.chmod(locked, stat.S_IREAD)
            with mock.patch.object(temp_tree, "_processes", return_value=[]):
                temp_tree.remove_tree(tree, timeout=5)
            self.assertFalse(tree.exists())

    def test_the_context_removes_its_directory(self):
        with mock.patch.object(temp_tree, "_processes", return_value=[]),              temp_tree.temp_tree("temp-tree-test-") as path:
            Path(path, "file").write_text("x", encoding="utf-8")
        self.assertFalse(Path(path).exists())


class TempLedgerTests(unittest.TestCase):
    def test_each_test_is_named_with_what_it_left_and_the_run_directory_goes(self):
        saved = {name: os.environ.get(name) for name in ("TEMP", "TMP", "TMPDIR")}
        saved_tempdir = tempfile.tempdir
        try:
            ledger = run_unittest.TempLedger()
            self.assertEqual(Path(tempfile.gettempdir()), ledger.root)
            before = ledger.entries()
            (ledger.root / "left-by-test").mkdir()
            self.assertEqual(ledger.record("module.Class.test_leaks", before), ["left-by-test"])
            self.assertEqual(ledger.record("module.Class.test_clean", ledger.entries()), [])
            stream = io.StringIO()
            ledger.close(stream)
            self.assertIn("module.Class.test_leaks: left-by-test", stream.getvalue())
            self.assertNotIn("test_clean", stream.getvalue())
            self.assertFalse(ledger.root.exists())
        finally:
            tempfile.tempdir = saved_tempdir
            for name, value in saved.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value


if __name__ == "__main__":
    unittest.main()
