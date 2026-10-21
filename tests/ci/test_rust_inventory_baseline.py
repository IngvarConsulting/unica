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

    def explain(self, profile, *, own=None, entry="this entry (#10)"):
        bodies = {
            5: "Earlier entry.\n\n- Removes-test-target: `unica-old`\nRust-inventory-baseline: reset\n",
            10: "This change.\n\n  Removes-test: unica-coder::cli::*  \n`Rust-inventory-baseline: reset`\n",
            11: "Prose only: we could write Removes-test-target: x later.\n",
        }

        def gh(*args):
            number = int(args[1].rsplit("/", 1)[1])
            if number == 404:
                raise baseline.NotFound("gh api failed: HTTP 404")
            return bodies.get(number, "")

        with tempfile.TemporaryDirectory() as temporary, patch.object(baseline, "gh", side_effect=gh):
            repository = Repository(Path(temporary))
            base = repository.commit("main")
            repository.commit("fix: issue reference only (#404)")
            repository.commit("prose (#11)")
            repository.commit("earlier entry (#7) (#5)")
            repository.commit(entry)
            return baseline.explanations(repository.root, REPOSITORY, base, profile, own)

    def test_descriptions_since_the_baseline_explain_removals(self):
        found = self.explain("queue")
        self.assertEqual(found["pullRequests"], [5, 10, 11, 404])
        self.assertEqual(found["removedTargets"], ["unica-old"])
        self.assertEqual(found["removedTests"], ["unica-coder::cli::*"])
        self.assertEqual(found["declaredBy"]["Removes-test-target: unica-old"], [5])

    def test_a_reset_belongs_to_the_change_itself_in_pull_requests_and_queue(self):
        # The queue entry's own pull request may reset; a batch neighbour may not.
        self.assertTrue(self.explain("queue")["reset"])
        self.assertEqual(self.explain("queue")["declaredBy"]["Rust-inventory-baseline: reset"], [10])
        self.assertFalse(self.explain("queue", entry="other entry (#11)")["reset"])
        # A pull request is its own change; its commits carry no number.
        self.assertTrue(self.explain("pr", own=10, entry="wip")["reset"])
        self.assertFalse(self.explain("pr", own=11, entry="wip")["reset"])
        self.assertFalse(self.explain("pr", own=None, entry="wip")["reset"])
        # No batches elsewhere: a reset merged since the baseline applies.
        for profile in ("main", "large", "release"):
            self.assertTrue(self.explain(profile, entry="other entry (#11)")["reset"])

    def test_crlf_descriptions_declare_like_any_other(self):
        with patch.object(baseline, "gh", return_value="Intro\r\nRemoves-test-target: unica-old\r\nRust-inventory-baseline: reset\r\n"):
            body = baseline.description(REPOSITORY, 5)
        self.assertEqual(baseline.REMOVED.findall(body), ["unica-old"])
        self.assertIsNotNone(baseline.RESET.search(body))

    def test_pull_request_branch_commits_do_not_name_other_pull_requests(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Repository(Path(temporary))
            base = repository.commit("main")
            repository.git("switch", "-q", "-c", "topic")
            repository.commit("cherry-picked fix (#77)")
            repository.git("switch", "-q", "main")
            repository.commit("other main work (#78)")
            repository.git("merge", "-q", "--no-ff", "-m", "Merge abc into def", "topic")
            self.assertEqual(baseline.pull_requests(repository.root, f"{base}..HEAD"), {78})

    def test_summary_names_each_declaration_and_a_reset_prominently(self):
        document = {"source": {"runId": 11, "headSha": "a" * 40},
                    "explanations": {"declaredBy": {"Removes-test: x::y": [10], "Rust-inventory-baseline: reset": [12]}}}
        text = baseline.summary_text(document)
        self.assertIn("**Baseline reset declared by #12", text)
        self.assertIn("- `Removes-test: x::y` — #10", text)
        self.assertIn("**none found**", baseline.summary_text({"source": None, "explanations": {"declaredBy": {}}}))

    def test_commit_messages_no_longer_explain_anything(self):
        found = self.explain("queue", entry="this entry (#12)\n\nRemoves-test-target: hidden\nRust-inventory-baseline: reset")
        self.assertNotIn("hidden", found["removedTargets"])
        self.assertFalse(found["reset"])

    def test_description_api_failures_other_than_404_stop(self):
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(baseline, "gh", side_effect=RuntimeError("gh api failed: HTTP 502")):
            repository = Repository(Path(temporary))
            base = repository.commit("main")
            repository.commit("entry (#10)")
            with self.assertRaisesRegex(RuntimeError, "502"):
                baseline.explanations(repository.root, REPOSITORY, base, "queue", None)

    def test_own_pull_request_comes_from_the_event_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            event = Path(temporary) / "event.json"
            event.write_text(json.dumps({"pull_request": {"number": 1883}}))
            with patch.dict(baseline.os.environ, {"GITHUB_EVENT_NAME": "pull_request", "GITHUB_EVENT_PATH": str(event)}):
                self.assertEqual(baseline.own_pull_request(), 1883)
            with patch.dict(baseline.os.environ, {"GITHUB_EVENT_NAME": "merge_group", "GITHUB_EVENT_PATH": str(event)}):
                self.assertIsNone(baseline.own_pull_request())

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
                if args[0] == "api" and "/pulls/" in args[1]:
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
