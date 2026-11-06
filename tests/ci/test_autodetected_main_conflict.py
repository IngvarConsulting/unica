"""The public workspace check explains discarded autodetected main roots."""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from scripts.ci.prepared_binary import ensure_binary
from tests.ci.acceptance_controls import tool_result
from tests.ci.acceptance_profiles import completed_delivery_call, isolated_environment
from tests.ci.test_acceptance_scenarios import AcceptanceServer, REPO_ROOT
from tests.ci.test_unica_mcp_smoke import snapshot_workspace_files


CONFLICT = "source_set.autodetected_main_conflict"


class AutodetectedMainConflictWireTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        ensure_binary(REPO_ROOT)

    def _start(self, roots, config=None):
        temporary = tempfile.TemporaryDirectory(prefix="unica-autodetected-main-")
        self.addCleanup(temporary.cleanup)
        temporary_root = Path(temporary.name).resolve()
        self.workspace = temporary_root / "workspace"
        self.workspace.mkdir()
        for index, relative in enumerate(roots):
            root = self.workspace / relative
            root.mkdir(parents=True, exist_ok=True)
            (root / "Configuration.xml").write_text(
                '<?xml version="1.0" encoding="UTF-8"?>\n'
                '<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">'
                '<Configuration><Properties>'
                f'<Name>Source{index}</Name>'
                '</Properties><ChildObjects/></Configuration></MetaDataObject>\n',
                encoding="utf-8",
            )
        if config is not None:
            (self.workspace / "v8project.yaml").write_text(config, encoding="utf-8")
        self.expected_names = {relative: f"Source{index}" for index, relative in enumerate(roots)}
        self.before = snapshot_workspace_files(self.workspace)
        self.before_metadata = self._entry_metadata()
        state = temporary_root / "state"
        state.mkdir()
        environment = isolated_environment(REPO_ROOT / "plugins/unica", state)
        environment["UNICA_CACHE_DIR"] = str(state / "workspace-cache")
        self.server = AcceptanceServer(self.workspace, state, "2025-11-25", environment)
        self.addCleanup(self.server.close)

    def _entry_metadata(self):
        return {
            path.relative_to(self.workspace).as_posix():
                (metadata.st_mode, metadata.st_dev, metadata.st_ino, metadata.st_mtime_ns)
            for path in [self.workspace, *self.workspace.rglob("*")]
            for metadata in [path.lstat()]
        }

    def _call(self, tool, arguments):
        response = completed_delivery_call(self.server, tool, arguments, "autodetected-main")
        result = tool_result(response)
        self.assertIsInstance(result, dict, response)
        self.assertEqual(snapshot_workspace_files(self.workspace), self.before,
                         "discovery/read must not write workspace files")
        self.assertEqual(self._entry_metadata(), self.before_metadata,
                         "discovery/read must preserve entry identities and create no directories")
        return result

    def _diagnostics(self):
        result = self._call("unica.check", {})
        self.assertIs(result.get("ok"), True, result)
        self.check_data = result.get("data") or {}
        return (result.get("data") or {}).get("diagnostics") or []

    def _selected_root_is_readable(self, source_set, selected):
        at = f"{source_set}:Configuration"
        result = self._call("unica.view", {"at": at})
        self.assertIs(result.get("ok"), True, result)
        self.assertEqual(result["data"]["props"]["name"], self.expected_names[selected], result)
        workspace = self._call("unica.view", {})
        selected_sets = [item for item in workspace["data"]["sourceSets"]
                         if item["name"] == source_set]
        self.assertEqual([item["path"] for item in selected_sets], [selected], workspace)
        resolved = self._call("unica.resolve", {"at": at})
        self.assertIs(resolved.get("ok"), True, resolved)
        self.assertEqual(resolved["data"]["path"], "Configuration.xml", resolved)

    def _conflict(self, selected, skipped):
        diagnostics = self._diagnostics()
        conflicts = [item for item in diagnostics if item.get("code") == CONFLICT]
        self.assertEqual(len(conflicts), 1, diagnostics)
        conflict = conflicts[0]
        self.assertIs(self.check_data.get("discoveredReady"), True, self.check_data)
        self.assertEqual(conflict.get("severity"), "warning", conflict)
        self.assertEqual(conflict.get("count"), len(skipped), conflict)
        self.assertEqual(set(conflict.get("paths") or []), {selected, *skipped}, conflict)
        self.assertEqual(set(conflict.get("evidence") or []),
                         {f"selected: {selected}", *(f"skipped: {path}" for path in skipped)}, conflict)
        self.assertTrue((conflict.get("remediation") or {}).get("steps"), conflict)
        self.assertFalse(any(item.get("code") == "source_set.name_ambiguous"
                             for item in diagnostics), diagnostics)
        self._selected_root_is_readable("main", selected)

    def test_base_keeps_main_and_names_both_skipped_extensions(self):
        """rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT"""
        self._start(["src/cf", "src/cfe/main", "src/extensions/main"])
        self._conflict("src/cf", ["src/cfe/main", "src/extensions/main"])

    def test_without_base_the_first_layout_keeps_main(self):
        """rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT"""
        self._start(["src/cfe/main", "src/extensions/main"])
        self._conflict("src/cfe/main", ["src/extensions/main"])

    def test_only_extension_main_has_no_conflict(self):
        """rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT"""
        self._start(["src/extensions/main"])
        self.assertFalse(any(item.get("code") == CONFLICT for item in self._diagnostics()))
        self._selected_root_is_readable("main", "src/extensions/main")

    def test_extension_root_container_does_not_publish_its_nested_main(self):
        """rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT"""
        self._start(["src/cfe", "src/cfe/main"])
        self.assertFalse(any(item.get("code") == CONFLICT for item in self._diagnostics()))
        self._selected_root_is_readable("cfe", "src/cfe")
        missing = self._call("unica.view", {"at": "main:Configuration"})
        self.assertIs(missing.get("ok"), False, missing)

    def test_explicit_distinct_names_expose_both_former_claimants(self):
        """rule: INV.SOURCE.AUTODETECTED-MAIN-CONFLICT"""
        self._start(["src/cfe/main", "src/extensions/main"],
                    self._config([("first", "src/cfe/main"), ("second", "src/extensions/main")]))
        self.assertFalse(any(item.get("code") == CONFLICT for item in self._diagnostics()))
        self._selected_root_is_readable("first", "src/cfe/main")
        self._selected_root_is_readable("second", "src/extensions/main")

    @staticmethod
    def _config(entries):
        return "format: DESIGNER\nsource-set:\n" + "".join(
            f"  - name: {name}\n    type: EXTENSION\n    path: {path}\n"
            for name, path in entries
        )

    def test_explicit_duplicate_main_stays_ambiguous(self):
        """rule: INV.SOURCE.AMBIGUOUS-NAME-HEALTH"""
        self._start(["src/cfe/main", "src/extensions/main"],
                    self._config([("main", "src/cfe/main"), ("main", "src/extensions/main")]))
        diagnostics = self._diagnostics()
        duplicate = [item for item in diagnostics if item.get("code") == "source_set.name_ambiguous"]
        self.assertEqual(len(duplicate), 1, diagnostics)
        self.assertEqual(duplicate[0].get("count"), 2, duplicate)
        self.assertFalse(any(item.get("code") == CONFLICT for item in diagnostics))
        self.assertIs(self._call("unica.view", {"at": "main:Configuration"}).get("ok"), False)

    def test_other_autodetected_duplicate_name_stays_ambiguous(self):
        """rule: INV.SOURCE.AMBIGUOUS-NAME-HEALTH"""
        self._start(["src/cfe/shared", "src/extensions/shared"])
        diagnostics = self._diagnostics()
        duplicate = [item for item in diagnostics if item.get("code") == "source_set.name_ambiguous"]
        self.assertEqual(len(duplicate), 1, diagnostics)
        self.assertEqual(duplicate[0].get("count"), 2, duplicate)
        self.assertFalse(any(item.get("code") == CONFLICT for item in diagnostics))
        self.assertIs(self._call("unica.view", {"at": "shared:Configuration"}).get("ok"), False)
