"""Сайт ждёт, пока очередь вольёт проверенное дерево, и не ждёт зря."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "await-queue-merge.py"


def load_module():
    spec = importlib.util.spec_from_file_location("await_queue_merge", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Queue:
    """Очередь, какой её видит скрипт: ответы сравнения с main по порядку и ветка очереди."""

    def __init__(self, statuses: list[str | None], branch_checks_before_gone: int | None = None) -> None:
        self.statuses = statuses
        self.compared = 0
        self.branch_checks = 0
        self.branch_checks_before_gone = branch_checks_before_gone
        self.now = 0.0
        self.slept: list[float] = []

    def status(self) -> str | None:
        answer = self.statuses[min(self.compared, len(self.statuses) - 1)]
        self.compared += 1
        return answer

    def still_queued(self) -> bool:
        self.branch_checks += 1
        return self.branch_checks_before_gone is None or self.branch_checks <= self.branch_checks_before_gone

    def sleep(self, seconds: float) -> None:
        self.slept.append(seconds)
        self.now += seconds

    def clock(self) -> float:
        return self.now


class AwaitQueueMergeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def wait(self, queue: Queue, timeout: float = 60, interval: float = 10) -> str:
        return self.module.wait(queue.status, queue.still_queued, queue.sleep, queue.clock, timeout, interval)

    def test_a_commit_already_in_main_needs_no_wait(self) -> None:
        for status in ("identical", "ahead"):
            with self.subTest(status):
                queue = Queue([status])
                self.assertEqual(self.wait(queue), self.module.MERGED)
                self.assertEqual(queue.slept, [])

    def test_waits_while_the_queue_holds_the_commit_and_stops_once_it_lands(self) -> None:
        queue = Queue(["behind", "behind", "identical"])

        self.assertEqual(self.wait(queue), self.module.MERGED)
        self.assertEqual(queue.slept, [10, 10])

    def test_a_commit_the_queue_let_go_is_not_waited_for_until_the_deadline(self) -> None:
        """Снятый с очереди pull request: ветки очереди нет и в main коммита нет — ждать нечего."""
        queue = Queue(["behind"], branch_checks_before_gone=1)

        self.assertEqual(self.wait(queue), self.module.REMOVED)
        self.assertEqual(queue.slept, [10])

    def test_a_merge_between_two_requests_is_not_taken_for_removal(self) -> None:
        """Ветка очереди пропадает и после вливания, поэтому перед выводом сравнение повторяется."""
        queue = Queue(["behind", "identical"], branch_checks_before_gone=0)

        self.assertEqual(self.wait(queue), self.module.MERGED)

    def test_a_main_that_went_elsewhere_ends_the_wait_at_once(self) -> None:
        """Так выглядел коммит #1031: успешный прогон, а в main он не лёг."""
        queue = Queue(["diverged"])

        self.assertEqual(self.wait(queue), self.module.DIVERGED)
        self.assertEqual(queue.slept, [])

    def test_a_silent_api_waits_no_longer_than_the_deadline(self) -> None:
        queue = Queue([None])

        self.assertEqual(self.wait(queue, timeout=35, interval=10), self.module.TIMED_OUT)
        self.assertGreaterEqual(queue.now, 35)
        self.assertLess(queue.now, 35 + 10)

    def test_the_queue_holds_the_commit_only_while_its_branch_points_at_it(self) -> None:
        sha = "a5484434bb2b3a1f9a6d5f06a0fb6bd4eb0cdd7e"
        cases = {
            "ветка на проверенном коммите": ((0, sha + "\n", ""), True),
            "очередь пересобрала элемент под тем же именем": ((0, "39982c2f" + "0" * 32, ""), False),
            "ветки нет": ((1, "", "gh: Not Found (HTTP 404)"), False),
            "сбой API ждёт до срока": ((1, "", "gh: Server Error (HTTP 502)"), True),
        }
        for name, (answer, expected) in cases.items():
            with self.subTest(name):
                self.assertIs(self.module.holds(*answer, sha), expected)


if __name__ == "__main__":
    unittest.main()
