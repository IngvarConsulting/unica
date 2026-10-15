"""Acceptance corpus storage: parallel additions merge, losses are caught.

The corpus is split so that two changes, each adding its own scenario, touch
disjoint files (#1489).  These tests prove it on a real Git merge and show
that the per-scenario manifest still catches a dropped scenario or step.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.ci.acceptance_controls import corpus_storage

STORAGE = corpus_storage()


def scenario_file(root: Path, scenario: dict) -> Path:
    path = root / STORAGE.SCENARIO_DIR / f"{scenario['id']}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(scenario, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return path


class CorpusStorageTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "acceptance"
        self.root.mkdir()
        shutil.copy2(STORAGE.ROOT / STORAGE.LEGACY_FILE, self.root / STORAGE.LEGACY_FILE)
        shutil.copytree(STORAGE.ROOT / STORAGE.MANIFEST_DIR, self.root / STORAGE.MANIFEST_DIR)
        if (STORAGE.ROOT / STORAGE.SCENARIO_DIR).is_dir():
            shutil.copytree(STORAGE.ROOT / STORAGE.SCENARIO_DIR, self.root / STORAGE.SCENARIO_DIR)

    def problems(self) -> list[str]:
        return STORAGE.manifest_problems(STORAGE.load(self.root), STORAGE.read_manifest(self.root))

    def new_scenario(self, identifier: str, task: str) -> dict:
        template = STORAGE.load(self.root)["scenarios"][0]
        return {**json.loads(json.dumps(template)), "id": identifier, "task": task}

    def legacy(self) -> dict:
        return json.loads((self.root / STORAGE.LEGACY_FILE).read_text(encoding="utf-8"))

    def write_legacy(self, corpus: dict) -> None:
        (self.root / STORAGE.LEGACY_FILE).write_text(
            json.dumps(corpus, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
        )


class ManifestCatchesLossTests(CorpusStorageTestCase):
    def test_the_copied_corpus_matches_its_manifest(self) -> None:
        self.assertEqual(self.problems(), [])

    def test_dropped_historical_scenario_is_caught(self) -> None:
        corpus = self.legacy()
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["id"] != "S047"]
        self.write_legacy(corpus)
        problems = self.problems()
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].startswith("S047: the manifest freezes a scenario"), problems)

    def test_dropped_step_is_caught(self) -> None:
        corpus = self.legacy()
        scenario = next(s for s in corpus["scenarios"] if s["id"] == "S001")
        self.assertGreater(len(scenario["wire"]), 1)
        scenario["wire"].pop()
        self.write_legacy(corpus)
        problems = self.problems()
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].startswith("S001: the manifest freezes 2 steps"), problems)

    def test_dropped_per_file_scenario_is_caught(self) -> None:
        scenario = self.new_scenario("S-9001-dropped", "Сценарий, который потом пропадёт")
        path = scenario_file(self.root, scenario)
        STORAGE.freeze([scenario["id"]], self.root)
        self.assertEqual(self.problems(), [])
        path.unlink()
        self.assertEqual(len(self.problems()), 1)

    def test_scenario_without_manifest_is_caught(self) -> None:
        scenario_file(self.root, self.new_scenario("S-9001-unfrozen", "Сценарий без манифеста"))
        problems = self.problems()
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("--freeze S-9001-unfrozen", problems[0])

    def test_deliberate_removal_edits_the_manifest(self) -> None:
        corpus = self.legacy()
        corpus["scenarios"] = [s for s in corpus["scenarios"] if s["id"] != "S047"]
        self.write_legacy(corpus)
        (self.root / STORAGE.MANIFEST_DIR / "S047.steps").unlink()
        self.assertEqual(self.problems(), [])


class StableIdentifierTests(CorpusStorageTestCase):
    def test_the_historical_block_is_closed(self) -> None:
        corpus = self.legacy()
        corpus["scenarios"].append({**corpus["scenarios"][0], "id": f"S{STORAGE.LEGACY_LAST + 1:03d}"})
        self.write_legacy(corpus)
        with self.assertRaisesRegex(STORAGE.CorpusError, "outside the closed block"):
            STORAGE.load(self.root)

    def test_new_identifier_and_file_name_are_checked(self) -> None:
        for identifier in ["S441", "S-1489", "S-0-name", "S-12-name", "S-440-name", "S-1489-Name", "S-1489-name_x", "1489-name"]:
            with self.subTest(identifier=identifier):
                path = scenario_file(self.root, self.new_scenario(identifier, "Неверный идентификатор"))
                with self.assertRaises(STORAGE.CorpusError):
                    STORAGE.load(self.root)
                path.unlink()
        path = self.root / STORAGE.SCENARIO_DIR / "S-1489-other.json"
        path.write_text(json.dumps(self.new_scenario("S-1489-name", "Чужое имя файла")), encoding="utf-8")
        with self.assertRaisesRegex(STORAGE.CorpusError, "whose id is"):
            STORAGE.load(self.root)

    def test_hidden_files_are_not_part_of_the_corpus(self) -> None:
        for directory in (STORAGE.SCENARIO_DIR, STORAGE.MANIFEST_DIR):
            (self.root / directory).mkdir(exist_ok=True)
            (self.root / directory / ".DS_Store").write_bytes(b"\0")
        self.assertEqual(self.problems(), [])

    def test_order_is_the_historical_block_then_issue_number_and_slug(self) -> None:
        for identifier in ["S-1500-b", "S-999-z", "S-1500-a"]:
            scenario_file(self.root, self.new_scenario(identifier, identifier))
        identifiers = [scenario["id"] for scenario in STORAGE.load(self.root)["scenarios"]]
        self.assertEqual(identifiers[STORAGE.LEGACY_LAST - 1], f"S{STORAGE.LEGACY_LAST:03d}")
        added = [identifier for identifier in identifiers if identifier in {"S-1500-b", "S-999-z", "S-1500-a"}]
        self.assertEqual(added, ["S-999-z", "S-1500-a", "S-1500-b"])
        self.assertLess(identifiers.index(f"S{STORAGE.LEGACY_LAST:03d}"), identifiers.index("S-999-z"))


class ParallelAdditionMergeTests(CorpusStorageTestCase):
    """Two branches from one base each add a scenario; Git merges both."""

    def git(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", "-c", "user.name=corpus", "-c", "user.email=corpus@example.invalid",
             "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main", *args],
            cwd=self.root, check=check, capture_output=True, text=True,
        )

    def commit_all(self, message: str) -> None:
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def add_on_branch(self, branch: str, identifier: str) -> None:
        self.git("switch", "-q", "-c", branch, "main")
        scenario_file(self.root, self.new_scenario(identifier, f"Задача ветки {branch}"))
        STORAGE.freeze([identifier], self.root)
        self.assertEqual(self.problems(), [])
        self.commit_all(f"add {identifier}")

    def test_two_branches_adding_scenarios_merge_without_conflict(self) -> None:
        self.git("init", "-q")
        self.commit_all("base")
        # Adjacent issue numbers: under sequential numbering both would take
        # the same next number and append at the same place.
        self.add_on_branch("first", "S-901485-first-task")
        self.add_on_branch("second", "S-901486-second-task")
        self.git("switch", "-q", "main")
        self.git("merge", "-q", "--no-edit", "first")
        merged = self.git("merge", "--no-edit", "second", check=False)
        self.assertEqual(merged.returncode, 0, merged.stdout + merged.stderr)
        self.assertEqual(self.problems(), [])
        identifiers = [scenario["id"] for scenario in STORAGE.load(self.root)["scenarios"]]
        self.assertLess(identifiers.index("S-901485-first-task"), identifiers.index("S-901486-second-task"))

    def test_appending_to_the_shared_file_is_what_used_to_conflict(self) -> None:
        self.git("init", "-q")
        self.commit_all("base")
        for branch, task in [("first", "Первая"), ("second", "Вторая")]:
            self.git("switch", "-q", "-c", branch, "main")
            corpus = self.legacy()
            corpus["scenarios"].append({**corpus["scenarios"][0], "id": "S441", "task": task})
            self.write_legacy(corpus)
            self.commit_all(f"append {task}")
        self.git("switch", "-q", "main")
        self.git("merge", "-q", "--no-edit", "first")
        merged = self.git("merge", "--no-edit", "second", check=False)
        self.assertNotEqual(merged.returncode, 0, "the counterexample must reproduce the old conflict")
        self.git("merge", "--abort")


if __name__ == "__main__":
    unittest.main()
