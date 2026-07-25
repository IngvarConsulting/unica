from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "release-warden.py"


def load_module():
    spec = importlib.util.spec_from_file_location("release_warden", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    # Dataclasses resolve their deferred annotations through sys.modules, so the
    # module has to be registered before it is executed.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ReleaseWardenTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def pull(self, branch: str, *, checks, files=(".agents/plugins/marketplace.json",)):
        return self.module.PullRequest(number=7, branch=branch, files=tuple(files), checks=checks)

    def green(self):
        return (("consumer-fresh-install", "COMPLETED", "SUCCESS"),)

    def test_nothing_to_do_when_the_catalog_already_serves_the_release(self) -> None:
        action = self.module.decide(
            self.module.ReleaseState(latest_release="v0.9.1", catalog_ref="v0.9.1")
        )

        self.assertEqual(action.kind, "noop")

    def test_a_green_staging_pull_request_is_merged_and_carries_promotion_inputs(self) -> None:
        stage = self.pull("codex/stage-v0.9.2-30117190305", checks=self.green())

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", stage_pr=stage)
        )

        self.assertEqual(action.kind, "merge-stage")
        self.assertEqual(action.pull_request, 7)
        # Both promotion inputs are derived from the branch name, so no value is
        # transcribed by hand.
        self.assertEqual(action.context["release_tag"], "v0.9.2")
        self.assertEqual(action.context["source_run_id"], "30117190305")

    def test_staging_is_not_merged_while_a_check_is_pending(self) -> None:
        stage = self.pull(
            "codex/stage-v0.9.2-1",
            checks=(("staged-package-smoke", "IN_PROGRESS", None),),
        )

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", stage_pr=stage)
        )

        self.assertEqual(action.kind, "wait")

    def test_staging_is_not_merged_when_a_check_failed(self) -> None:
        stage = self.pull(
            "codex/stage-v0.9.2-1",
            checks=(("contract", "COMPLETED", "FAILURE"),),
        )

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", stage_pr=stage)
        )

        self.assertEqual(action.kind, "wait")
        self.assertIn("contract: failure", action.detail)

    def test_promotion_waits_until_the_tag_makes_the_install_checks_pass(self) -> None:
        promote = self.pull(
            "codex/promote-v0.9.2-1",
            checks=(("consumer-fresh-install", "COMPLETED", "FAILURE"),),
        )

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", promote_pr=promote)
        )

        # Red here means the catalog ref does not resolve yet, which is exactly
        # the state before the human publishes the tag.
        self.assertEqual(action.kind, "wait")
        self.assertIn("v0.9.2", action.detail)

    def test_a_green_promotion_publishes_the_release(self) -> None:
        promote = self.pull("codex/promote-v0.9.2-1", checks=self.green())

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", promote_pr=promote)
        )

        self.assertEqual(action.kind, "merge-promote")
        self.assertEqual(action.pull_request, 7)

    def test_a_promotion_touching_more_than_the_catalog_is_never_merged(self) -> None:
        promote = self.pull(
            "codex/promote-v0.9.2-1",
            checks=self.green(),
            files=(".agents/plugins/marketplace.json", "plugins/unica/.mcp.json"),
        )

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", promote_pr=promote)
        )

        self.assertEqual(action.kind, "alert")
        self.assertIn("plugins/unica/.mcp.json", action.detail)

    def test_both_host_catalogs_are_an_acceptable_promotion_diff(self) -> None:
        promote = self.pull(
            "codex/promote-v0.9.2-1",
            checks=self.green(),
            files=(".agents/plugins/marketplace.json", ".claude-plugin/marketplace.json"),
        )

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", promote_pr=promote)
        )

        self.assertEqual(action.kind, "merge-promote")

    def test_a_release_with_no_open_pull_request_is_reported_as_stalled(self) -> None:
        action = self.module.decide(self.module.ReleaseState("v0.9.2", "v0.9.1"))

        # This is the failure that went unnoticed for a day: the release exists,
        # nobody is serving it, and nothing was open to merge.
        self.assertEqual(action.kind, "alert")
        self.assertIn("stalled", action.detail)

    def test_a_pull_request_without_any_check_is_never_treated_as_green(self) -> None:
        stage = self.pull("codex/stage-v0.9.2-1", checks=())

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", stage_pr=stage)
        )

        self.assertEqual(action.kind, "wait")

    def test_staging_is_advanced_before_promotion_when_both_are_open(self) -> None:
        stage = self.pull("codex/stage-v0.9.2-1", checks=self.green())
        promote = self.pull("codex/promote-v0.9.2-1", checks=self.green())

        action = self.module.decide(
            self.module.ReleaseState("v0.9.2", "v0.9.1", stage_pr=stage, promote_pr=promote)
        )

        # Promotion must never overtake the payload it points at.
        self.assertEqual(action.kind, "merge-stage")


if __name__ == "__main__":
    unittest.main()
