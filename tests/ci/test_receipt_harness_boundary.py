"""Страж границы бегунка контракта ReceiptLedger: он наблюдает, а не пишет.

Правило `INV.TEST.LEDGER-HARNESS-OBSERVES`: диспетчер действий сценария не
делает durable-переходов квитанции, писатели живут только у перечисленных
помощников-владельцев, а `ReceiptLedgerStore`/`ReceiptLedgerPort` открывают
лишь названные фикстуры. Проверяется и на живом дереве, и на синтетических
исходниках, чтобы страж падал ровно на том, что запрещено.
"""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "ci" / "check-receipt-harness-boundary.py"
HARNESS_ROOT = Path(
    "crates/unica-coder/src/infrastructure/daemon/runtime_v5/receipt_scenario_v5.rs"
)
HARNESS_DIR = Path(
    "crates/unica-coder/src/infrastructure/daemon/runtime_v5/receipt_scenario_v5"
)
SCANNED_FILES = (
    "control.rs",
    "dispatch.rs",
    "scenario_hooks.rs",
    "scenario_probes.rs",
    "wire.rs",
)


def write_harness(root: Path, source: str = "", scanned: dict[str, str] | None = None) -> None:
    """Кладёт корень обвязки и все читаемые файлы: страж требует их все."""
    path = root / HARNESS_ROOT
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")
    directory = root / HARNESS_DIR
    directory.mkdir(parents=True, exist_ok=True)
    for name in SCANNED_FILES:
        (directory / name).write_text((scanned or {}).get(name, ""), encoding="utf-8")


def run_guard(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(SCRIPT_PATH), "--root", str(root)],
        text=True,
        capture_output=True,
        check=False,
    )


