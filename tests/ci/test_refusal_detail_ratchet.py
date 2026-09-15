"""Храповик уточнений отказа: неуточнённых `provider_unavailable` может стать
только меньше.

`provider_unavailable` без `detailCode` отвечает умолчанием `needsHuman` — самым
дорогим исходом для агента. Словарь уточнений `RefusalDetail` (`source_unreadable`,
`provider_absent`, `backend_busy`, …) сужает код и выбирает исход точнее. Этот
страж считает места production-кода, которые по-прежнему поднимают голый
`RefusalCode::ProviderUnavailable`, и держит потолок: правка, снизившая число,
обязана опустить потолок вместе с собой, правка, поднявшая его, не проходит.
"""
from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "crates" / "unica-coder" / "src"

# Потолок опускается вместе с каждым срезом D-1 (#871): читатели
# `v13_read.rs`/`v13_read_port.rs` (дефекты дескриптора и нечитаемые файлы
# набора — `source_unreadable`), затем `check`/`apply` в `v13_service.rs`
# (провайдер диагностик не отработал — `provider_absent`).
UNDETAILED_PROVIDER_UNAVAILABLE_CEILING = 155

TEST_TAIL = re.compile(r"#\[cfg\(test\)\]\s*(?:pub(?:\(crate\))?\s+)?mod\s+\w+")
STRING_LITERAL = re.compile(r'"(?:\\.|[^"\\])*"')
LINE_COMMENT = re.compile(r"//[^\n]*")


def production_text(path: Path) -> str:
    """Код файла: без хвостового тестового модуля, строковых литералов и
    комментариев — упоминание кода в прозе или в тексте сообщения местом
    отказа не является."""
    text = path.read_text(encoding="utf-8")
    match = TEST_TAIL.search(text)
    if match:
        text = text[: match.start()]
    text = STRING_LITERAL.sub('""', text)
    return LINE_COMMENT.sub("", text)


def undetailed_sites() -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(SOURCE_ROOT.rglob("*.rs")):
        if path.name == "tests.rs" or "tests" in path.parts:
            continue
        count = production_text(path).count("RefusalCode::ProviderUnavailable")
        if count:
            counts[path.relative_to(REPO_ROOT).as_posix()] = count
    return counts


class RefusalDetailRatchetTests(unittest.TestCase):
    def test_undetailed_provider_unavailable_only_decreases(self) -> None:
        counts = undetailed_sites()
        total = sum(counts.values())
        worst = sorted(counts.items(), key=lambda item: -item[1])[:8]
        self.assertLessEqual(
            total,
            UNDETAILED_PROVIDER_UNAVAILABLE_CEILING,
            "новое место с голым provider_unavailable: назови уточнение из "
            f"RefusalDetail или объясни, почему его нет; сейчас {total}, "
            f"крупнейшие файлы {worst}",
        )
        self.assertEqual(
            total,
            UNDETAILED_PROVIDER_UNAVAILABLE_CEILING,
            "неуточнённых мест стало меньше — опусти "
            f"UNDETAILED_PROVIDER_UNAVAILABLE_CEILING до {total}",
        )

    def test_production_text_keeps_code_and_drops_tests_strings_and_comments(
        self,
    ) -> None:
        sample = (
            "// RefusalCode::ProviderUnavailable in a comment\n"
            'fn a() -> &str { "RefusalCode::ProviderUnavailable in a string" }\n'
            "fn b() { let _ = RefusalCode::ProviderUnavailable; } // trailing\n"
            "#[cfg(test)]\nmod tests {\n    fn c() { RefusalCode::ProviderUnavailable }\n}\n"
        )
        path = REPO_ROOT / "tests" / "ci" / "_ratchet_sample.rs"
        path.write_text(sample, encoding="utf-8")
        try:
            self.assertEqual(
                production_text(path).count("RefusalCode::ProviderUnavailable"), 1
            )
        finally:
            path.unlink()


if __name__ == "__main__":
    unittest.main()
