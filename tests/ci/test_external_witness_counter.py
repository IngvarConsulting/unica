"""Сводка положительных свидетелей внешних акторов не держит глобальный счётчик.

Каждый прогон считает своих свидетелей сам: общий на процесс счётчик
`EXTERNAL_ACTOR_WITNESS_RUNS` и его сброс делали итог зависимым от соседних
тестов того же процесса.
"""

from __future__ import annotations

import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE = "crates/unica-source-designer/src/project_sources.rs"
# Склеено, чтобы сам страж не содержал запрещённых имён.
FORBIDDEN = (
    "EXTERNAL_ACTOR_" + "WITNESS_RUNS",
    "reset_external_actor_" + "witness_runs",
    "external_actor_" + "witness_runs",
)


def violations(source: str) -> list[str]:
    return [f"external-positive aggregate still depends on process-global state: {name}"
            for name in FORBIDDEN if name in source]


class ExternalWitnessCounterTests(unittest.TestCase):
    def test_external_actor_positive_witness_uses_no_process_global_counter(self) -> None:
        self.assertEqual(violations((REPO_ROOT / SOURCE).read_text(encoding="utf-8")), [])


if __name__ == "__main__":
    unittest.main()