class ReceiptHarnessBoundaryTests(unittest.TestCase):
    def test_live_tree_keeps_the_harness_observing(self) -> None:
        """Живое дерево: диспетчер не пишет, писатели только у владельцев."""
        result = run_guard(REPO_ROOT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_owner_helpers_may_write_what_the_inventory_allows(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                "fn seed_receipt_state() {\n"
                "    runtime.receipt_ledger.promise_task_unbound(key, deadline)?;\n"
                "    runtime.receipt_ledger.publish_direct_terminal(key, deadline)?;\n"
                "}\n"
                "fn run_direct_load() {\n"
                "    runtime.submit_direct_batch_for_load(work, deadline)?;\n"
                "}\n",
                {
                    "dispatch.rs": "fn run_supported_receipt_scenario_for_test() {\n"
                    "    let observed = actor.recover(key, deadline)?;\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_write_in_the_action_dispatcher_fails(self) -> None:
        """Диспетчер живёт в своём файле — стража это не должно смущать."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                scanned={
                    "dispatch.rs": "fn run_supported_receipt_scenario_for_test() {\n"
                    "    actor.promise_task_unbound(key, epoch_ms, deadline)?;\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("the action dispatcher writes", result.stdout)

    def test_a_batch_writer_in_the_dispatcher_fails(self) -> None:
        """Пакетные команды актора — тоже записи.

        Это была настоящая дыра: `reserve` и пакетные команды не значились
        писателями, и диспетчер писал мимо описи.
        """
        for writer in (
            "reserve",
            "reserve_batch",
            "bind_reserved_actor_batch",
            "mark_reserved_begun_batch",
            "publish_direct_terminal_batch",
        ):
            with self.subTest(writer=writer), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                write_harness(
                    root,
                    scanned={
                        "dispatch.rs": "fn run_supported_receipt_scenario_for_test() {\n"
                        f"    actor.{writer}(key, deadline)?;\n"
                        "}\n"
                    },
                )
                result = run_guard(root)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("the action dispatcher writes", result.stdout)

    def test_an_indented_method_does_not_inherit_the_previous_owner(self) -> None:
        """Метод в `impl` отвечает за себя, а не за соседа сверху."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                "fn seed_receipt_state() {\n"
                "    actor.promise_task_unbound(key, deadline)?;\n"
                "}\n"
                "impl Runtime {\n"
                "    fn quietly_advances(&self) {\n"
                "        self.receipt_ledger.promise_task_unbound(key, deadline)?;\n"
                "    }\n"
                "}\n",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("`quietly_advances` writes", result.stdout)

    def test_a_named_probe_method_may_write_what_it_owns(self) -> None:
        """Воротная проба на живом рантайме — владелец из описи."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                scanned={
                    "scenario_probes.rs": "impl V5ReceiptRuntime {\n"
                    "    fn bind_task_under_gate_for_test(&self) {\n"
                    "        self.receipt_ledger.begin_bound_task_handoff(key, deadline)?;\n"
                    "    }\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_sibling_module_write_is_not_missed(self) -> None:
        """Записи в соседнем модуле проходят ту же опись, что и корень."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                scanned={
                    "scenario_probes.rs": "impl V5ReceiptRuntime {\n"
                    "    fn unnamed_probe(&self) {\n"
                    "        self.receipt_ledger.reserve_batch(work, deadline)?;\n"
                    "    }\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("`unnamed_probe` writes", result.stdout)

    def test_declaring_a_writer_is_not_calling_it(self) -> None:
        """Определение обёртки-писателя — не её вызов."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                scanned={
                    "scenario_hooks.rs": "pub(super) fn publish_direct_terminal_for_scenario(\n"
                    "    actor: &ReceiptLedgerActor,\n"
                    ") -> Result<(), String> {\n"
                    "    Ok(())\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_writer_outside_the_inventory_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                "fn advance_the_receipt_myself() {\n"
                "    actor.begin_bound_task_handoff(key, epoch_ms, deadline)?;\n"
                "}\n",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("the owner inventory does not allow", result.stdout)

    def test_an_owner_writing_a_transition_it_does_not_own_fails(self) -> None:
        """Опись — пара «владелец → переход», а не пропуск на любые записи."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                "fn seed_direct_probe_terminal() {\n"
                "    actor.promise_task_unbound(key, epoch_ms, deadline)?;\n"
                "}\n",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("the owner inventory does not allow", result.stdout)

    def test_only_named_fixtures_may_open_the_store(self) -> None:
        for forbidden in ("ReceiptLedgerStore", "ReceiptLedgerPort"):
            with self.subTest(kind=forbidden), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                write_harness(
                    root,
                    "fn open_owner() {\n"
                    f"    let store = {forbidden}::open_retained_directory(receipts)?;\n"
                    "}\n",
                )
                result = run_guard(root)
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn("bypasses the actor", result.stdout)

    def test_a_named_store_opener_may_open_it(self) -> None:
        """Актора кто-то должен поднять — эта фикстура и есть дверь."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(
                root,
                scanned={
                    "scenario_hooks.rs": "use crate::infrastructure::receipt_ledger::{\n"
                    "    ReceiptLedgerStore,\n"
                    "};\n"
                    "pub(super) fn open_receipt_actor_for_scenario() {\n"
                    "    let store = ReceiptLedgerStore::open_retained_directory(receipts)?;\n"
                    "    Ok(ReceiptLedgerActor::spawn(store))\n"
                    "}\n"
                },
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_exempt_unit_tests_are_not_the_harness(self) -> None:
        """Юнит-тесты хранилища освобождены: им актор не предписан."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(root)
            (root / HARNESS_DIR / "tests.rs").write_text(
                "fn store_unit_test() {\n"
                "    let store = ReceiptLedgerStore::open(directory)?;\n"
                "    store.promise_task_unbound(key, epoch_ms, deadline)?;\n"
                "}\n",
                encoding="utf-8",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_an_unclassified_harness_file_fails(self) -> None:
        """Новый файл обвязки обязан быть назван — читаемым или освобождённым."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(root)
            (root / HARNESS_DIR / "seeds.rs").write_text("fn seed() {}\n", encoding="utf-8")
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("unclassified harness file", result.stdout)

    def test_a_nested_harness_module_is_not_missed(self) -> None:
        """`mod rogue;` грузит `rogue/mod.rs` — вложенный каталог тоже назван."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_harness(root)
            nested = root / HARNESS_DIR / "rogue"
            nested.mkdir(parents=True, exist_ok=True)
            (nested / "mod.rs").write_text(
                "fn quietly_advances() {\n"
                "    actor.promise_task_unbound(key, deadline)?;\n"
                "}\n",
                encoding="utf-8",
            )
            result = run_guard(root)
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("unclassified harness file", result.stdout)
            self.assertIn("rogue/mod.rs", result.stdout)

    def test_a_missing_harness_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = run_guard(Path(directory))
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("harness source is missing", result.stdout)


if __name__ == "__main__":
    unittest.main()
