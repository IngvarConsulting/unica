"""Whole-template removal is inspectable through the same saved plan that is applied."""
from __future__ import annotations

import base64
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest

from tests.ci.acceptance_controls import tool_result
from tests.ci.acceptance_profiles import completed_delivery_call, isolated_environment
from tests.ci.test_acceptance_scenarios import AcceptanceServer, REPO_ROOT
from tests.ci.test_unica_mcp_smoke import snapshot_workspace_files


FIXTURE = REPO_ROOT / "tests/fixtures/v013/whole-template-removal/workspace"
OWNER = "Reports/OwnRemovalReport.xml"
TEMPLATES = "Reports/OwnRemovalReport/Templates"
RED = f"{TEMPLATES}/OwnDiagram/Ext/Template/Items/RedSquare/Picture.png"
BLUE = f"{TEMPLATES}/OwnDiagram/Ext/Template/Items/BlueSquare/Picture.png"
CHUNK_FIELDS = {
    "pathEncoding", "path", "side", "present", "totalBytes", "rangeStart",
    "rangeEnd", "startByte", "endByte", "encoding", "content",
}


def workspace_image(root: Path):
    """Add no-follow topology/identity to the existing full file-byte observer.

    The copied fixture is quiescent between completed calls. This is a test
    oracle, not a new production filesystem admission or racing-reader policy.
    """
    entries = {}

    def visit(path):
        metadata = path.lstat()
        if (stat.S_ISLNK(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0)
                & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)):
            raise AssertionError("unexpected link/reparse point in the owned fixture")
        relative = path.relative_to(root).as_posix()
        identity = (metadata.st_dev, metadata.st_ino)
        mode = stat.S_IMODE(metadata.st_mode)
        if stat.S_ISDIR(metadata.st_mode):
            entries[relative] = ("directory", None, mode, identity)
            for child in path.iterdir():
                visit(child)
        elif stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1:
            entries[relative] = ("file", None, mode, identity)
        else:
            raise AssertionError("unexpected nonregular/hardlinked fixture entry")

    visit(root)
    files = snapshot_workspace_files(root)
    if set(files) != {path for path, entry in entries.items() if entry[0] == "file"}:
        raise AssertionError("fixture membership changed while taking its snapshot")
    for path, content in files.items():
        entry = entries[path]
        entries[path] = (entry[0], content, *entry[2:])
    return entries


class WholeTemplatePlanDiffWireTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(
            ["cargo", "build", "--quiet", "--package", "unica-coder", "--bin", "unica"],
            cwd=REPO_ROOT,
            check=True,
        )

    def _start(self):
        temporary = tempfile.TemporaryDirectory(prefix="unica-whole-template-plan-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name).resolve()
        self.workspace = root / "workspace"
        shutil.copytree(FIXTURE, self.workspace)
        (self.workspace / "unrelated/empty").mkdir(parents=True)
        (self.workspace / "unrelated/keep.bin").write_bytes(b"unrelated\x00\xff\r\n")
        state = root / "state"
        state.mkdir()
        environment = isolated_environment(REPO_ROOT / "plugins/unica", state)
        # This test exercises the actor's existing default cache participant.
        environment.pop("UNICA_CACHE_DIR", None)
        self.server = AcceptanceServer(self.workspace, state, "2025-11-25", environment)
        self.addCleanup(self.server.close)
        self.before = workspace_image(self.workspace)
        self.assertFalse((self.workspace / ".build").exists(), "the fixture starts without cache")

    def _call(self, tool, arguments):
        # This helper follows an issued Task; it never retries a successful apply.
        response = completed_delivery_call(self.server, tool, arguments, "whole-template-plan")
        result = tool_result(response)
        self.assertTrue(isinstance(result, dict), "a canonical tool result is required")
        return result

    def _unchanged(self, expected):
        self.assertTrue(workspace_image(self.workspace) == expected, "read/refusal changed workspace entries")
        self.assertFalse((self.workspace / ".build").exists(), "read/refusal created cache")

    def _read_only_call(self, tool, arguments, expected):
        result = self._call(tool, arguments)
        self._unchanged(expected)
        return result

    def _plan(self, selected):
        at = f"main:Report.OwnRemovalReport.Template.{selected}"
        plan = self._read_only_call(
            "unica.apply", {"at": at, "ops": [{"op": "template.remove", "args": {"at": at}}]},
            self.before,
        )
        self.assertIs(plan.get("ok"), True, "the valid whole template must be plannable")
        data = plan.get("data") or {}
        self.assertEqual(data.get("mode"), "preview")
        token = data.get("executionToken")
        identity = plan.get("rev")
        self.assertTrue(isinstance(token, str) and bool(token), "a saved plan token is required")
        self.assertTrue(isinstance(identity, str) and bool(identity), "the preview names its plan identity")
        return token, identity

    def _refused(self, result, code):
        self.assertIs(result.get("ok"), False, "the invalid/stale request must refuse")
        self.assertTrue(result.get("changed", []) == [], "a refusal commits no changes")
        codes = [item.get("code") for item in result.get("diagnostics") or []]
        self.assertIn(code, codes)
        self.assertIsNone((result.get("data") or {}).get("executionToken"))

    def _pages(self, token, identity, first_page):
        cursor = None
        seen = set()
        chunks = []
        # Every nonempty chunk must consume a source byte, every empty side one position.
        source_bytes = sum(len(value[1]) for value in self.before.values() if value[0] == "file")
        remaining_positions = 2 * source_bytes + 2 * len(self.before)
        while True:
            arguments = {"executionToken": token, "limit": 1}
            if cursor is not None:
                arguments["cursor"] = cursor
            result = self._read_only_call("unica.diff", arguments, self.before)
            if cursor is None:
                self.assertTrue(result == first_page, "the original first page survives cursor refusals")
            replay = self._read_only_call("unica.diff", arguments, self.before)
            self.assertTrue(replay == result, "each pending-plan page must replay exactly")
            self.assertIs(result.get("ok"), True, "saved source pages must succeed")
            data = result.get("data") or {}
            self.assertTrue(data.get("planIdentity") == identity, "all pages belong to the previewed plan")
            self.assertEqual(data.get("sourceSet"), "main")
            page = data.get("chunks")
            self.assertTrue(isinstance(page, list) and len(page) == 1, "limit1 returns exactly one chunk")
            chunk = page[0]
            self.assertEqual(set(chunk), CHUNK_FIELDS)
            for field in ["totalBytes", "rangeStart", "rangeEnd", "startByte", "endByte"]:
                self.assertIs(type(chunk[field]), int, "byte positions are integers")
            self.assertIs(type(chunk["present"]), bool)
            self.assertEqual(chunk["pathEncoding"], "utf8", "the fixture paths remain ordinary text")
            self.assertIn(chunk["side"], ["before", "after"])
            self.assertIn(chunk["encoding"], ["utf8", "base64"])
            self.assertIsInstance(chunk["content"], str)
            content = (
                chunk["content"].encode("utf-8") if chunk["encoding"] == "utf8"
                else base64.b64decode(chunk["content"], validate=True)
            )
            self.assertTrue(0 <= chunk["rangeStart"] <= chunk["startByte"] <= chunk["endByte"]
                            <= chunk["rangeEnd"] <= chunk["totalBytes"], "invalid source byte range")
            self.assertEqual(len(content), chunk["endByte"] - chunk["startByte"])
            chunks.append((chunk, content))
            remaining_positions -= max(1, len(content))
            self.assertGreaterEqual(remaining_positions, 0, "page stream exceeds the fixture's finite source extent")
            next_cursor = data.get("cursor")
            if next_cursor is None:
                self.assertIs(data.get("truncated"), False)
                self.assertEqual(result.get("page"), {"stoppedBy": "complete"})
                self.assertEqual(result.get("next", []), [], "the complete page has no continuation")
                return chunks
            self.assertTrue(isinstance(next_cursor, str) and bool(next_cursor), "continuation is nonempty")
            self.assertTrue(next_cursor not in seen, "continuation must advance")
            seen.add(next_cursor)
            self.assertIs(data.get("truncated"), True)
            self.assertEqual(result.get("page"), {"stoppedBy": "limit"})
            self.assertTrue(result.get("next") == [{
                "tool": "unica.diff",
                "arguments": {"executionToken": token, "limit": 1, "cursor": next_cursor},
            }], "continuation keeps this exact token, limit and cursor")
            cursor = next_cursor

    def _planned_files(self, selected, chunks):
        deleted = {f"{TEMPLATES}/{selected}.xml", f"{TEMPLATES}/{selected}/Ext/Template.xml"}
        if selected == "OwnDiagram":
            deleted.update([RED, BLUE])
        paths = sorted({OWNER, *deleted})
        expected_order = [("utf8", path, side) for path in paths for side in ["before", "after"]]
        groups = {}
        order = []
        for chunk, content in chunks:
            key = (chunk["pathEncoding"], chunk["path"], chunk["side"])
            if not order or order[-1] != key:
                self.assertNotIn(key, groups, "a source side cannot reappear out of order")
                order.append(key)
                groups[key] = []
            groups[key].append((chunk, content))
        self.assertTrue(order == expected_order, "preview must enumerate exactly the owner and selected files")

        def window(path, side):
            parts = groups[("utf8", path, side)]
            first = parts[0][0]
            position = first["rangeStart"]
            data = bytearray()
            for chunk, content in parts:
                for field in ["present", "totalBytes", "rangeStart", "rangeEnd", "encoding"]:
                    self.assertEqual(chunk[field], first[field], "source-side metadata is stable")
                self.assertEqual(chunk["startByte"], position, "chunks have no byte gaps or overlaps")
                position = chunk["endByte"]
                data.extend(content)
            self.assertEqual(position, first["rangeEnd"], "the entire promised range was read")
            return first, bytes(data)

        expected = {path: value[1] for path, value in self.before.items() if value[0] == "file"}
        original_owner = expected[f"src/{OWNER}"]
        selected_line = f"      <Template>{selected}</Template>\n".encode()
        self.assertEqual(original_owner.count(selected_line), 1)
        expected_owner = original_owner.replace(selected_line, b"", 1)
        before, before_window = window(OWNER, "before")
        after, after_window = window(OWNER, "after")
        self.assertIs(before["present"], True)
        self.assertIs(after["present"], True)
        self.assertEqual(before["totalBytes"], len(original_owner))
        self.assertEqual(after["totalBytes"], len(expected_owner))
        self.assertEqual(before["rangeStart"], after["rangeStart"])
        self.assertEqual(before_window, original_owner[before["rangeStart"]:before["rangeEnd"]])
        self.assertEqual(after_window, expected_owner[after["rangeStart"]:after["rangeEnd"]])
        self.assertEqual(original_owner[:before["rangeStart"]], expected_owner[:after["rangeStart"]])
        self.assertEqual(original_owner[before["rangeEnd"]:], expected_owner[after["rangeEnd"]:])
        reconstructed = original_owner[:before["rangeStart"]] + after_window + original_owner[before["rangeEnd"]:]
        self.assertEqual(reconstructed, expected_owner, "public changed windows reconstruct the exact owner postimage")
        expected[f"src/{OWNER}"] = reconstructed
        for path in deleted:
            original = expected.pop(f"src/{path}")
            before, before_window = window(path, "before")
            after, after_window = window(path, "after")
            self.assertIs(before["present"], True)
            self.assertEqual((before["rangeStart"], before["rangeEnd"], before["totalBytes"]),
                             (0, len(original), len(original)))
            self.assertEqual(before_window, original, "deleted-file preview contains every original byte")
            self.assertIs(after["present"], False)
            self.assertEqual((after["rangeStart"], after["rangeEnd"], after["totalBytes"]), (0, 0, 0))
            self.assertEqual(after_window, b"")
            if path.endswith(".png"):
                self.assertEqual(before["encoding"], "base64", "PNG bytes are lossless binary content")
        return expected

    def _assert_postimage(self, selected, expected):
        post = workspace_image(self.workspace)
        cache_present = ".build/unica" in post
        if cache_present:
            self.assertEqual(post[".build/unica"][0], "directory")
            self.assertEqual(post[".build"][0], "directory")
        public = {}
        for path, entry in post.items():
            if cache_present and (path in [".build", ".build/unica"] or path.startswith(".build/unica/")):
                continue
            public[path] = entry
        files = {path: value[1] for path, value in public.items() if value[0] == "file"}
        self.assertTrue(files == expected, "apply must publish exactly the source postimage established before execution")
        selected_root = f"src/{TEMPLATES}/{selected}"
        for path, entry in public.items():
            self.assertIn(path, self.before, "apply created an unplanned public entry")
            if path != f"src/{OWNER}":
                self.assertEqual(entry, self.before[path], "untouched entry bytes, identity or mode changed")
        for path, entry in self.before.items():
            if entry[0] != "file" and path != selected_root and not path.startswith(selected_root + "/"):
                self.assertEqual(public.get(path), entry, "apply lost an unrelated entry")
        return post

    def test_complete_removal_pages_define_the_same_token_publication(self):
        for selected in ["OwnDiagram", "KeepDiagram"]:
            with self.subTest(template=selected):
                self._start()
                token, identity = self._plan(selected)
                first = self._read_only_call("unica.diff", {"executionToken": token, "limit": 1}, self.before)
                self.assertIs(first.get("ok"), True)
                cursor = (first.get("data") or {}).get("cursor")
                self.assertTrue(isinstance(cursor, str) and bool(cursor), "this plan spans several chunks")
                foreign_token, _ = self._plan(selected)
                self.assertTrue(foreign_token != token, "distinct plans have distinct tokens")
                foreign = self._read_only_call(
                    "unica.diff", {"executionToken": foreign_token, "limit": 1}, self.before,
                )
                self.assertIs(foreign.get("ok"), True)
                foreign_cursor = (foreign.get("data") or {}).get("cursor")
                self.assertTrue(isinstance(foreign_cursor, str) and bool(foreign_cursor))
                for arguments in [
                    {"executionToken": token, "limit": 1, "cursor": foreign_cursor},
                    {"executionToken": token, "limit": 2, "cursor": cursor},
                ]:
                    self._refused(self._read_only_call("unica.diff", arguments, self.before), "bad_value")
                foreign_replay = self._read_only_call(
                    "unica.diff", {"executionToken": foreign_token, "limit": 1}, self.before,
                )
                self.assertTrue(foreign_replay == foreign, "cursor refusals cannot consume either plan binding")
                chunks = self._pages(token, identity, first)
                expected = self._planned_files(selected, chunks)
                self._unchanged(self.before)
                applied = self._call("unica.apply", {"executionToken": token})
                self.assertIs(applied.get("ok"), True, "cursor refusals cannot consume the saved plan")
                self.assertEqual((applied.get("data") or {}).get("mode"), "published")
                post = self._assert_postimage(selected, expected)
                replay = self._call("unica.apply", {"executionToken": token})
                self.assertTrue(replay == applied, "execution replays the exact same result")
                self.assertTrue(workspace_image(self.workspace) == post, "replay changes no entry, including cache")

    def _stale(self, mutation):
        self._start()
        token, _ = self._plan("OwnDiagram")
        first = self._read_only_call("unica.diff", {"executionToken": token, "limit": 1}, self.before)
        self.assertIs(first.get("ok"), True)
        cursor = (first.get("data") or {}).get("cursor")
        self.assertTrue(isinstance(cursor, str) and bool(cursor))
        mutation(self.workspace / "src")
        concurrent = workspace_image(self.workspace)
        self.assertTrue(concurrent != self.before, "the concurrent fixture mutation must be real")
        stale_diff = self._read_only_call(
            "unica.diff", {"executionToken": token, "limit": 1, "cursor": cursor}, concurrent,
        )
        self._refused(stale_diff, "stale_revision")
        stale_apply = self._read_only_call("unica.apply", {"executionToken": token}, concurrent)
        self._refused(stale_apply, "stale_revision")
        replay = self._read_only_call("unica.apply", {"executionToken": token}, concurrent)
        self.assertTrue(replay == stale_apply, "stale execution also replays exactly")

    def test_owner_mutation_makes_diff_and_apply_stale_without_overwriting_it(self):
        def mutate(source):
            path = source / OWNER
            original = path.read_bytes()
            old = b"<Template>OwnDiagram</Template>"
            self.assertEqual(original.count(old), 1)
            path.write_bytes(original.replace(old, b"<Template>ConcurrentDiagram</Template>", 1))
        self._stale(mutate)

    def test_file_membership_mutation_makes_diff_and_apply_stale_without_overwriting_it(self):
        def mutate(source):
            path = source / TEMPLATES / "OwnDiagram/Ext/Template/Items/Concurrent.bin"
            self.assertFalse(path.exists())
            path.write_bytes(b"concurrent member\x00\xff")
        self._stale(mutate)

    def test_picture_bytes_mutation_makes_diff_and_apply_stale_without_overwriting_it(self):
        def mutate(source):
            path = source / RED
            data = bytearray(path.read_bytes())
            self.assertGreater(len(data), 16)
            data[-1] ^= 1
            path.write_bytes(data)
        self._stale(mutate)
