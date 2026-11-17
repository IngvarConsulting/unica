"""Provider daemon environment for a state directory owned by a test."""

from pathlib import Path
from typing import Mapping


def provider_fixture_environment(
    environment: Mapping[str, str], state_root: Path
) -> dict[str, str]:
    return {
        **environment,
        "UNICA_PROVIDER_STATE_DIR": str(state_root.resolve()),
        "UNICA_DAEMON_IDLE_GRACE_MS": "5000",
    }
