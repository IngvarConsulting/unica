"""Check publication structure, not whether independent reviews took place."""

from pathlib import Path
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]


class AgentReviewPolicyTests(unittest.TestCase):
    def test_nested_heading_does_not_publish_policy(self) -> None:
        text = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
        nested = text.replace(
            "\n## Классификация задач и независимая проверка\n",
            "\n### Классификация задач и независимая проверка\n",
        )
        self.assertNotEqual(text, nested)
        with patch.object(Path, "read_text", return_value=nested):
            with self.assertRaises(AssertionError):
                self.test_review_policy_is_published_in_agent_entrypoint()

    def test_review_policy_is_published_in_agent_entrypoint(self) -> None:
        text = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
        heading = "## Классификация задач и независимая проверка"
        lines = text.splitlines()
        self.assertEqual(lines.count(heading), 1)
        start = lines.index(heading) + 1
        end = next(
            (index for index in range(start, len(lines)) if lines[index].startswith("## ")),
            len(lines),
        )
        section = "\n".join(lines[start:end])
        self.assertIn("DEC.2026-09-17.AGENT-REVIEW-POLICY", section)
        for task_class in (
            "Косметика",
            "Локальное поведение",
            "Правила и требования",
            "Архитектура и публичные контракты",
        ):
            with self.subTest(task_class=task_class):
                self.assertIn(f"| {task_class} |", section)
        for finding_type in ("Дефект", "Риск", "Вопрос", "Предпочтение"):
            with self.subTest(finding_type=finding_type):
                self.assertIn(f"| {finding_type} |", section)
        for topic in (
            "Скептик проверяет замысел.",
            "Reviewer проверяет результат.",
            "Замечания имеют тип и отдельную важность.",
            "Замечание должно получить проверяемый исход.",
            "Защита от возвращения причины.",
            "Организация проверок.",
        ):
            with self.subTest(topic=topic):
                self.assertIn(f"**{topic}**", section)


if __name__ == "__main__":
    unittest.main()
