"""Shared MCP initialization messages for CI harnesses."""

from __future__ import annotations

from typing import Any, Iterable


MCP_HANDSHAKE_ID = "unica-ci-handshake"
MCP_INITIALIZE_PARAMS = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "unica-ci", "version": "1"},
}
RELEASE_ASSESSMENT_HANDSHAKE_ID = "unica-assessment-handshake"
RELEASE_ASSESSMENT_INITIALIZE_PARAMS = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "unica-release-assessment", "version": "1"},
}


def prepare_messages(
    messages: Iterable[dict[str, Any]],
    *,
    handshake_id: str = MCP_HANDSHAKE_ID,
    initialize_params: dict[str, Any] = MCP_INITIALIZE_PARAMS,
) -> list[dict[str, Any]]:
    """Prepend the standard handshake unless the caller starts with initialize."""
    prepared = list(messages)
    if prepared and prepared[0].get("method") == "initialize":
        return prepared
    return [
        {
            "jsonrpc": "2.0",
            "id": handshake_id,
            "method": "initialize",
            "params": initialize_params,
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        *prepared,
    ]


def without_handshake_response(
    responses: Iterable[dict[str, Any]], *, handshake_id: str = MCP_HANDSHAKE_ID
) -> list[dict[str, Any]]:
    """Hide the helper's initialize response from scenario-level assertions."""
    return [response for response in responses if response.get("id") != handshake_id]
