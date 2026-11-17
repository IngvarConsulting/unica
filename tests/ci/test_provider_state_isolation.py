"""The process boundaries of the MCP test drivers give every daemon private state."""

from __future__ import annotations

import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tests.ci import test_unica_mcp_script_parity as parity
from tests.ci import test_unica_mcp_smoke as smoke
from tests.ci.test_tool_surface_ledger import load_generator


class LaunchObserved(Exception):
    def __init__(self, environment: dict[str, str], cwd: Path | None = None):
        super().__init__("MCP process launch observed")
        self.state = environment.get("UNICA_PROVIDER_STATE_DIR")
        self.idle_grace = environment.get("UNICA_DAEMON_IDLE_GRACE_MS")
        self.cwd = cwd


def stop_before_spawn(*_args, env: dict[str, str], cwd: Path | None = None, **_kwargs):
    raise LaunchObserved(env, cwd)


class ProviderStateIsolationTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = self.enterContext(tempfile.TemporaryDirectory(prefix="unica-state-guard-"))
        self.scope = Path(temporary).resolve()
        self.user_home = self.scope / "home"
        self.user_home.mkdir()
        self.user_state = self.user_home / ".unica" / "provider-state"
        self.enterContext(patch.dict(os.environ, {
            "HOME": str(self.user_home),
            "USERPROFILE": str(self.user_home),
            "UNICA_PROVIDER_STATE_DIR": str(self.user_state),
            "UNICA_DAEMON_IDLE_GRACE_MS": "900000",
        }))
        self.subprocess_probe = SimpleNamespace(
            Popen=stop_before_spawn, run=stop_before_spawn, PIPE=subprocess.PIPE
        )
        self.binary = self.scope / "unica"

    def assert_test_state(self, launch: LaunchObserved) -> Path:
        self.assertIsNotNone(launch.state)
        state = Path(launch.state)
        self.assertTrue(state.is_absolute())
        self.assertFalse(state.is_relative_to(self.user_home))
        self.assertEqual(launch.idle_grace, "5000")
        return state

    def test_notification_smoke_allocates_its_own_state(self) -> None:
        case = smoke.UnicaMcpSmokeTests("test_notifications_do_not_count_as_responses")
        self.addCleanup(case.doCleanups)
        with (
            patch.object(smoke, "ensure_binary", return_value=self.binary),
            patch.object(smoke, "subprocess", self.subprocess_probe),
            self.assertRaises(LaunchObserved) as caught,
        ):
            case.test_notifications_do_not_count_as_responses()
        self.assertTrue(self.assert_test_state(caught.exception).parent.is_dir())

    def test_interactive_smoke_uses_the_state_directory_from_its_fixture(self) -> None:
        case = smoke.UnicaMcpSmokeTests()
        state = self.scope / "interactive-state"
        with (
            patch.object(smoke, "subprocess", self.subprocess_probe),
            self.assertRaises(LaunchObserved) as caught,
            case.mcp_session(state_dir=state, command=[str(self.binary)]),
        ):
            self.fail("the product launch was not intercepted")
        self.assertEqual(self.assert_test_state(caught.exception), state)

    def test_parity_launches_share_state_only_inside_their_own_case(self) -> None:
        case = parity.UnicaMcpScriptParityTests()
        case.unica_bin = self.binary
        cache = self.scope / "cache"
        with patch.object(parity, "subprocess", self.subprocess_probe):
            with self.assertRaises(LaunchObserved) as listing:
                case.call_mcp_messages(
                    [{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}],
                    cache,
                    extra_env={"UNICA_PROVIDER_STATE_DIR": str(self.user_state)},
                )
            with (
                patch.object(type(case), "execution_by_tool", {"unica.view": "read"}),
                self.assertRaises(LaunchObserved) as invocation,
            ):
                case.call_mcp_tool("unica.view", {}, self.scope, cache)
        for launch in (listing.exception, invocation.exception):
            self.assertEqual(self.assert_test_state(launch), cache / "provider-state")

    def test_tool_registry_discards_its_temporary_state(self) -> None:
        generator = load_generator()
        with (
            patch.object(generator, "subprocess", self.subprocess_probe),
            self.assertRaises(LaunchObserved) as caught,
        ):
            generator.read_registry(self.binary)
        state = self.assert_test_state(caught.exception)
        self.assertFalse(state.parent.exists())

    def test_packaged_smoke_scopes_provider_state_to_its_workspace_fixture(self) -> None:
        module = smoke.source_smoke_oracle()

        def session(_command, environment, _timeout, *, cwd, **_kwargs):
            raise LaunchObserved(environment, cwd)

        with patch.object(module, "McpSession", session), self.assertRaises(LaunchObserved) as caught:
            plugin_root = Path(__file__).resolve().parents[2] / "plugins" / "unica"
            module.smoke([str(self.binary)], plugin_root, 5, time.monotonic() + 5)
        launch = caught.exception
        self.assertEqual(
            self.assert_test_state(launch), launch.cwd.parent.resolve() / "provider-state"
        )
