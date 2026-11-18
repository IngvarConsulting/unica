"""Здоровая среда квитанций отпускает именованную власть последней.

Rust освобождает поля структуры в порядке объявления. В `V5ReceiptRuntime`
актор журнала квитанций объявлен раньше блокировки власти: при обычном
завершении сначала останавливаются актор и хранилище, и только потом
освобождается власть над журналом.
"""

from __future__ import annotations

import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE = "crates/unica-execution/src/receipt_runtime.rs"
DECLARATION = "struct V5ReceiptRuntime<H: InvocationHost> {"
ACTOR_FIELD = "receipt_ledger: ReceiptLedgerActor"
AUTHORITY_FIELD = "_stable_authority: ReceiptAuthorityLock"


def violations(source: str) -> list[str]:
    start = source.find(DECLARATION)
    if start < 0:
        return ["runtime owner declaration is missing"]
    body, closed, _ = source[start:].partition("\n}")
    if not closed:
        return ["runtime owner declaration has no end"]
    actor, authority = body.find(ACTOR_FIELD), body.find(AUTHORITY_FIELD)
    if actor < 0 or authority < 0:
        return [f"runtime owner lacks {ACTOR_FIELD if actor < 0 else AUTHORITY_FIELD}"]
    if actor > authority:
        return ["healthy Rust drop order must join actor/store before authority release"]
    return []


class ReceiptRuntimeDropOrderTests(unittest.TestCase):
    def test_healthy_runtime_drops_the_actor_store_before_releasing_named_authority(self) -> None:
        source = (REPO_ROOT / SOURCE).read_text(encoding="utf-8").replace("\r\n", "\n")
        self.assertEqual(violations(source), [])

    def test_a_swapped_or_missing_field_fails(self) -> None:
        good = f"{DECLARATION}\n    {ACTOR_FIELD},\n    {AUTHORITY_FIELD},\n}}\n"
        swapped = f"{DECLARATION}\n    {AUTHORITY_FIELD},\n    {ACTOR_FIELD},\n}}\n"
        self.assertEqual(violations(good), [])
        self.assertTrue(violations(swapped))
        self.assertTrue(violations(good.replace(ACTOR_FIELD, "ledger: Other")))
        self.assertTrue(violations(good.replace(AUTHORITY_FIELD, "lock: Other")))
        # A field declared after the struct does not count.
        self.assertTrue(violations(f"{DECLARATION}\n    {AUTHORITY_FIELD},\n}}\nfn f() {{ {ACTOR_FIELD} }}\n"))


if __name__ == "__main__":
    unittest.main()
