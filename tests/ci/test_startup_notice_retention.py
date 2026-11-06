"""Actual MCP delivery boundary; launcher failures are covered by the Rust CLI tests."""
from __future__ import annotations

import importlib.util
import json
import os
import tempfile
import time
import unittest
import uuid
from contextlib import contextmanager
from pathlib import Path

from scripts.ci.prepared_binary import ensure_binary


ROOT = Path(__file__).resolve().parents[2]
NOTICE = "Interrupted installation notice: retained until the real MCP session opens"
MODERN_META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {},
}


def wire_module():
    spec = importlib.util.spec_from_file_location(
        "startup_notice_wire", ROOT / "scripts/ci/probe-unica-wire.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def initialize(request_id=1):
    return {
        "jsonrpc": "2.0", "id": request_id, "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "startup-notice-test", "version": "1"},
        },
    }


def modern_request(method, request_id=2, **params):
    return {
        "jsonrpc": "2.0", "id": request_id, "method": method,
        "params": {**params, "_meta": MODERN_META},
    }


class NoticeJournal:
    """Initial on-disk evidence, independently observed as JSONL events."""

    def __init__(self, root):
        self.cache = root / "artifacts"
        self.path = self.cache / ".attempts" / "unica" / "test-version-test-target.jsonl"
        self.path.parent.mkdir(parents=True)
        self.partial = root / "received.partial"
        self.partial.write_bytes(b"already received bytes")
        self.old_id = self.add_attempt()

    def add_attempt(self):
        attempt = str(uuid.uuid4())
        event = {
            "event": "started", "attempt": attempt, "at": 1,
            "artifact": "unica", "version": "test-version", "target": "test-target",
            "url": "https://github.com/IngvarConsulting/unica/releases/download/vtest/runtime.tar.gz",
            "partial": str(self.partial), "stage": "download",
        }
        with self.path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(event) + "\n")
        return attempt

    def unfinished(self):
        if not self.path.exists():
            return set()
        events = [json.loads(line) for line in self.path.read_text(encoding="utf-8").splitlines()]
        started = {event["attempt"] for event in events if event["event"] == "started"}
        terminal = {
            event["attempt"] for event in events if event["event"] in {"closed", "reported"}
        }
        return started - terminal


class StartupNoticeRetentionTests(unittest.TestCase):
    """rule: INV.PKG.STARTUP-NOTICE-RETENTION"""

    @classmethod
    def setUpClass(cls):
        cls.binary = ensure_binary(ROOT)
        cls.wire = wire_module()

    @contextmanager
    def session(self, root, journal, *, with_notice=True):
        state = root / "state"
        env = os.environ.copy()
        for key in (
            "UNICA_RUNTIME_MANIFEST", "UNICA_PLUGIN_ROOT", "UNICA_STARTUP_NOTICE",
            "UNICA_STARTUP_NOTICE_ACK", "CLAUDE_PROJECT_DIR", "ZCODE_PROJECT_DIR",
        ):
            env.pop(key, None)
        env.update({
            "UNICA_ARTIFACT_CACHE": str(journal.cache),
            "UNICA_PROVIDER_STATE_DIR": str(state),
            "UNICA_DAEMON_IDLE_GRACE_MS": "100",
            "UNICA_STARTUP_NOTICE_ACK": json.dumps([journal.old_id]),
        })
        if with_notice:
            env["UNICA_STARTUP_NOTICE"] = NOTICE
        session = self.wire.JsonRpcSession(
            [str(self.binary)], env, cwd=root,
            deadline=time.monotonic() + 60, timeout_seconds=30,
        )
        identities = set()
        try:
            yield session
        finally:
            # The isolated endpoint names only this test's daemon. Capture its
            # process identity before closing the frontend; never signal a bare PID.
            pids = set()
            for path in state.glob("*/endpoint.json"):
                try:
                    pids.add(json.loads(path.read_text(encoding="utf-8"))["pid"])
                except FileNotFoundError:
                    pass
            identities.update(session.process_ownership.capture_identities(pids))
            try:
                if session.process.poll() is None:
                    session.close()
            finally:
                session.terminate_tree(identities)

    def test_legacy_initialize_acknowledges_only_the_delivered_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                newer = journal.add_attempt()
                response = session.request(initialize())
                self.assertIn(NOTICE, response["result"]["instructions"])
                response = session.request({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
                self.assertIn("tools", response["result"])
            self.assertEqual(journal.unfinished(), {newer})
            self.assertEqual(journal.partial.read_bytes(), b"already received bytes")

    def test_discovery_notice_waits_for_a_successful_modern_opener(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                response = session.request(modern_request("server/discover", 1))
                self.assertIn(NOTICE, json.dumps(response, ensure_ascii=False))
                self.assertEqual(journal.unfinished(), {journal.old_id})
                newer = journal.add_attempt()
                response = session.request(modern_request("tools/list"))
                self.assertIn("tools", response["result"])
            self.assertEqual(journal.unfinished(), {newer})

    def test_discovery_only_preserves_the_notice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                response = session.request(modern_request("server/discover", 1))
                self.assertIn(NOTICE, json.dumps(response, ensure_ascii=False))
            self.assertEqual(journal.unfinished(), {journal.old_id})

    def test_discovery_then_legacy_initialize_acknowledges_the_notice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                response = session.request(modern_request("server/discover", 1))
                self.assertIn(NOTICE, json.dumps(response, ensure_ascii=False))
                self.assertEqual(journal.unfinished(), {journal.old_id})
                response = session.request(initialize(2))
                self.assertIn(NOTICE, response["result"]["instructions"])
            self.assertEqual(journal.unfinished(), set())

    def test_discovery_then_an_error_opener_preserves_the_notice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                response = session.request(modern_request("server/discover", 1))
                self.assertIn(NOTICE, json.dumps(response, ensure_ascii=False))
                response = session.request(modern_request(
                    "tools/call", name="unica.nonexistent", arguments={},
                ))
                self.assertIn("error", response)
            self.assertEqual(journal.unfinished(), {journal.old_id})

    def test_direct_modern_opener_without_notice_delivery_does_not_ack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                response = session.request(modern_request("tools/list", 1))
                self.assertIn("tools", response["result"])
            self.assertEqual(journal.unfinished(), {journal.old_id})

    def test_ack_context_without_a_notice_does_not_ack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal, with_notice=False) as session:
                response = session.request(initialize())
                self.assertNotIn(NOTICE, response["result"]["instructions"])
            self.assertEqual(journal.unfinished(), {journal.old_id})

    def test_killed_frontend_before_initialize_preserves_the_next_start_notice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            journal = NoticeJournal(root)
            with self.session(root, journal) as session:
                # A real protocol response proves readiness, but a pre-init ping
                # carries no notice and does not establish the MCP session.
                response = session.request({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}})
                self.assertIn("result", response)
                session.terminate_tree()
            self.assertEqual(journal.unfinished(), {journal.old_id})
            with self.session(root, journal) as session:
                response = session.request(initialize())
                self.assertIn(NOTICE, response["result"]["instructions"])
            self.assertEqual(journal.unfinished(), set())


if __name__ == "__main__":
    unittest.main()
