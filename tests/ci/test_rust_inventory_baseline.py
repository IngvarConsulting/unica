"""The previous Rust inventory comes from a verified first-parent ancestor of this repository."""
from __future__ import annotations
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import rust_inventory_baseline as baseline

REPOSITORY = "owner/unica"
REPOSITORY_ID = 7


def artifact(sha, *, name="ci-rust-inventory-queue", run=11, repository=REPOSITORY_ID, head=REPOSITORY_ID, expired=False):
    return {"name": name, "expired": expired, "workflow_run": {"id": run, "head_sha": sha,
            "repository_id": repository, "head_repository_id": head}}


class Repository:
    """A real git history: main commits, then the change under test."""

    def __init__(self, root: Path):
        self.root = root
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "ci@example.invalid")
        self.git("config", "user.name", "CI")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, message):
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")


class InventoryBaselineTests(unittest.TestCase):
    def test_pull_requests_compare_with_their_target_line_and_publish_nothing(self):
        # main is verified by the queue, release lines by their pushes (profile main).
        self.assertEqual(baseline.candidate_names("pr"), [["ci-rust-inventory-queue", "ci-rust-inventory-main"]])
        self.assertEqual(baseline.candidate_names("queue"), [["ci-rust-inventory-queue"]])
        self.assertEqual(baseline.candidate_names("large"), [["ci-rust-inventory-large"], ["ci-rust-inventory-queue"]])

    def test_nearest_verified_ancestor_of_this_repository_wins(self):
        ancestors = ["head", "parent", "grandparent"]
        found = baseline.nearest([
            artifact("unrelated", run=1), artifact("grandparent", run=2), artifact("parent", run=3, expired=True),
            artifact("parent", run=4, head=99), artifact("parent", run=5, repository=99), artifact("parent", run=6),
            artifact("parent", run=7),
        ], ancestors, REPOSITORY_ID)
        self.assertEqual(found["workflow_run"]["id"], 6)
        self.assertIsNone(baseline.nearest([artifact("unrelated")], ancestors, REPOSITORY_ID))

    def test_removals_since_the_baseline_count_but_a_reset_belongs_to_the_change_itself(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Repository(Path(temporary))
            base = repository.commit("main")
            repository.commit("earlier entry\n\n* drop\n\nRemoves-test-target: unica-old\nRust-inventory-baseline: reset")
            repository.commit("this change\n\n  Removes-test-target: unica-coder::bin/unica  ")
            for profile in ("pr", "queue"):
                found = baseline.explanations(repository.root, base, profile)
                self.assertEqual(found["removedTargets"], ["unica-coder::bin/unica", "unica-old"])
                # A queue batch neighbour's reset does not cover this entry.
                self.assertFalse(found["reset"])
            for profile in ("main", "large", "release"):
                # No batches here: a reset merged since the baseline applies.
                self.assertTrue(baseline.explanations(repository.root, base, profile)["reset"])
            repository.commit("next\n\nRust-inventory-baseline: reset")
            self.assertTrue(baseline.explanations(repository.root, None, "queue")["reset"])
            self.assertEqual(baseline.explanations(repository.root, None, "queue")["removedTargets"], [])
            # A mention inside prose is not a declaration.
            repository.commit("prose\n\nWe could write Removes-test-target: x later.")
            self.assertEqual(baseline.explanations(repository.root, None, "queue")["removedTargets"], [])

    def run_baseline(self, listing, *, inventory=None, path=baseline.WORKFLOW, failure=None, profile="queue"):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Repository(Path(temporary))
            main = repository.commit("main")
            repository.commit("change")
            calls = []

            def gh(*args):
                calls.append(args)
                if failure and args[0] == failure:
                    raise RuntimeError("gh api failed: HTTP 502")
                if args[0] == "api" and "/artifacts?" in args[1]:
                    name = args[1].split("name=", 1)[1].split("&", 1)[0]
                    return json.dumps({"artifacts": [found for found in (item(main) for item in listing) if found["name"] == name]})
                if args[0] == "run":
                    directory = Path(args[args.index("--dir") + 1])
                    document = {"schemaVersion": 1, "sha": main, "profile": "queue", "runId": "11", "configurations": []}
                    (directory / "inventory.json").write_text(json.dumps({**document, **(inventory or {})}))
                    return ""
                if args[0] == "api" and "/actions/runs/" in args[1]:
                    return path + "\n"
                raise AssertionError(args)

            with patch.object(baseline, "gh", side_effect=gh):
                return baseline.baseline(repository.root, REPOSITORY, REPOSITORY_ID, profile), main, calls

    def test_found_inventory_is_downloaded_from_its_own_run(self):
        document, main, calls = self.run_baseline([lambda sha: artifact(sha)])
        self.assertEqual(document["source"], {"artifact": "ci-rust-inventory-queue", "runId": 11, "headSha": main})
        self.assertEqual(document["inventory"]["sha"], main)
        self.assertEqual(document["explanations"]["range"], main + "..HEAD")
        self.assertIn(("run", "download", "11", "--repo", REPOSITORY, "--name", "ci-rust-inventory-queue"), [call[:7] for call in calls])

    def test_pull_request_to_a_release_line_finds_its_push_inventory(self):
        document, main, _ = self.run_baseline([lambda sha: artifact(sha, name="ci-rust-inventory-main")],
                                              inventory={"profile": "main"}, profile="pr")
        self.assertEqual(document["source"]["artifact"], "ci-rust-inventory-main")
        self.assertEqual(document["source"]["headSha"], main)

    def test_empty_search_is_absent_but_mismatches_and_api_failures_stop(self):
        document, _, _ = self.run_baseline([])
        self.assertIsNone(document["source"])
        self.assertEqual(document["explanations"]["range"], "HEAD^1..HEAD")
        for options in ({"inventory": {"sha": "0" * 40}}, {"inventory": {"runId": "12"}},
                        {"inventory": {"profile": "main"}}, {"path": ".github/workflows/other.yml"}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.run_baseline([lambda sha: artifact(sha)], **options)
        with self.assertRaisesRegex(RuntimeError, "502"):
            self.run_baseline([], failure="api")


if __name__ == "__main__":
    unittest.main()
