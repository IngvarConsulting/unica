from __future__ import annotations

import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.ci.mcp_handshake import (
    MCP_HANDSHAKE_ID,
    RELEASE_ASSESSMENT_HANDSHAKE_ID,
    RELEASE_ASSESSMENT_INITIALIZE_PARAMS,
    prepare_messages,
    without_handshake_response,
)


class McpHandshakeContractTests(unittest.TestCase):
    def test_prepare_messages_prepends_one_shared_handshake(self) -> None:
        tool_request = {"jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": {}}

        prepared = prepare_messages([tool_request])

        self.assertEqual(prepared[0]["id"], MCP_HANDSHAKE_ID)
        self.assertEqual(prepared[0]["method"], "initialize")
        self.assertEqual(prepared[1]["method"], "notifications/initialized")
        self.assertEqual(prepared[2], tool_request)

    def test_prepare_messages_preserves_caller_initialize_without_duplication(self) -> None:
        initialize = {
            "jsonrpc": "2.0",
            "id": "caller-handshake",
            "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}},
        }

        prepared = prepare_messages([initialize])

        self.assertEqual(prepared, [initialize])

    def test_without_handshake_response_keeps_tool_responses(self) -> None:
        responses = [
            {"jsonrpc": "2.0", "id": MCP_HANDSHAKE_ID, "result": {}},
            {"jsonrpc": "2.0", "id": 7, "result": {"tools": []}},
        ]

        self.assertEqual(without_handshake_response(responses), [responses[1]])

    def test_prepare_messages_can_preserve_assessment_identity(self) -> None:
        prepared = prepare_messages(
            [],
            handshake_id=RELEASE_ASSESSMENT_HANDSHAKE_ID,
            initialize_params=RELEASE_ASSESSMENT_INITIALIZE_PARAMS,
        )

        self.assertEqual(prepared[0]["id"], RELEASE_ASSESSMENT_HANDSHAKE_ID)
        self.assertEqual(prepared[0]["params"], RELEASE_ASSESSMENT_INITIALIZE_PARAMS)


if __name__ == "__main__":
    unittest.main()
