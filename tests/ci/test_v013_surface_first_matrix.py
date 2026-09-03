"""Guards the reviewed v0.12.3 -> v0.13 transition matrix."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE = REPO_ROOT / "tests/fixtures/migration/v0.12.3-baseline.json"
MATRIX = REPO_ROOT / "docs/design/2026-08-31-v0-13-surface-first-cutover-design.md"
LEDGER = REPO_ROOT / "arch/tool-surface.md"
REVIEW = REPO_ROOT / "arch/tool-surface-review.json"
ROW = re.compile(r"^\| `([^`]+)` \|(?P<body>.+)$", re.MULTILINE)
CANONICAL = {
    "unica.view",
    "unica.apply",
    "unica.find",
    "unica.search",
    "unica.check",
    "unica.diff",
    "unica.run",
    "unica.docs",
}
COMPATIBILITY = {
    "unica.task.get",
    "unica.task.result",
    "unica.task.cancel",
}


class SurfaceFirstTransitionMatrixTests(unittest.TestCase):
    def test_every_published_v0123_name_has_exactly_one_transition_row(self) -> None:
        baseline = json.loads(BASELINE.read_text(encoding="utf-8"))["wire"][
            "toolNames"
        ]
        rows = ROW.findall(MATRIX.read_text(encoding="utf-8"))
        names = [name for name, _body in rows]

        self.assertEqual(len(baseline), 74)
        self.assertEqual(len(names), 74)
        self.assertEqual(len(set(names)), 74)
        self.assertEqual(set(names), set(baseline))

    def test_matrix_names_all_eight_targets_and_explicitly_excludes_skills(self) -> None:
        text = MATRIX.read_text(encoding="utf-8")
        for tool in sorted(CANONICAL):
            self.assertIn(f"`{tool}`", text)
        self.assertIn("`plugins/unica/skills/**`\nis excluded from scope", text)
        self.assertIn("**Merging to `main` is not a release**", text)

    def test_target_examples_do_not_encode_pseudo_choices_inside_json_values(self) -> None:
        text = MATRIX.read_text(encoding="utf-8")
        offenders = []
        for line_number, line in enumerate(text.splitlines(), 1):
            for code_span in re.findall(r"`([^`]+)`", line):
                if (
                    code_span.startswith("unica.") or code_span.startswith("run(")
                ) and "|" in code_span:
                    offenders.append((line_number, code_span))
        self.assertEqual(offenders, [])

    def test_generated_ledger_and_review_are_the_exact_compatibility_profile(self) -> None:
        ledger = LEDGER.read_text(encoding="utf-8")
        names = set(re.findall(r"^### `([^`]+)`$", ledger, re.MULTILINE))
        review = set(json.loads(REVIEW.read_text(encoding="utf-8")))

        self.assertEqual(names, CANONICAL | COMPATIBILITY)
        self.assertEqual(review, names)
        self.assertIn("- Инструментов: **11**", ledger)

    def test_shipped_agent_guidance_does_not_route_to_retired_project_tools(self) -> None:
        roots = [
            REPO_ROOT / "plugins/unica/skills",
            REPO_ROOT / "plugins/unica/references",
        ]
        offenders = []
        for root in roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                for retired in (
                    "unica.project.map",
                    "unica.project.status",
                    "source-set-from-project-map",
                ):
                    if retired in text:
                        offenders.append((str(path.relative_to(REPO_ROOT)), retired))
        self.assertEqual(offenders, [])


if __name__ == "__main__":
    unittest.main()
